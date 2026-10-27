use aqua_pattern_core::{
    mcp::{self, PROTOCOL_VERSION},
    song::{example_song, InstrumentId},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "leviathan-mcp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("root");
        fs::create_dir_all(&root).unwrap();
        Self { base, root }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    replies: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    id: u64,
}
impl Client {
    fn new(f: &Fixture) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_leviathan_mcp"))
            .arg("--workspace-root")
            .arg(&f.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (tx, replies) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            replies,
            reader: Some(reader),
            id: 0,
        }
    }
    fn ready(f: &Fixture) -> Self {
        let mut client = Self::new(f);
        let result = client.request("initialize",json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"Leviathan test client","version":"1"}}));
        assert_eq!(result["result"]["protocolVersion"], PROTOCOL_VERSION);
        client.notify("notifications/initialized");
        client
    }
    fn line(&mut self, line: &str) {
        let input = self.input.as_mut().unwrap();
        input.write_all(line.as_bytes()).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }
    fn receive(&self) -> Value {
        let line = self
            .replies
            .recv_timeout(Duration::from_secs(60))
            .expect("MCP must finish within the generous test deadline; synchronous tools have no wall-clock SLA");
        serde_json::from_str(&line).expect("stdout must contain only JSON-RPC")
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.line(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string());
        let response = self.receive();
        assert_eq!(response["id"], id);
        response
    }
    fn notify(&mut self, method: &str) {
        self.line(&json!({"jsonrpc":"2.0","method":method}).to_string());
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.request("tools/call", json!({"name":name,"arguments":arguments}));
        assert!(
            reply.get("error").is_none(),
            "unexpected protocol failure: {reply}"
        );
        let result = &reply["result"];
        let mirrored: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(mirrored, result["structuredContent"]);
        assert_eq!(
            result["isError"].as_bool().unwrap(),
            !mirrored["ok"].as_bool().unwrap()
        );
        mirrored
    }
    fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments);
        assert_eq!(result["ok"], true, "{result}");
        result
    }
    fn document(&mut self) -> Value {
        self.ok("leviathan_project_read", json!({"section":"document"}))
    }
    fn finish(&mut self) {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "server must exit on stdin EOF");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn title(value: &str) -> Value {
    json!({"op":"set_song","title":value})
}
fn edit(client: &mut Client, revision: u64, operations: Vec<Value>) -> Value {
    client.call(
        "leviathan_edit_batch",
        json!({"expected_revision":revision,"operations":operations}),
    )
}

#[test]
fn stdio_initialize_discover_tools_and_error_layers_are_real_protocol_messages() {
    let f = Fixture::new();
    let mut c = Client::new(&f);
    assert_eq!(c.request("tools/list", json!({}))["error"]["code"], -32002);
    let init=c.request("initialize",json!({"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(c.request("tools/list", json!({}))["error"]["code"], -32002);
    c.notify("notifications/initialized");
    c.notify("notifications/unknown");
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
    let catalog = c.request("tools/list", json!({}));
    let tools = catalog["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 64);
    for tool in tools {
        assert!(!tool["title"].as_str().unwrap().is_empty());
        assert!(!tool["description"].as_str().unwrap().is_empty());
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(tool["outputSchema"]["type"], "object");
    }
    let capabilities = c.ok("leviathan_capabilities", json!({}));
    assert_eq!(capabilities["data"]["scope"], "isolated_headless_document");
    assert!(capabilities["data"]["unavailable"]["live_transport"].is_string());
    assert_eq!(
        c.request("tools/call", json!({"name":"not_a_tool","arguments":{}}))["error"]["code"],
        -32602
    );
    assert_eq!(
        c.request(
            "tools/call",
            json!({"name":"leviathan_capabilities","arguments":[]})
        )["error"]["code"],
        -32602
    );
    assert_eq!(
        c.call("leviathan_project_new", json!({}))["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(
        c.request("not_a_method", json!({}))["error"]["code"],
        -32601
    );
    c.finish();
}

#[test]
fn malformed_json_and_unusable_ids_never_make_invalid_response_ids() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    for id in [json!({}), json!([]), json!(true), json!(1.5), Value::Null] {
        c.line(&json!({"jsonrpc":"2.0","id":id,"method":"ping"}).to_string());
        let response = c.receive();
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], -32600);
    }
    c.line("{bad-json");
    assert_eq!(c.receive()["error"]["code"], -32700);
    c.line("[]");
    assert_eq!(c.receive()["error"]["code"], -32600);
    c.line(r#"{"id":"kept","method":"ping"}"#);
    let response = c.receive();
    assert_eq!(response["id"], "kept");
    assert_eq!(response["error"]["code"], -32600);
    c.line(r#"{"jsonrpc":"2.0","id":"string-id","method":"ping"}"#);
    assert_eq!(c.receive()["id"], "string-id");
    c.finish();
}

#[test]
fn end_to_end_create_effects_edit_save_export_undo_redo_and_reopen() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(
        c.ok("leviathan_project_new", json!({"expected_revision":0}))["revision"],
        1
    );
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":1,"path":"sample.wav"}),
    );
    let batch = edit(
        &mut c,
        1,
        vec![
            json!({"op":"set_song","title":"MCP song","bpm":400,"lines_per_beat":256}),
            json!({"op":"upsert_pattern","id":1,"name":"A","rows":4}),
            json!({"op":"upsert_instrument","id":1,"name":"Demo","sample":"sample.wav"}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.2,"effects":{"retrigger":{"repeats":1,"spacing":0.25},"release":0.75,"glide":{"target":67,"semitones_per_second":12}}}}),
            json!({"op":"clone_pattern","source_id":1,"new_id":8,"name":"Independent"}),
            json!({"op":"set_order","pattern_ids":[1,8,1]}),
        ],
    );
    assert_eq!(batch["ok"], true, "{batch}");
    assert_eq!(batch["revision"], 2);
    let before = c.document()["data"]["song"].clone();
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    assert!(!evaluated["data"]["manifest"]["events"]
        .as_array()
        .unwrap()
        .is_empty());
    let exported = c.ok(
        "leviathan_song_export",
        json!({"expected_revision":2,"path":"song.wav"}),
    );
    assert_eq!(exported["revision"], 2);
    assert_eq!(exported["data"]["sample_rate"], 48000);
    let disk_manifest: Value =
        serde_json::from_slice(&fs::read(f.root.join("song.events.json")).unwrap()).unwrap();
    let disk_typed: aqua_pattern_core::song_render::Manifest =
        serde_json::from_value(disk_manifest).unwrap();
    let evaluated_typed: aqua_pattern_core::song_render::Manifest =
        serde_json::from_value(evaluated["data"]["manifest"].clone()).unwrap();
    assert_eq!(disk_typed, evaluated_typed);
    let wav = hound::WavReader::open(f.root.join("song.wav")).unwrap();
    assert_eq!(wav.spec().channels, 2);
    assert_eq!(
        u64::from(wav.duration()),
        exported["data"]["stats"]["frames"].as_u64().unwrap()
    );
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":2,"path":"song.leviathan.json"}),
    );
    let saved: Value =
        serde_json::from_slice(&fs::read(f.root.join("song.leviathan.json")).unwrap()).unwrap();
    assert_eq!(saved["song"]["instruments"]["1"]["sample"], "sample.wav");
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"]["title"], "Untitled");
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before);
    let reopened = c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"song.leviathan.json"}),
    );
    assert_eq!(reopened["revision"], 6);
    assert_eq!(reopened["data"]["can_undo"], false);
    assert_eq!(c.document()["data"]["song"], before);
    c.finish();
}

#[test]
fn batches_are_atomic_revision_checked_single_undo_and_noops_preserve_history() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = c.document();
    let failed = edit(
        &mut c,
        0,
        vec![
            title("Must roll back"),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":128}}}),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 1);
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(&mut c, 90, vec![title("stale")])["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(
        edit(
            &mut c,
            0,
            vec![json!({"op":"unsupported_effect","value":1})]
        )["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(
        edit(&mut c, 0, vec![])["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(edit(&mut c, 0, vec![title("Untitled")])["revision"], 0);
    let changed = edit(&mut c, 0, vec![title("one"), title("two")]);
    assert_eq!(changed["revision"], 1);
    c.ok("leviathan_history_undo", json!({"expected_revision":1}));
    let undone = c.document();
    assert_eq!(undone["data"]["song"], before["data"]["song"]);
    assert_eq!(undone["data"]["session"]["can_undo"], false);
}

#[test]
fn structural_tools_cover_tracks_instruments_patterns_order_and_field_preservation() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let mut track_order = vec![9, 4, 3, 2, 1];
    track_order.extend(5..=8);
    track_order.extend(10..=24);
    let result = edit(
        &mut c,
        0,
        vec![
            json!({"op":"upsert_track","id":9,"name":"Nine","note_columns":3}),
            json!({"op":"set_track_order","track_ids":track_order}),
            json!({"op":"upsert_instrument","id":9,"name":"No sample yet","sample":null}),
            json!({"op":"upsert_pattern","id":9,"name":"Nine","rows":16}),
            json!({"op":"set_cell","pattern_id":9,"track_id":9,"row":3,"column":2,"cell":{"note":{"Note":67},"instrument":9,"velocity":0.4}}),
            json!({"op":"set_effects","pattern_id":9,"track_id":9,"row":3,"column":2,"effects":{"release":0.5}}),
            json!({"op":"append_repeat","pattern_id":9}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let song = c.document()["data"]["song"].clone();
    assert_eq!(
        song["patterns"]["9"]["tracks"]["9"]["3"]["2"]["velocity"],
        json!(0.4f32)
    );
    let removed = edit(
        &mut c,
        1,
        vec![
            json!({"op":"clear_cell","pattern_id":9,"track_id":9,"row":3,"column":2}),
            json!({"op":"clear_pattern_track","pattern_id":9,"track_id":9}),
            json!({"op":"set_order","pattern_ids":[1]}),
            json!({"op":"remove_pattern","id":9}),
            json!({"op":"remove_instrument","id":9}),
            json!({"op":"remove_track","id":9}),
        ],
    );
    assert_eq!(removed["ok"], true, "{removed}");
    let after = c.document();
    assert!(after["data"]["song"]["patterns"].get("9").is_none());
}

#[test]
fn unsaved_destructive_changes_require_explicit_revision_bound_discard() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    edit(&mut c, 0, vec![title("unsaved")]);
    let before = c.document();
    assert_eq!(
        c.call("leviathan_project_new", json!({"expected_revision":1}))["error"]["code"],
        "unsaved_changes"
    );
    assert_eq!(
        c.call(
            "leviathan_project_open",
            json!({"expected_revision":1,"path":"missing.leviathan.json"})
        )["error"]["code"],
        "unsaved_changes"
    );
    assert_eq!(c.document(), before);
    c.ok(
        "leviathan_project_new",
        json!({"expected_revision":1,"discard_unsaved":true}),
    );
    assert_eq!(c.document()["data"]["song"]["title"], "Untitled");
}

#[test]
fn saves_never_overwrite_without_review_and_changed_bytes_invalidate_token() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":0,"path":"song.leviathan.json"}),
    );
    assert_eq!(
        c.call("leviathan_project_save", json!({"expected_revision":1}))["error"]["code"],
        "overwrite_confirmation_required"
    );
    let token = c.ok(
        "leviathan_project_prepare_overwrite",
        json!({"expected_revision":1,"path":"song.leviathan.json"}),
    )["data"]["overwrite_token"]
        .clone();
    let path = f.root.join("song.leviathan.json");
    let mut changed = fs::read(&path).unwrap();
    changed.push(b' ');
    fs::write(&path, &changed).unwrap();
    let failed = c.call(
        "leviathan_project_save",
        json!({"expected_revision":1,"overwrite_token":token}),
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(fs::read(&path).unwrap(), changed);
    let token = c.ok(
        "leviathan_project_prepare_overwrite",
        json!({"expected_revision":1,"path":"song.leviathan.json"}),
    )["data"]["overwrite_token"]
        .clone();
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":1,"overwrite_token":token}),
    );
    let token = c.ok(
        "leviathan_project_prepare_overwrite",
        json!({"expected_revision":2,"path":"song.leviathan.json"}),
    )["data"]["overwrite_token"]
        .clone();
    edit(&mut c, 2, vec![title("new revision")]);
    assert_eq!(
        c.call(
            "leviathan_project_save",
            json!({"expected_revision":3,"overwrite_token":token})
        )["error"]["code"],
        "overwrite_confirmation_required"
    );
}

#[test]
fn root_confinement_rejects_traversal_absolute_inputs_and_embedded_asset_escape() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    for path in [
        "../outside.leviathan.json".to_owned(),
        f.base
            .join("outside.leviathan.json")
            .to_string_lossy()
            .into_owned(),
    ] {
        let result = c.call(
            "leviathan_project_save",
            json!({"expected_revision":0,"path":path}),
        );
        assert_eq!(result["error"]["code"], "path_denied");
    }
    let mut song = example_song();
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample =
        Some(f.base.join("does-not-exist.wav"));
    fs::write(
        f.root.join("escaped.leviathan.json"),
        serde_json::to_vec(&json!({"format":"leviathan-song","version":1,"song":song})).unwrap(),
    )
    .unwrap();
    let failed = c.call(
        "leviathan_project_open",
        json!({"expected_revision":0,"path":"escaped.leviathan.json"}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("outside the startup workspace"));
    assert_eq!(c.document()["revision"], 0);
    assert_eq!(
        edit(
            &mut c,
            0,
            vec![
                json!({"op":"upsert_instrument","id":1,"name":"escape","sample":"../outside.wav"})
            ]
        )["error"]["code"],
        "path_denied"
    );
    assert!(!f.base.join("outside.leviathan.json").exists());
}

#[cfg(unix)]
#[test]
fn symlink_files_parents_and_export_companion_are_rejected_without_outputs() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::write(f.base.join("outside.wav"), b"unchanged").unwrap();
    symlink(f.base.join("outside.wav"), f.root.join("sample.wav")).unwrap();
    symlink(&f.base, f.root.join("escape")).unwrap();
    symlink(
        f.base.join("outside.wav"),
        f.root.join("output.events.json"),
    )
    .unwrap();
    let mut c = Client::ready(&f);
    for (tool, path) in [
        ("leviathan_sample_generate", "sample.wav"),
        ("leviathan_project_save", "escape/project.leviathan.json"),
        ("leviathan_song_export", "output.wav"),
    ] {
        assert_eq!(
            c.call(tool, json!({"expected_revision":0,"path":path}))["error"]["code"],
            "path_denied"
        );
    }
    assert_eq!(
        edit(
            &mut c,
            0,
            vec![json!({"op":"upsert_instrument","id":1,"name":"alias","sample":"sample.wav"})]
        )["error"]["code"],
        "path_denied"
    );
    assert_eq!(fs::read(f.base.join("outside.wav")).unwrap(), b"unchanged");
    assert!(!f.root.join("output.wav").exists());
    let files = c.ok("leviathan_workspace_list", json!({}));
    assert!(files["data"]["files"].as_array().unwrap().is_empty());
}

#[test]
fn oversized_messages_are_drained_and_the_next_request_still_works() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    c.line(&"x".repeat(1024 * 1024 + 2));
    assert_eq!(c.receive()["error"]["code"], -32600);
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
}

#[test]
fn startup_requires_explicit_existing_root_and_emits_no_stdout_banner() {
    let output = Command::new(env!("CARGO_BIN_EXE_leviathan_mcp"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let f = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_leviathan_mcp"))
        .arg("--workspace-root")
        .arg(f.root.join("missing"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn advertised_catalog_has_unique_names_and_all_batch_variants_are_typed() {
    let catalog = mcp::tool_catalog();
    let names: std::collections::BTreeSet<_> = catalog
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), catalog.len());
    for name in ["leviathan_onset_v2_prepare", "leviathan_onset_v2_repick"] {
        let picker = &catalog.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"]
            ["properties"]["picker"]["properties"];
        for field in [
            "enable_phase",
            "threshold",
            "mad_multiplier",
            "min_gap_ms",
            "dense_gap_ms",
            "refinement",
        ] {
            assert!(
                picker.get(field).is_some(),
                "Missing UI-shared control {field}"
            );
        }
    }
    let batch = catalog
        .iter()
        .find(|t| t["name"] == "leviathan_edit_batch")
        .unwrap();
    let variants = batch["inputSchema"]["properties"]["operations"]["items"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 55);
    for variant in variants {
        assert_eq!(variant["additionalProperties"], false);
        assert!(variant["properties"]["op"]["const"].is_string());
    }
}

#[test]
fn clone_amplification_is_rejected_per_operation_before_commit() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let result = edit(
        &mut c,
        0,
        vec![json!({"op":"upsert_pattern","id":1,"name":"x".repeat(256*1024),"rows":32})],
    );
    assert_eq!(result["ok"], true);
    let before = c.document();
    let clones = (100..355)
        .map(|id| json!({"op":"clone_pattern","source_id":1,"new_id":id}))
        .collect();
    let failed = edit(&mut c, 1, clones);
    assert_eq!(failed["error"]["code"], "document_too_large");
    assert!(
        failed["error"]["details"]["operation_index"]
            .as_u64()
            .unwrap()
            < 64
    );
    assert_eq!(c.document(), before);
}

#[test]
fn cut_offset_reverse_are_discoverable_and_render_through_the_shared_engine() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let catalog = c.request("tools/list", json!({}));
    let batch = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "leviathan_edit_batch")
        .unwrap();
    for name in ["cut", "offset", "direction"] {
        assert!(batch["inputSchema"]["$defs"]["effects"]["properties"]
            .get(name)
            .is_some());
    }
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"sample.wav"}),
    );
    let edited = edit(
        &mut c,
        0,
        vec![
            json!({"op":"set_song","bpm":400,"lines_per_beat":256}),
            json!({"op":"upsert_pattern","id":1,"name":"Reverse","rows":4}),
            json!({"op":"upsert_instrument","id":1,"name":"Sample","sample":"sample.wav"}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.2,"effects":{"offset":100,"direction":"Reverse","cut":0.5}}}),
        ],
    );
    assert_eq!(edited["ok"], true, "{edited}");
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":1}));
    assert_eq!(evaluated["data"]["manifest"]["schema"], 3);
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":1,"path":"reverse.wav"}),
    );
    assert!(hound::WavReader::open(f.root.join("reverse.wav"))
        .unwrap()
        .samples::<f32>()
        .any(|sample| sample.unwrap() != 0.0));
    let before = c.document();
    let invalid = edit(
        &mut c,
        1,
        vec![
            json!({"op":"set_effects","pattern_id":1,"track_id":1,"row":0,"column":0,"effects":{"cut":0.5,"release":0.5}}),
        ],
    );
    assert_eq!(invalid["ok"], false);
    assert_eq!(c.document(), before);
}

#[test]
fn instrument_rename_cannot_silently_clear_an_omitted_sample_assignment() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"sample.wav"}),
    );
    let assigned = edit(
        &mut c,
        0,
        vec![json!({"op":"upsert_instrument","id":1,"name":"Assigned","sample":"sample.wav"})],
    );
    assert_eq!(assigned["ok"], true);
    let before = c.document();
    let missing = edit(
        &mut c,
        1,
        vec![json!({"op":"upsert_instrument","id":1,"name":"Renamed"})],
    );
    assert_eq!(missing["error"]["code"], "invalid_arguments");
    assert_eq!(c.document(), before);
    let cleared = edit(
        &mut c,
        1,
        vec![json!({"op":"upsert_instrument","id":1,"name":"Unassigned","sample":null})],
    );
    assert_eq!(cleared["ok"], true);
    assert!(c.document()["data"]["song"]["instruments"]["1"]["sample"].is_null());
}

fn phrase_metadata(id: u64) -> Value {
    json!({"op":"upsert_phrase","id":id,"name":"Instrument phrase","instrument_id":1,"rows":4,"lines_per_beat":8,"note_columns":2,"root_note":60})
}

#[test]
fn instrument_phrase_subprocess_roundtrip_transposes_repeats_exports_and_undoes_atomically() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let empty = c.document();
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"phrase.wav"}),
    );
    let authored = edit(
        &mut c,
        0,
        vec![
            json!({"op":"set_song","title":"Phrase sketch","bpm":120,"lines_per_beat":4}),
            json!({"op":"upsert_instrument","id":1,"name":"Phrase instrument","sample":"phrase.wav"}),
            json!({"op":"upsert_pattern","id":1,"name":"Triggers","rows":8}),
            phrase_metadata(41),
            json!({"op":"set_phrase_cell","phrase_id":41,"row":0,"column":0,"cell":{"note":{"Note":60},"velocity":0.2}}),
            json!({"op":"set_phrase_cell","phrase_id":41,"row":2,"column":0,"cell":{"note":{"Note":64},"velocity":0.3}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":72},"instrument":1,"velocity":0.5,"pan":0.25}}),
            json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":{"phrase":41,"cycles":2}}),
        ],
    );
    assert_eq!(authored["ok"], true, "{authored}");
    assert_eq!(authored["revision"], 1);
    assert_eq!(authored["data"]["session"]["phrase_count"], 1);
    assert_eq!(authored["data"]["session"]["phrase_trigger_cell_count"], 1);
    let phrases = c.ok("leviathan_project_read", json!({"section":"phrases"}));
    assert_eq!(phrases["data"]["phrases"]["41"]["instrument"], 1);
    assert_eq!(
        phrases["data"]["phrases"]["41"]["cells"]["2"]["0"]["note"]["Note"],
        64
    );
    let before = c.document()["data"]["song"].clone();
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":1}));
    assert_eq!(evaluated["data"]["manifest"]["schema"], 4);
    let events = evaluated["data"]["manifest"]["events"].as_array().unwrap();
    assert!(events
        .iter()
        .filter(|event| event["command"].get("On").is_some())
        .all(|event| event["phrase"]["phrase"] == 41));
    let capabilities = c.ok("leviathan_capabilities", json!({}));
    assert!(capabilities["data"]["available"]
        .as_array()
        .unwrap()
        .contains(&json!("instrument_bound_phrases")));
    assert_eq!(capabilities["data"]["phrases"]["nested_triggers"], false);
    assert_eq!(
        capabilities["data"]["limits"]["max_phrase_definitions"],
        128
    );
    let starts: Vec<_> = events
        .iter()
        .filter_map(|event| {
            event["command"].get("On").map(|on| {
                (
                    event["frame"].as_u64().unwrap(),
                    on["pitch"].as_u64().unwrap(),
                )
            })
        })
        .collect();
    assert_eq!(starts, vec![(0, 72), (6000, 76), (12000, 72), (18000, 76)]);
    let exported = c.ok(
        "leviathan_song_export",
        json!({"expected_revision":1,"path":"phrase-song.wav"}),
    );
    assert!(exported["data"]["stats"]["peak"].as_f64().unwrap() > 0.0);
    let stored_manifest: aqua_pattern_core::song_render::Manifest =
        serde_json::from_slice(&fs::read(f.root.join("phrase-song.events.json")).unwrap()).unwrap();
    let evaluated_manifest: aqua_pattern_core::song_render::Manifest =
        serde_json::from_value(evaluated["data"]["manifest"].clone()).unwrap();
    assert_eq!(stored_manifest, evaluated_manifest);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":1,"path":"phrase-song.leviathan.json"}),
    );
    let stored: Value =
        serde_json::from_slice(&fs::read(f.root.join("phrase-song.leviathan.json")).unwrap())
            .unwrap();
    assert_eq!(stored["song"]["phrases"]["41"]["root_note"], 60);
    assert_eq!(
        stored["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["phrase"]["cycles"],
        2
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], empty["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before);
    c.ok("leviathan_project_new", json!({"expected_revision":4}));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"phrase-song.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], before);
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":6,"path":"phrase-reopened.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("phrase-song.wav")).unwrap(),
        fs::read(f.root.join("phrase-reopened.wav")).unwrap()
    );
}

#[test]
fn phrase_mutations_use_shared_reference_bounds_and_preserve_the_old_revision_on_failure() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let setup = edit(
        &mut c,
        0,
        vec![
            phrase_metadata(1),
            json!({"op":"set_phrase_cell","phrase_id":1,"row":3,"column":1,"cell":{"note":{"Note":60},"velocity":0.25}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.4,"phrase":{"phrase":1,"cycles":1}}}),
        ],
    );
    assert_eq!(setup["ok"], true, "{setup}");
    let before = c.document();
    let mut shrink_rows = phrase_metadata(1);
    shrink_rows["rows"] = json!(3);
    let mut shrink_columns = phrase_metadata(1);
    shrink_columns["note_columns"] = json!(1);
    let mut missing_instrument = phrase_metadata(1);
    missing_instrument["instrument_id"] = json!(999);
    let mut too_long = phrase_metadata(1);
    too_long["rows"] = json!(1025);
    let mut too_many_columns = phrase_metadata(1);
    too_many_columns["note_columns"] = json!(17);
    let invalid = vec![
        json!({"op":"remove_phrase","id":1}),
        shrink_rows,
        shrink_columns,
        missing_instrument,
        too_long,
        too_many_columns,
        json!({"op":"set_phrase_cell","phrase_id":1,"row":4,"column":0,"cell":{"note":"Off"}}),
        json!({"op":"set_phrase_cell","phrase_id":1,"row":0,"column":2,"cell":{"note":"Off"}}),
        json!({"op":"set_phrase_cell","phrase_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":2}}),
        json!({"op":"set_phrase_cell","phrase_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"phrase":{"phrase":1,"cycles":1}}}),
        json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":{"phrase":999,"cycles":1}}),
        json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":{"phrase":1,"cycles":0}}),
        json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":{"phrase":1,"cycles":65}}),
        json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":1,"column":0,"trigger":{"phrase":1,"cycles":1}}),
    ];
    for operation in invalid {
        let failed = edit(&mut c, 1, vec![title("must roll back"), operation.clone()]);
        assert_eq!(
            failed["ok"], false,
            "accepted invalid phrase operation: {operation}"
        );
        assert_eq!(failed["error"]["details"]["operation_index"], 1);
        assert_eq!(c.document(), before);
    }
    let omitted = edit(
        &mut c,
        1,
        vec![json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0})],
    );
    assert_eq!(omitted["error"]["code"], "invalid_arguments");
    assert_eq!(c.document(), before);
    let cleared = edit(
        &mut c,
        1,
        vec![
            json!({"op":"clear_phrase_cell","phrase_id":1,"row":3,"column":1}),
            json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":null}),
            json!({"op":"remove_phrase","id":1}),
        ],
    );
    assert_eq!(cleared["ok"], true, "{cleared}");
    let after = c.document();
    let cell = &after["data"]["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"];
    assert!(cell.get("phrase").is_none());
    assert_eq!(cell["note"]["Note"], 60);
    assert_eq!(cell["velocity"], json!(0.4f32));
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn phrase_definition_limit_and_transposition_rejections_are_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let initial = c.document();
    let too_many = edit(&mut c, 0, (1..=129).map(phrase_metadata).collect());
    assert_eq!(too_many["ok"], false);
    assert_eq!(too_many["error"]["details"]["operation_index"], 128);
    assert_eq!(c.document(), initial);
    let valid = edit(
        &mut c,
        0,
        vec![
            phrase_metadata(1),
            json!({"op":"set_phrase_cell","phrase_id":1,"row":0,"column":0,"cell":{"note":{"Note":120}}}),
        ],
    );
    assert_eq!(valid["ok"], true);
    let before = c.document();
    let outside_pitch = edit(
        &mut c,
        1,
        vec![
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":80},"instrument":1,"phrase":{"phrase":1,"cycles":1}}}),
        ],
    );
    assert_eq!(outside_pitch["ok"], false);
    assert_eq!(c.document(), before);
}

#[test]
fn old_instrument_operations_preserve_phrases_and_plain_documents_remain_phrase_free() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let plain = c.document();
    assert!(plain["data"]["song"].get("phrases").is_none());
    let no_op = edit(&mut c, 0, vec![title("Untitled")]);
    assert_eq!(no_op["revision"], 0);
    let created = edit(&mut c, 0, vec![phrase_metadata(7)]);
    assert_eq!(created["ok"], true);
    let phrases =
        c.ok("leviathan_project_read", json!({"section":"phrases"}))["data"]["phrases"].clone();
    let renamed = edit(
        &mut c,
        1,
        vec![
            json!({"op":"upsert_instrument","id":1,"name":"Renamed by legacy client","sample":null}),
        ],
    );
    assert_eq!(renamed["ok"], true);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"phrases"}))["data"]["phrases"],
        phrases
    );
    let before = c.document();
    let removed = edit(&mut c, 2, vec![json!({"op":"remove_instrument","id":1})]);
    assert_eq!(removed["ok"], false);
    assert_eq!(c.document(), before);
}

#[test]
fn exposed_editor_and_mcp_phrase_operations_have_persistent_and_evaluator_parity() {
    use aqua_pattern_core::{
        editor::Editor,
        session::{Action, Session},
        song::{Cell, Note, Phrase, PhraseId, PhraseTrigger, TrackId},
        song_render,
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"parity.wav"}),
    );
    let mut editor = Editor::default();
    Session::default()
        .request(&mut editor, Action::New)
        .unwrap();
    editor.edit_mode = true;
    editor
        .assign_sample(InstrumentId(1), Some(f.root.join("parity.wav")))
        .unwrap();
    editor
        .upsert_phrase(
            PhraseId(41),
            Phrase {
                name: "Instrument phrase".into(),
                instrument: InstrumentId(1),
                rows: 4,
                lines_per_beat: 8,
                note_columns: 2,
                root_note: 60,
                cells: Default::default(),
            },
        )
        .unwrap();
    editor
        .set_phrase_cell(
            PhraseId(41),
            0,
            0,
            Cell {
                note: Note::Note(60),
                velocity: Some(0.2),
                pan: Some(-0.2),
                ..Default::default()
            },
        )
        .unwrap();
    editor.select(0, TrackId(1)).unwrap();
    editor.row = 0;
    editor.column = 0;
    editor
        .set_cell(Cell {
            note: Note::Note(72),
            instrument: Some(InstrumentId(1)),
            velocity: Some(0.5),
            pan: Some(0.25),
            delay: Some(0.1),
            ..Default::default()
        })
        .unwrap();
    editor
        .set_phrase_trigger(Some(PhraseTrigger {
            phrase: PhraseId(41),
            cycles: 2,
        }))
        .unwrap();
    let result = edit(
        &mut c,
        0,
        vec![
            json!({"op":"upsert_instrument","id":1,"name":"Instrument 1","sample":"parity.wav"}),
            phrase_metadata(41),
            json!({"op":"set_phrase_cell","phrase_id":41,"row":0,"column":0,"cell":{"note":{"Note":60},"velocity":0.2,"pan":-0.2}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":72},"instrument":1,"velocity":0.5,"pan":0.25,"delay":0.1}}),
            json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":{"phrase":41,"cycles":2}}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let from_mcp: aqua_pattern_core::song::Song =
        serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":1}));
    let from_mcp: song_render::Manifest =
        serde_json::from_value(evaluated["data"]["manifest"].clone()).unwrap();
    assert_eq!(from_mcp, song_render::evaluate(&editor.song, None).unwrap());
    let mut renamed = editor.song.phrases[&PhraseId(41)].clone();
    renamed.name = "Renamed phrase".into();
    editor.upsert_phrase(PhraseId(41), renamed).unwrap();
    let mut rename = phrase_metadata(41);
    rename["name"] = json!("Renamed phrase");
    assert_eq!(edit(&mut c, 1, vec![rename])["ok"], true);
    let from_mcp: aqua_pattern_core::song::Song =
        serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    assert!(!from_mcp.phrases[&PhraseId(41)].cells.is_empty());
    editor.set_phrase_trigger(None).unwrap();
    editor
        .set_phrase_cell(PhraseId(41), 0, 0, Cell::default())
        .unwrap();
    editor.remove_phrase(PhraseId(41)).unwrap();
    let removed = edit(
        &mut c,
        2,
        vec![
            json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":null}),
            json!({"op":"clear_phrase_cell","phrase_id":41,"row":0,"column":0}),
            json!({"op":"remove_phrase","id":41}),
        ],
    );
    assert_eq!(removed["ok"], true, "{removed}");
    let from_mcp: aqua_pattern_core::song::Song =
        serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
}

fn write_onset_source(path: &std::path::Path, rate: u32, frames: u32, channels: u16) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for frame in 0..frames {
        let sample = if [frames / 8, frames / 2, frames - 1].contains(&frame) {
            0.5f32
        } else {
            0.0
        };
        for channel in 0..channels {
            writer
                .write_sample(if channel == 0 { sample } else { -sample })
                .unwrap();
        }
    }
    writer.finalize().unwrap();
}
fn region_edit(id: u64, start: u64, end: u64) -> Value {
    json!({"op":"upsert_region","id":id,"name":format!("Slice {id}"),"sample_id":7,"start_frame":start,"end_frame":end,"root_note":60})
}
fn import_source(c: &mut Client, revision: u64, id: u64, path: &str) -> Value {
    c.ok(
        "leviathan_sample_import",
        json!({"expected_revision":revision,"sample_id":id,"path":path}),
    )
}

#[test]
fn source_import_is_native_verified_immutable_idempotent_and_undoable_for_wav_and_flac() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 44100, 44100, 1);
    fs::copy(
        "tests/fixtures/slice-stereo-44100.flac",
        f.root.join("source.flac"),
    )
    .unwrap();
    let mut c = Client::ready(&f);
    let original = c.document()["data"]["song"].clone();
    assert!(original.get("samples").is_none());
    assert!(original.get("regions").is_none());
    let imported = import_source(&mut c, 0, 7, "source.wav");
    assert_eq!(imported["revision"], 1);
    assert_eq!(imported["data"]["assigned"], false);
    assert_eq!(imported["data"]["sample"]["sample_rate"], 44100);
    assert_eq!(imported["data"]["sample"]["frames"], 44100);
    assert_eq!(imported["data"]["sample"]["channels"], 1);
    assert_eq!(
        imported["data"]["sample"]["sha256"].as_str().unwrap().len(),
        64
    );
    let before = c.document();
    assert_eq!(
        import_source(&mut c, 1, 7, "source.wav")["data"]["changed"],
        false
    );
    assert_eq!(c.document(), before);
    let inspected = c.ok(
        "leviathan_sample_inspect",
        json!({"expected_revision":1,"sample_id":7}),
    );
    assert_eq!(inspected["data"]["identity_and_metadata_verified"], true);
    assert_eq!(inspected["data"]["sample"], imported["data"]["sample"]);
    assert_eq!(
        c.call(
            "leviathan_sample_import",
            json!({"expected_revision":1,"sample_id":7,"path":"source.flac"})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    let flac = import_source(&mut c, 1, 8, "source.flac");
    assert_eq!(flac["data"]["sample"]["sample_rate"], 44100);
    assert_eq!(flac["data"]["sample"]["channels"], 2);
    let analyzed = c.ok(
        "leviathan_sample_analyze",
        json!({"expected_revision":2,"sample_id":8,"config":{"waveform_bins":8}}),
    );
    assert_eq!(
        analyzed["data"]["analysis"]["provenance"]["source_frames"],
        flac["data"]["sample"]["frames"]
    );
    assert_eq!(analyzed["data"]["analysis"]["provenance"]["channels"], 2);
    assert_eq!(
        c.ok("leviathan_workspace_list", json!({}))["data"]["files"],
        json!(["source.flac", "source.wav"])
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], original);
    assert!(f.root.join("source.wav").exists());
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn imported_regions_analysis_export_save_reopen_and_single_batch_undo_share_persistent_state() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 44100, 44100, 2);
    let bytes = fs::read(f.root.join("source.wav")).unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let source_only = c.document()["data"]["song"].clone();
    let applied = edit(
        &mut c,
        1,
        vec![
            region_edit(21, 5000, 12000),
            region_edit(22, 20000, 30000),
            json!({"op":"assign_region","instrument_id":1,"region_id":21}),
            json!({"op":"assign_region","instrument_id":2,"region_id":22}),
            json!({"op":"set_song","bpm":170}),
            json!({"op":"upsert_pattern","id":1,"name":"Mapped source","rows":4}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.25}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":2,"row":2,"column":0,"cell":{"note":{"Note":60},"instrument":2,"velocity":0.25}}),
        ],
    );
    assert_eq!(applied["ok"], true, "{applied}");
    let before = c.document();
    assert_eq!(before["data"]["session"]["sample_count"], 1);
    assert_eq!(before["data"]["session"]["region_count"], 2);
    let analyzed = c.ok(
        "leviathan_sample_analyze",
        json!({"expected_revision":2,"sample_id":7,"config":{"waveform_bins":16}}),
    );
    assert_eq!(analyzed["revision"], 2);
    assert_eq!(analyzed["data"]["document_changed"], false);
    assert_eq!(
        analyzed["data"]["analysis"]["provenance"]["source_frames"],
        44100
    );
    let frames: Vec<_> = analyzed["data"]["analysis"]["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["frame"].as_u64().unwrap())
        .collect();
    assert_eq!(frames, vec![5512, 22050, 44099]);
    assert_eq!(
        analyzed["data"]["analysis"]["waveform"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["end_frame"],
        44100
    );
    assert_eq!(
        c.document(),
        before,
        "Analysis cannot alter accepted region boundaries or history"
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"regions"}))["data"]["regions"],
        before["data"]["song"]["regions"]
    );
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    assert_eq!(manifest["data"]["manifest"]["schema"], 5);
    assert_eq!(manifest["data"]["source_identity_verified"], false);
    assert_eq!(
        manifest["data"]["manifest"]["regions"]["1"]["sample_rate"],
        44100
    );
    assert_eq!(
        manifest["data"]["manifest"]["regions"]["1"]["start_frame"],
        5000
    );
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":2,"path":"mapped.wav"}),
    );
    assert!(hound::WavReader::open(f.root.join("mapped.wav"))
        .unwrap()
        .samples::<f32>()
        .any(|v| v.unwrap() != 0.0));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":2,"path":"mapped.leviathan.json"}),
    );
    let saved: Value =
        serde_json::from_slice(&fs::read(f.root.join("mapped.leviathan.json")).unwrap()).unwrap();
    assert_eq!(saved["song"]["samples"]["7"]["path"], "source.wav");
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], source_only);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"mapped.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":6,"path":"reopened.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("mapped.wav")).unwrap(),
        fs::read(f.root.join("reopened.wav")).unwrap()
    );
    assert_eq!(fs::read(f.root.join("source.wav")).unwrap(), bytes);
}

#[test]
fn region_reference_bounds_source_switches_and_invalid_batches_are_explicit_and_atomic() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 1000, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 0, 1000),
                json!({"op":"assign_region","instrument_id":1,"region_id":21})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    for op in [
        region_edit(22, 500, 500),
        region_edit(22, 0, 1001),
        region_edit(0, 0, 1),
        json!({"op":"upsert_region","id":22,"name":"bad","sample_id":8,"start_frame":0,"end_frame":1,"root_note":60}),
        json!({"op":"remove_sample","id":7}),
        json!({"op":"remove_region","id":21}),
        json!({"op":"assign_region","instrument_id":99,"region_id":21}),
        json!({"op":"assign_region","instrument_id":1,"region_id":99}),
    ] {
        let failed = edit(&mut c, 2, vec![title("Should roll back"), op]);
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(failed["error"]["details"]["operation_index"], 1);
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![json!({"op":"assign_region","instrument_id":1})]
        )["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![json!({"op":"upsert_instrument","id":1,"name":"Renamed","sample":null})]
        )["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["song"]["instruments"]["1"]["region"],
        21
    );
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![json!({"op":"upsert_instrument","id":1,"name":"Legacy","sample":"source.wav"})]
        )["ok"],
        true
    );
    let legacy = c.document();
    assert!(legacy["data"]["song"]["instruments"]["1"]
        .get("region")
        .is_none());
    assert!(legacy["data"]["song"]["instruments"]["1"]["sample"].is_string());
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"assign_region","instrument_id":1,"region_id":21})]
        )["ok"],
        true
    );
    assert!(c.document()["data"]["song"]["instruments"]["1"]["sample"].is_null());
    let mapped = c.document()["data"]["song"].clone();
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![
                json!({"op":"assign_region","instrument_id":1,"region_id":null}),
                json!({"op":"remove_region","id":21}),
                json!({"op":"remove_sample","id":7})
            ]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    assert_eq!(c.document()["data"]["song"], mapped);
    assert!(f.root.join("source.wav").exists());
}

#[test]
fn transient_suggestions_are_bounded_read_only_deterministic_and_changed_sources_fail_closed() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 1);
    write_onset_source(&f.root.join("long.wav"), 8000, 8000 * 61, 1);
    write_onset_source(&f.root.join("low-rate.wav"), 7999, 100, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    import_source(&mut c, 1, 8, "long.wav");
    import_source(&mut c, 2, 9, "low-rate.wav");
    let before = c.document();
    let args = json!({"expected_revision":3,"sample_id":7,"config":{"waveform_bins":0}});
    let first = c.ok("leviathan_sample_analyze", args.clone());
    assert_eq!(first, c.ok("leviathan_sample_analyze", args));
    assert_eq!(first["data"]["analysis"]["waveform"], json!([]));
    for config in [
        json!({"max_suggestions":1}),
        json!({"max_suggestions":4097}),
        json!({"waveform_bins":2049}),
        json!({"sensitivity":2}),
        json!({"min_spacing_ms":0}),
        json!({"minimum_rms":-1}),
        json!({"max_frames":1}),
    ] {
        assert_eq!(
            c.call(
                "leviathan_sample_analyze",
                json!({"expected_revision":3,"sample_id":7,"config":config})
            )["ok"],
            false
        );
        assert_eq!(c.document(), before);
    }
    for sample_id in [8, 9] {
        assert_eq!(
            c.call(
                "leviathan_sample_analyze",
                json!({"expected_revision":3,"sample_id":sample_id})
            )["error"]["code"],
            "analysis_limit"
        );
    }
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                region_edit(21, 0, 48000),
                json!({"op":"assign_region","instrument_id":1,"region_id":21}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1}})
            ]
        )["ok"],
        true
    );
    let mapped = c.document();
    // Valid audio with the same metadata, but different bytes, is still a changed source.
    let mut changed = fs::read(f.root.join("source.wav")).unwrap();
    *changed.last_mut().unwrap() ^= 1;
    fs::write(f.root.join("source.wav"), changed).unwrap();
    for (tool, args) in [
        (
            "leviathan_sample_inspect",
            json!({"expected_revision":4,"sample_id":7}),
        ),
        (
            "leviathan_sample_analyze",
            json!({"expected_revision":4,"sample_id":7}),
        ),
        (
            "leviathan_song_export",
            json!({"expected_revision":4,"path":"denied.wav"}),
        ),
        (
            "leviathan_project_save",
            json!({"expected_revision":4,"path":"denied.leviathan.json"}),
        ),
    ] {
        let failed = c.call(tool, args);
        assert_eq!(failed["ok"], false, "{failed}");
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("SHA-256"),
            "{failed}"
        );
        assert_eq!(c.document(), mapped);
    }
    assert!(!f.root.join("denied.wav").exists());
    assert!(!f.root.join("denied.events.json").exists());
    assert!(!f.root.join("denied.leviathan.json").exists());
}

#[test]
fn import_rejects_escape_untrusted_metadata_and_corrupt_or_over_budget_sources_without_edits() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 480, 1);
    fs::write(f.root.join("corrupt.wav"), b"not an audio file").unwrap();
    fs::File::create(f.root.join("large.wav"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let mut c = Client::ready(&f);
    let before = c.document();
    for path in [
        "../outside.wav".to_owned(),
        f.base.join("outside.wav").to_string_lossy().into_owned(),
        "corrupt.wav".into(),
        "large.wav".into(),
    ] {
        assert_eq!(
            c.call(
                "leviathan_sample_import",
                json!({"expected_revision":0,"sample_id":7,"path":path})
            )["ok"],
            false
        );
        assert_eq!(c.document(), before);
    }
    for args in [
        json!({"expected_revision":0,"sample_id":0,"path":"source.wav"}),
        json!({"expected_revision":1,"sample_id":7,"path":"source.wav"}),
        json!({"expected_revision":0,"sample_id":7,"path":"source.wav","sha256":"client-controlled"}),
    ] {
        assert_eq!(c.call("leviathan_sample_import", args)["ok"], false);
        assert_eq!(c.document(), before);
    }
    import_source(&mut c, 0, 7, "source.wav");
    let mut escaped = c.document()["data"]["song"].clone();
    escaped["samples"]["7"]["path"] = json!(f.base.join("missing-outside.wav"));
    fs::write(
        f.root.join("escape.leviathan.json"),
        serde_json::to_vec(&json!({"format":"leviathan-song","version":1,"song":escaped})).unwrap(),
    )
    .unwrap();
    let imported = c.document();
    let failed = c.call(
        "leviathan_project_open",
        json!({"expected_revision":1,"discard_unsaved":true,"path":"escape.leviathan.json"}),
    );
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("outside the startup workspace"),
        "{failed}"
    );
    assert_eq!(c.document(), imported);
}

#[cfg(unix)]
#[test]
fn import_and_later_source_access_reject_symlinks_before_decoding() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    write_onset_source(&f.base.join("outside.wav"), 48000, 480, 1);
    symlink(f.base.join("outside.wav"), f.root.join("link.wav")).unwrap();
    symlink(&f.base, f.root.join("outside")).unwrap();
    let mut c = Client::ready(&f);
    for path in ["link.wav", "outside/outside.wav"] {
        assert_eq!(
            c.call(
                "leviathan_sample_import",
                json!({"expected_revision":0,"sample_id":7,"path":path})
            )["error"]["code"],
            "path_denied"
        );
    }
    fs::copy(f.base.join("outside.wav"), f.root.join("source.wav")).unwrap();
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    fs::remove_file(f.root.join("source.wav")).unwrap();
    symlink(f.base.join("outside.wav"), f.root.join("source.wav")).unwrap();
    for tool in ["leviathan_sample_inspect", "leviathan_sample_analyze"] {
        assert_eq!(
            c.call(tool, json!({"expected_revision":1,"sample_id":7}))["error"]["code"],
            "path_denied"
        );
        assert_eq!(c.document(), before);
    }
}

#[test]
fn source_region_mapping_operations_have_exact_editor_mcp_and_evaluator_parity() {
    use aqua_pattern_core::{
        editor::Editor,
        sample_asset,
        session::{Action, Session},
        song::{Cell, Note, RegionId, SampleId, SampleRegion, TrackId},
        song_render,
    };
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 44100, 44100, 1);
    let mut c = Client::ready(&f);
    let mut editor = Editor::default();
    Session::default()
        .request(&mut editor, Action::New)
        .unwrap();
    editor.edit_mode = true;
    editor
        .register_sample(
            SampleId(7),
            sample_asset::import(&f.root.join("source.wav"))
                .unwrap()
                .asset,
        )
        .unwrap();
    let region = SampleRegion {
        name: "Slice 21".into(),
        sample: SampleId(7),
        start_frame: 5000,
        end_frame: 12000,
        root_note: 60,
        amp_envelope: None,
        forward_loop: None,
    };
    editor.upsert_region(RegionId(21), region).unwrap();
    editor
        .assign_region(InstrumentId(1), Some(RegionId(21)))
        .unwrap();
    editor.select(0, TrackId(1)).unwrap();
    editor
        .set_cell(Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            ..Default::default()
        })
        .unwrap();
    import_source(&mut c, 0, 7, "source.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 5000, 12000),
                json!({"op":"assign_region","instrument_id":1,"region_id":21}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1}})
            ]
        )["ok"],
        true
    );
    let from_mcp: aqua_pattern_core::song::Song =
        serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    let from_mcp: song_render::Manifest =
        serde_json::from_value(evaluated["data"]["manifest"].clone()).unwrap();
    assert_eq!(from_mcp, song_render::evaluate(&editor.song, None).unwrap());
    editor.assign_region(InstrumentId(1), None).unwrap();
    editor.remove_region(RegionId(21)).unwrap();
    editor.remove_sample(SampleId(7)).unwrap();
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"assign_region","instrument_id":1,"region_id":null}),
                json!({"op":"remove_region","id":21}),
                json!({"op":"remove_sample","id":7})
            ]
        )["ok"],
        true
    );
    let from_mcp: aqua_pattern_core::song::Song =
        serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
}

fn marker_edit(id: u64, bpm: Value, markers: Vec<u64>) -> Value {
    json!({"op":"upsert_marker_set","id":id,"name":"Accepted boundaries","sample_id":7,"source_bpm":bpm,"markers":markers})
}
fn break_config() -> Value {
    json!({"marker_set":31,"target_bpm":170,"timing":"SourceRhythm","cycles":1,"roll":null,"tail":"CutPrevious","minimum_slice_frames":1,"root_note":60,"pad_tails":false,"name":"Compiled break"})
}
fn write_seventy_bpm_source(path: &std::path::Path) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..151200u32 {
        let phase = frame % 37800;
        writer
            .write_sample(if phase < 256 {
                0.1f32 + phase as f32 / 10000.0
            } else {
                0.0
            })
            .unwrap();
    }
    writer.finalize().unwrap();
}
fn compiler_fixture(c: &mut Client, f: &Fixture) -> Value {
    write_seventy_bpm_source(&f.root.join("break70.wav"));
    import_source(c, 0, 7, "break70.wav");
    let edited = edit(
        c,
        1,
        vec![
            marker_edit(31, json!(70), vec![0, 37800, 75600, 113400, 151200]),
            json!({"op":"set_order","pattern_ids":[]}),
        ],
    );
    assert_eq!(edited["ok"], true, "{edited}");
    c.document()
}
fn compile_preview(c: &mut Client, revision: u64, config: Value) -> Value {
    c.ok(
        "leviathan_break_compile_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn compile_commit(c: &mut Client, revision: u64, preview: &Value) -> Value {
    c.call(
        "leviathan_break_compile_commit",
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}

#[test]
fn accepted_marker_sets_are_persistent_validated_undoable_and_distinct_from_analysis() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    assert!(c.document()["data"]["song"].get("marker_sets").is_none());
    let accepted = marker_edit(31, json!(70), vec![0, 12000, 48000]);
    assert_eq!(edit(&mut c, 1, vec![accepted.clone()])["ok"], true);
    let before = c.document();
    assert_eq!(before["data"]["session"]["marker_set_count"], 1);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"marker_sets"}))["data"]["marker_sets"],
        before["data"]["song"]["marker_sets"]
    );
    c.ok(
        "leviathan_sample_analyze",
        json!({"expected_revision":2,"sample_id":7}),
    );
    assert_eq!(c.document(), before);
    assert_eq!(edit(&mut c, 2, vec![accepted])["data"]["changed"], false);
    assert_eq!(c.document(), before);
    for bad in [
        marker_edit(31, json!(70), vec![0, 48000, 12000]),
        marker_edit(31, json!(70), vec![0, 12000, 12000, 48000]),
        marker_edit(31, json!(70), vec![1, 48000]),
        marker_edit(31, json!(70), vec![0, 47999]),
        marker_edit(31, json!(19), vec![0, 48000]),
        marker_edit(0, json!(70), vec![0, 48000]),
        marker_edit(
            31,
            json!(70),
            (0..257).chain(std::iter::once(48000)).collect(),
        ),
        json!({"op":"remove_sample","id":7}),
        json!({"op":"upsert_marker_set","id":31,"name":"missing","sample_id":99,"source_bpm":70,"markers":[0,48000]}),
    ] {
        let failed = edit(&mut c, 2, vec![title("rollback"), bad]);
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(failed["error"]["details"]["operation_index"], 1);
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"upsert_marker_set","id":31,"name":"missing bpm field","sample_id":7,"markers":[0,48000]})
            ]
        )["error"]["code"],
        "invalid_arguments"
    );
    let too_many = (100..229)
        .map(|id| marker_edit(id, Value::Null, vec![0, 48000]))
        .collect();
    let failed = edit(&mut c, 2, too_many);
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 127);
    assert_eq!(c.document(), before);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":2,"path":"markers.leviathan.json"}),
    );
    assert_eq!(
        edit(&mut c, 3, vec![json!({"op":"remove_marker_set","id":31})])["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"markers.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn compiler_preview_commit_70_to_170_is_one_edit_with_native_pitch_and_source_preserved() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = compiler_fixture(&mut c, &f);
    let source_bytes = fs::read(f.root.join("break70.wav")).unwrap();
    let preview = compile_preview(&mut c, 2, break_config());
    assert_eq!(preview["revision"], 2);
    assert_eq!(preview["data"]["document_changed"], false);
    assert_eq!(preview["data"]["source_identity_verified"], true);
    assert!(preview["data"]["diff"].is_object());
    assert_eq!(c.document(), before);
    let committed = compile_commit(&mut c, 2, &preview);
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["revision"], 3);
    assert_eq!(committed["data"]["preview_consumed"], true);
    let after = c.document();
    let song = &after["data"]["song"];
    assert_eq!(song["bpm"], 170.0);
    assert_eq!(
        song["lines_per_beat"],
        before["data"]["song"]["lines_per_beat"]
    );
    assert_eq!(song["regions"].as_object().unwrap().len(), 4);
    assert_eq!(
        song["instruments"].as_object().unwrap().len(),
        before["data"]["song"]["instruments"]
            .as_object()
            .unwrap()
            .len()
            + 4
    );
    assert_eq!(song["patterns"].as_object().unwrap().len(), 2);
    assert_eq!(
        song["tracks"].as_object().unwrap().len(),
        before["data"]["song"]["tracks"].as_object().unwrap().len() + 1
    );
    assert_eq!(song["order"].as_array().unwrap().len(), 1);
    assert_eq!(song["marker_sets"], before["data"]["song"]["marker_sets"]);
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let manifest: aqua_pattern_core::song_render::Manifest =
        serde_json::from_value(evaluated["data"]["manifest"].clone()).unwrap();
    let starts: Vec<_> = manifest
        .events
        .iter()
        .filter_map(|e| {
            matches!(
                e.command,
                aqua_pattern_core::song_render::Command::On { .. }
            )
            .then_some(e.frame)
        })
        .collect();
    assert_eq!(starts, vec![0, 16941, 33882, 50824]);
    for region in manifest.regions.values() {
        assert_eq!(region.sample_rate, 44100);
        assert_eq!(region.root_note, 60);
    }
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":3,"path":"compiled.wav"}),
    );
    let pcm: Vec<f32> = hound::WavReader::open(f.root.join("compiled.wav"))
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    for start in starts {
        for output_frame in 0..279usize {
            let position = output_frame as f64 * 44100.0 / 48000.0;
            let lower = position.floor() as u32;
            let frac = (position - f64::from(lower)) as f32;
            let a = if lower < 256 {
                0.1 + lower as f32 / 10000.0
            } else {
                0.0
            };
            let b = if lower + 1 < 256 {
                0.1 + (lower + 1) as f32 / 10000.0
            } else {
                0.0
            };
            let expected = (a + (b - a) * frac) * 0.5;
            assert!(
                (pcm[(start as usize + output_frame) * 2] - expected).abs() < 0.000002,
                "native pitch changed at start{start}/frame{output_frame}"
            );
        }
    }
    assert_eq!(fs::read(f.root.join("break70.wav")).unwrap(), source_bytes);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"compiled.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":6,"path":"compiled.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":7,"path":"reopened.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("compiled.wav")).unwrap(),
        fs::read(f.root.join("reopened.wav")).unwrap()
    );
}

#[test]
fn compiler_tokens_reject_aba_replacement_save_and_source_tampering_without_partial_commit() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let base = compiler_fixture(&mut c, &f);
    let old = compile_preview(&mut c, 2, break_config());
    let current = compile_preview(&mut c, 2, break_config());
    assert_ne!(
        old["data"]["preview_token"],
        current["data"]["preview_token"]
    );
    assert_eq!(
        compile_commit(&mut c, 2, &old)["error"]["code"],
        "invalid_preview_token"
    );
    assert_eq!(c.document(), base);
    let path = f.root.join("break70.wav");
    let bytes = fs::read(&path).unwrap();
    let mut changed = bytes.clone();
    *changed.last_mut().unwrap() ^= 1;
    fs::write(&path, changed).unwrap();
    assert_eq!(compile_commit(&mut c, 2, &current)["ok"], false);
    assert_eq!(c.document(), base);
    fs::write(&path, &bytes).unwrap();
    // Failed hash check retained the same exact review token.
    assert_eq!(compile_commit(&mut c, 2, &current)["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], base["data"]["song"]);
    assert_eq!(
        compile_commit(&mut c, 4, &current)["error"]["code"],
        "preview_unavailable"
    );
    let stale = compile_preview(&mut c, 4, break_config());
    assert_eq!(edit(&mut c, 4, vec![title("temporary")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], base["data"]["song"]);
    assert_eq!(
        compile_commit(&mut c, 6, &stale)["error"]["code"],
        "preview_unavailable"
    );
    let save_stale = compile_preview(&mut c, 6, break_config());
    fs::create_dir(f.root.join("subdir")).unwrap();
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"subdir/saved.leviathan.json"}),
    );
    assert_eq!(
        compile_commit(&mut c, 7, &save_stale)["error"]["code"],
        "preview_unavailable"
    );
    let open_stale = compile_preview(&mut c, 7, break_config());
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"subdir/saved.leviathan.json"}),
    );
    assert_eq!(
        compile_commit(&mut c, 8, &open_stale)["error"]["code"],
        "preview_unavailable"
    );
    let new_stale = compile_preview(&mut c, 8, break_config());
    c.ok("leviathan_project_new", json!({"expected_revision":8}));
    assert_eq!(
        compile_commit(&mut c, 9, &new_stale)["error"]["code"],
        "preview_unavailable"
    );
}

#[test]
fn marker_cleanup_is_a_bounded_shared_proposal_not_an_implicit_edit() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    let args = json!({"expected_revision":1,"sample_id":7,"candidates":[0,2,50,50,60,47990,48000],"minimum_slice_frames":32});
    let proposal = c.ok("leviathan_marker_cleanup_preview", args);
    assert_eq!(proposal["data"]["document_changed"], false);
    assert_eq!(
        proposal["data"]["cleanup"]["markers"],
        json!([0, 50, 48000])
    );
    assert_eq!(
        proposal["data"]["cleanup"]["removed"],
        json!([
            {"frame":2,"reason":"NearStart"},{"frame":50,"reason":"Duplicate"},
            {"frame":60,"reason":"TooClose"},{"frame":47990,"reason":"NearEnd"}
        ])
    );
    let asset: aqua_pattern_core::song::SampleAsset =
        serde_json::from_value(before["data"]["song"]["samples"]["7"].clone()).unwrap();
    let shared = aqua_pattern_core::break_compiler::suggest_marker_cleanup(
        &asset,
        &[0, 2, 50, 50, 60, 47990, 48000],
        32,
    )
    .unwrap();
    assert_eq!(
        proposal["data"]["cleanup"],
        serde_json::to_value(shared).unwrap()
    );
    assert_eq!(c.document(), before);
    for (candidates, minimum) in [
        (vec![0; 4097], 32),
        (vec![48001], 32),
        (vec![20], 0),
        (vec![20], 48001),
    ] {
        let failed=c.call("leviathan_marker_cleanup_preview",json!({"expected_revision":1,"sample_id":7,"candidates":candidates,"minimum_slice_frames":minimum}));
        assert_eq!(failed["ok"], false);
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![marker_edit(31, json!(70), vec![0, 50, 48000])]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn compiler_shared_editor_plan_and_mcp_have_exact_candidate_diff_and_manifest_parity() {
    use aqua_pattern_core::{
        break_compiler,
        editor::Editor,
        song::{Cell, InstrumentId, Note, Song, TrackId},
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = compiler_fixture(&mut c, &f);
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let config: break_compiler::CompileConfig = serde_json::from_value(break_config()).unwrap();
    let shared = break_compiler::preview(&base, &config).unwrap();
    let preview = compile_preview(&mut c, 2, break_config());
    assert_eq!(preview["data"]["compiler_preview_id"], shared.id());
    assert_eq!(
        preview["data"]["diff"],
        serde_json::to_value(shared.summary()).unwrap()
    );
    editor.commit_preview(editor.revision(), &shared).unwrap();
    assert_eq!(compile_commit(&mut c, 2, &preview)["ok"], true);
    let from_mcp: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, *shared.candidate());
    assert_eq!(from_mcp, editor.song);
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let from_mcp: aqua_pattern_core::song_render::Manifest =
        serde_json::from_value(manifest["data"]["manifest"].clone()).unwrap();
    assert_eq!(
        from_mcp,
        aqua_pattern_core::song_render::evaluate(&editor.song, None).unwrap()
    );
    // Generated output is an ordinary editable pattern, not an opaque renderer recipe.
    let summary = shared.summary();
    let instrument = summary.instruments[0];
    editor
        .select(summary.occurrence, TrackId(summary.track.0))
        .unwrap();
    editor.row = 0;
    editor.column = 0;
    editor
        .set_cell(Cell {
            note: Note::Note(64),
            instrument: Some(InstrumentId(instrument.0)),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"set_cell","pattern_id":summary.pattern.0,"track_id":summary.track.0,"row":0,"column":0,"cell":{"note":{"Note":64},"instrument":instrument.0}})
            ]
        )["ok"],
        true
    );
    let from_mcp: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    let old_patterns = c.document()["data"]["song"]["patterns"].clone();
    assert_eq!(
        edit(&mut c, 4, vec![marker_edit(31, json!(70), vec![0, 151200])])["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["song"]["patterns"],
        old_patterns,
        "Marker edits must not secretly recompile existing ordinary output"
    );
}

#[test]
fn compiler_exact_roll_units_and_all_tail_policies_reach_the_shared_evaluator() {
    for (division, denominator, tail) in [
        ("Sixteenth", 16, "Overlap"),
        ("ThirtySecond", 32, "ReleasePrevious"),
        ("SixtyFourth", 64, "CutPrevious"),
        ("OneTwentyEighth", 128, "CutPrevious"),
    ] {
        let f = Fixture::new();
        let mut c = Client::ready(&f);
        compiler_fixture(&mut c, &f);
        let mut config = break_config();
        config["roll"] = json!({"division":division,"hits":2});
        config["tail"] = json!(tail);
        config["pad_tails"] = json!(true);
        let preview = compile_preview(&mut c, 2, config);
        assert_eq!(preview["data"]["diff"]["note_starts"], 8);
        assert_eq!(compile_commit(&mut c, 2, &preview)["ok"], true);
        let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
        let manifest: aqua_pattern_core::song_render::Manifest =
            serde_json::from_value(manifest["data"]["manifest"].clone()).unwrap();
        let starts: Vec<_> = manifest
            .events
            .iter()
            .filter_map(|e| {
                matches!(
                    e.command,
                    aqua_pattern_core::song_render::Command::On { .. }
                )
                .then_some(e.frame)
            })
            .collect();
        let expected: Vec<u64> = (0..4)
            .flat_map(|slice| {
                (0..2).map(move |hit| {
                    ((slice as f64 + hit as f64 * 4.0 / denominator as f64) * 48000.0 * 60.0
                        / 170.0)
                        .round() as u64
                })
            })
            .collect();
        assert_eq!(starts, expected, "division {division}");
        if tail == "CutPrevious" {
            assert!(manifest.events.iter().any(|e| matches!(
                e.command,
                aqua_pattern_core::song_render::Command::Cut { .. }
            )));
        }
    }
}

#[test]
fn compiler_invalid_configs_and_allocation_limits_preserve_pending_review_and_document() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = compiler_fixture(&mut c, &f);
    let valid = compile_preview(&mut c, 2, break_config());
    for (field, value) in [
        ("target_bpm", json!(401)),
        ("cycles", json!(17)),
        ("root_note", json!(128)),
        ("minimum_slice_frames", json!(37801)),
        ("marker_set", json!(99)),
        ("roll", json!({"division":"Sixteenth","hits":16})),
        ("roll", json!({"division":"OneTwentyEighth","hits":1})),
        (
            "timing",
            json!({"Grid":{"division":"Sixteenth","ignored":true}}),
        ),
        ("seed", json!(123)),
        ("name", json!("x".repeat(257))),
    ] {
        let mut config = break_config();
        config[field] = value;
        let failed = c.call(
            "leviathan_break_compile_preview",
            json!({"expected_revision":2,"config":config}),
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
    }
    assert_eq!(c.call("leviathan_break_compile_commit",json!({"expected_revision":2,"preview_token":valid["data"]["preview_token"],"candidate":{}}))["error"]["code"],"invalid_arguments");
    assert_eq!(
        compile_commit(&mut c, 2, &valid)["ok"],
        true,
        "Invalid previews must not consume the last valid review token"
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![marker_edit(
                31,
                Value::Null,
                vec![0, 37800, 75600, 113400, 151200]
            )]
        )["ok"],
        true
    );
    let unknown_bpm = c.document();
    assert_eq!(
        c.call(
            "leviathan_break_compile_preview",
            json!({"expected_revision":5,"config":break_config()})
        )["ok"],
        false
    );
    assert_eq!(c.document(), unknown_bpm);
    // A valid marker set may exceed compiler start density when repeated; reject atomically.
    let boundaries = (0..=256).map(|n| n * 151200 / 256).collect();
    assert_eq!(
        edit(&mut c, 5, vec![marker_edit(31, json!(70), boundaries)])["ok"],
        true
    );
    let dense = c.document();
    let mut config = break_config();
    config["cycles"] = json!(16);
    assert_eq!(
        c.call(
            "leviathan_break_compile_preview",
            json!({"expected_revision":6,"config":config})
        )["ok"],
        false
    );
    assert_eq!(c.document(), dense);
    // ID exhaustion is checked in the pure allocator before any generated state commits.
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![
                marker_edit(31, json!(70), vec![0, 151200]),
                json!({"op":"upsert_instrument","id":u64::MAX,"name":"Last ID","sample":null})
            ]
        )["ok"],
        true
    );
    let exhausted = c.document();
    assert_eq!(
        c.call(
            "leviathan_break_compile_preview",
            json!({"expected_revision":7,"config":break_config()})
        )["ok"],
        false
    );
    assert_eq!(c.document(), exhausted);
}

#[test]
fn compiler_grid_cycles_are_explicit_ordinary_notes_not_source_tempo_inference() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    compiler_fixture(&mut c, &f);
    let mut config = break_config();
    config["timing"] = json!({"Grid":{"division":"Sixteenth"}});
    config["cycles"] = json!(2);
    let preview = compile_preview(&mut c, 2, config);
    assert_eq!(preview["data"]["diff"]["source_bpm"], 70.0);
    assert_eq!(preview["data"]["diff"]["note_starts"], 8);
    assert_eq!(compile_commit(&mut c, 2, &preview)["ok"], true);
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let manifest: aqua_pattern_core::song_render::Manifest =
        serde_json::from_value(manifest["data"]["manifest"].clone()).unwrap();
    let starts: Vec<_> = manifest
        .events
        .iter()
        .filter_map(|e| {
            matches!(
                e.command,
                aqua_pattern_core::song_render::Command::On { .. }
            )
            .then_some(e.frame)
        })
        .collect();
    let expected: Vec<u64> = (0..8)
        .map(|i| (i as f64 / 4.0 * 48000.0 * 60.0 / 170.0).round() as u64)
        .collect();
    assert_eq!(starts, expected);
}

#[cfg(unix)]
#[test]
fn compiler_and_cleanup_paths_are_reauthorized_before_every_source_access() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = compiler_fixture(&mut c, &f);
    let preview = compile_preview(&mut c, 2, break_config());
    let path = f.root.join("break70.wav");
    let outside = f.base.join("same-bytes.wav");
    fs::copy(&path, &outside).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&outside, &path).unwrap();
    for (tool, args) in [
        (
            "leviathan_break_compile_preview",
            json!({"expected_revision":2,"config":break_config()}),
        ),
        (
            "leviathan_break_compile_commit",
            json!({"expected_revision":2,"preview_token":preview["data"]["preview_token"]}),
        ),
        (
            "leviathan_marker_cleanup_preview",
            json!({"expected_revision":2,"sample_id":7,"candidates":[0,151200],"minimum_slice_frames":1}),
        ),
    ] {
        assert_eq!(c.call(tool, args)["error"]["code"], "path_denied");
        assert_eq!(c.document(), before);
    }
    fs::remove_file(&path).unwrap();
    fs::copy(&outside, &path).unwrap();
    assert_eq!(compile_commit(&mut c, 2, &preview)["ok"], true);
}

#[test]
fn marker_lock_edits_preserve_old_upserts_are_atomic_and_match_editor_methods() {
    use aqua_pattern_core::{
        editor::Editor,
        song::{MarkerSetId, Song},
    };
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 1000, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![marker_edit(31, json!(70), vec![0, 200, 400, 1000])]
        )["ok"],
        true
    );
    let baseline: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(baseline).unwrap();
    editor.edit_mode = true;
    editor.set_marker_lock(MarkerSetId(31), 200, true).unwrap();
    editor.add_marker(MarkerSetId(31), 600).unwrap();
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"set_marker_lock","marker_set_id":31,"frame":200,"locked":true}),
                json!({"op":"add_marker","marker_set_id":31,"frame":600})
            ]
        )["ok"],
        true
    );
    let from_mcp: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    let mut renamed = marker_edit(31, json!(80), vec![0, 200, 400, 600, 1000]);
    renamed["name"] = json!("Renamed without losing locks");
    assert_eq!(edit(&mut c, 3, vec![renamed])["ok"], true);
    let before = c.document();
    assert_eq!(
        before["data"]["song"]["marker_sets"]["31"]["locked_frames"],
        json!([200])
    );
    for op in [
        json!({"op":"move_marker","marker_set_id":31,"from_frame":200,"to_frame":250}),
        json!({"op":"remove_marker","marker_set_id":31,"frame":200}),
        json!({"op":"remove_marker_set","id":31}),
        marker_edit(31, json!(70), vec![0, 400, 600, 1000]),
        json!({"op":"set_marker_lock","marker_set_id":31,"frame":0,"locked":false}),
        json!({"op":"remove_marker","marker_set_id":31,"frame":1000}),
        json!({"op":"add_marker","marker_set_id":31,"frame":200}),
        json!({"op":"move_marker","marker_set_id":31,"from_frame":400,"to_frame":600}),
    ] {
        let failed = edit(&mut c, 4, vec![title("must rollback"), op]);
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
    }
    let mut bypass = marker_edit(31, json!(70), vec![0, 400, 600, 1000]);
    bypass["locked_frames"] = json!([]);
    assert_eq!(
        edit(&mut c, 4, vec![bypass])["error"]["code"],
        "invalid_arguments"
    );
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    editor.replace_document(base).unwrap();
    editor.edit_mode = true;
    editor.set_marker_lock(MarkerSetId(31), 200, false).unwrap();
    editor.move_marker(MarkerSetId(31), 200, 250).unwrap();
    editor.set_marker_lock(MarkerSetId(31), 250, true).unwrap();
    editor.remove_marker(MarkerSetId(31), 400).unwrap();
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"set_marker_lock","marker_set_id":31,"frame":200,"locked":false}),
                json!({"op":"move_marker","marker_set_id":31,"from_frame":200,"to_frame":250}),
                json!({"op":"set_marker_lock","marker_set_id":31,"frame":250,"locked":true}),
                json!({"op":"remove_marker","marker_set_id":31,"frame":400})
            ]
        )["ok"],
        true
    );
    let from_mcp: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"locked.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"locked.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

fn onset_prepare(c: &mut Client, revision: u64, preset: &str, picker: Value) -> Value {
    c.ok("leviathan_onset_v2_prepare",json!({"expected_revision":revision,"sample_id":7,"engine":"experimental_v2","preset":preset,"picker":picker}))
}
fn onset_page(
    c: &mut Client,
    revision: u64,
    prepared: &Value,
    start: usize,
    limit: usize,
) -> Value {
    c.ok("leviathan_onset_v2_evidence",json!({"expected_revision":revision,"cache_token":prepared["data"]["cache_token"],"start":start,"limit":limit}))
}

fn onset_full_result(c: &mut Client, revision: u64, prepared: &Value) -> Value {
    let mut sets = Vec::new();
    for kind in ["candidates", "suggestions"] {
        let mut items = Vec::new();
        let mut start = 0;
        loop {
            let page = c.ok("leviathan_onset_v2_results", json!({"expected_revision":revision,"cache_token":prepared["data"]["cache_token"],"kind":kind,"start":start,"limit":64}));
            assert_eq!(
                page["data"]["config_sha256"],
                prepared["data"]["source_analysis_summary"]["config_sha256"]
            );
            items.extend(page["data"]["items"].as_array().unwrap().clone());
            if let Some(next) = page["data"]["next_start"].as_u64() {
                start = next;
            } else {
                break;
            }
        }
        sets.push(Value::Array(items));
    }
    let summary = &prepared["data"]["source_analysis_summary"];
    json!({"binding":summary["binding"],"config_sha256":summary["config_sha256"],"analysis":{"detection":{"provenance":summary["provenance"],"candidates":sets[0],"envelope":[]},"suggestions":sets[1],"refinement_collisions":summary["refinement_collisions"]}})
}

#[test]
fn experimental_onset_cache_and_repick_equal_shared_service_and_keep_v1_and_locked_markers_unchanged(
) {
    use aqua_pattern_core::{onset_service, onset_v2, song::SampleAsset};
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 2);
    let bytes = fs::read(f.root.join("source.wav")).unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                marker_edit(31, json!(70), vec![0, 6000, 24000, 48000]),
                json!({"op":"set_marker_lock","marker_set_id":31,"frame":6000,"locked":true})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let v1 = c.ok(
        "leviathan_sample_analyze",
        json!({"expected_revision":2,"sample_id":7,"config":{"waveform_bins":16}}),
    );
    let prepared = onset_prepare(&mut c, 2, "Balanced", json!({}));
    assert_eq!(prepared["data"]["engine"], "experimental_v2");
    assert_eq!(prepared["data"]["policy"], onset_service::POLICY);
    assert_eq!(prepared["data"]["effective_config"]["enable_phase"], false);
    assert_eq!(prepared["data"]["phase_enabled"], false);
    assert_eq!(
        prepared["data"]["effective_config"]["max_suggestions"],
        4096
    );
    assert_eq!(prepared["data"]["fft_recomputed"], true);
    assert_eq!(
        prepared["data"]["source_analysis_summary"]["binding"]["source_frames"],
        48000
    );
    assert_eq!(
        prepared["data"]["source_analysis_summary"]["binding"]["algorithm"],
        onset_v2::ALGORITHM
    );
    let config: onset_v2::Config =
        serde_json::from_value(prepared["data"]["effective_config"].clone()).unwrap();
    let asset: SampleAsset =
        serde_json::from_value(before["data"]["song"]["samples"]["7"].clone()).unwrap();
    let shared = onset_service::prepare_verified(&asset, &asset.path, &config).unwrap();
    assert_eq!(
        onset_full_result(&mut c, 2, &prepared),
        serde_json::to_value(&shared.result).unwrap(),
        "MCP and UI shared service must interpret the exact effective config identically"
    );
    assert!(!shared.result.analysis.suggestions.is_empty());
    for suggestion in &shared.result.analysis.suggestions {
        assert_eq!(
            suggestion.refinement_shift,
            suggestion.attack_frame as i64 - suggestion.candidate.peak_frame as i64
        );
        assert!(suggestion.attack_frame < asset.frames);
        assert!(suggestion.candidate.peak_frame < asset.frames);
    }
    let page = onset_page(&mut c, 2, &prepared, 0, 17);
    assert_eq!(
        page["data"]["page"],
        serde_json::to_value(shared.cache.curve_page(0, 17).unwrap()).unwrap()
    );
    assert_eq!(page["data"]["next_start"], 17);
    let last_start = shared.cache.raw_curves().len().saturating_sub(5);
    let last = onset_page(&mut c, 2, &prepared, last_start, 256);
    assert!(last["data"]["next_start"].is_null());
    assert!(last["data"]["page"]["points"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["source_frame"].as_u64().unwrap() < 48000));
    let picker = json!({"threshold":0.2,"mad_multiplier":2.0,"min_gap_ms":16,"dense_gap_ms":5,"enable_phase":true,"refinement":{"enabled":true}});
    let repicked=c.ok("leviathan_onset_v2_repick",json!({"expected_revision":2,"cache_token":prepared["data"]["cache_token"],"preset":"GhostHunter","picker":picker}));
    assert_eq!(repicked["data"]["fft_recomputed"], false);
    assert_eq!(repicked["data"]["phase_enabled"], true);
    assert_ne!(
        repicked["data"]["cache_token"],
        prepared["data"]["cache_token"]
    );
    assert_eq!(
        repicked["data"]["source_analysis_summary"]["binding"]["extraction_key"],
        prepared["data"]["source_analysis_summary"]["binding"]["extraction_key"]
    );
    assert_ne!(
        repicked["data"]["source_analysis_summary"]["config_sha256"],
        prepared["data"]["source_analysis_summary"]["config_sha256"]
    );
    let config: onset_v2::Config =
        serde_json::from_value(repicked["data"]["effective_config"].clone()).unwrap();
    let reused = onset_service::repick(&shared.cache, &asset, &config).unwrap();
    let fresh = onset_service::prepare_verified(&asset, &asset.path, &config).unwrap();
    assert_eq!(
        serde_json::to_value(&reused).unwrap(),
        serde_json::to_value(&fresh.result).unwrap()
    );
    assert_eq!(
        onset_full_result(&mut c, 2, &repicked),
        serde_json::to_value(reused).unwrap()
    );
    assert_eq!(
        c.call(
            "leviathan_onset_v2_evidence",
            json!({"expected_revision":2,"cache_token":prepared["data"]["cache_token"]})
        )["error"]["code"],
        "invalid_onset_cache_token"
    );
    let reset=c.ok("leviathan_onset_v2_repick",json!({"expected_revision":2,"cache_token":repicked["data"]["cache_token"],"preset":"Conservative","picker":{}}));
    assert_eq!(reset["data"]["phase_enabled"], false);
    assert_eq!(reset["data"]["effective_config"]["threshold"], 0.62);
    assert_eq!(
        c.ok(
            "leviathan_sample_analyze",
            json!({"expected_revision":2,"sample_id":7,"config":{"waveform_bins":16}})
        ),
        v1
    );
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(f.root.join("source.wav")).unwrap(), bytes);
}

#[test]
fn experimental_cache_tokens_limits_invalid_pickers_and_prepare_failures_have_explicit_semantics() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 1);
    write_onset_source(&f.root.join("long.wav"), 8000, 8000 * 16, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    let prepared = onset_prepare(&mut c, 1, "Balanced", json!({}));
    for picker in [
        json!({"threshold":1}),
        json!({"dense_gap_ms":1001}),
        json!({"median_radius_ms":19}),
        json!({"refinement":{"attack_fraction":1}}),
        json!({"minimum_rms":0.5}),
        json!({"detection":{}}),
        json!({"enable_flux":false}),
    ] {
        let failed=c.call("leviathan_onset_v2_repick",json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"picker":picker}));
        assert_eq!(failed["ok"], false, "{failed}");
        onset_page(&mut c, 1, &prepared, 0, 1);
        assert_eq!(c.document(), before);
    }
    for (start, limit) in [(0, 0), (0, 257), (999999, 1)] {
        assert_eq!(c.call("leviathan_onset_v2_evidence",json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"start":start,"limit":limit}))["ok"],false);
    }
    assert_eq!(
        c.call(
            "leviathan_onset_v2_prepare",
            json!({"expected_revision":1,"sample_id":7,"engine":"v1","preset":"Balanced"})
        )["error"]["code"],
        "invalid_arguments"
    );
    onset_page(&mut c, 1, &prepared, 0, 1);
    // Once a new verified preparation starts, its old retained cache is retired.
    let failed=c.call("leviathan_onset_v2_prepare",json!({"expected_revision":1,"sample_id":7,"engine":"experimental_v2","preset":"Balanced","picker":{"threshold":1}}));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["previous_cache_retired"], true);
    assert_eq!(
        c.call(
            "leviathan_onset_v2_evidence",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"]})
        )["error"]["code"],
        "onset_cache_unavailable"
    );
    assert_eq!(c.document(), before);
    let first = onset_prepare(&mut c, 1, "Balanced", json!({}));
    let latest = onset_prepare(
        &mut c,
        1,
        "GhostHunter",
        json!({"refinement":{"enabled":false}}),
    );
    assert_eq!(
        c.call(
            "leviathan_onset_v2_evidence",
            json!({"expected_revision":1,"cache_token":first["data"]["cache_token"]})
        )["error"]["code"],
        "invalid_onset_cache_token"
    );
    for suggestion in onset_full_result(&mut c, 1, &latest)["analysis"]["suggestions"]
        .as_array()
        .unwrap()
    {
        assert_eq!(
            suggestion["candidate"]["peak_frame"],
            suggestion["attack_frame"]
        );
    }
    assert_eq!(edit(&mut c, 1, vec![title("temporary")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    assert_eq!(
        c.call(
            "leviathan_onset_v2_evidence",
            json!({"expected_revision":3,"cache_token":latest["data"]["cache_token"]})
        )["error"]["code"],
        "onset_cache_unavailable"
    );
    import_source(&mut c, 3, 8, "long.wav");
    let long_before = c.document();
    assert_eq!(c.call("leviathan_onset_v2_prepare",json!({"expected_revision":4,"sample_id":8,"engine":"experimental_v2","preset":"Balanced"}))["error"]["code"],"analysis_limit");
    assert_eq!(c.document(), long_before);
    // v1's existing60second limit and result remain available without v2 opt-in.
    c.ok(
        "leviathan_sample_analyze",
        json!({"expected_revision":4,"sample_id":8,"config":{"waveform_bins":0}}),
    );
}

#[test]
fn experimental_cached_evidence_and_repick_rehash_source_without_mutating_or_silently_reloading() {
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    let prepared = onset_prepare(&mut c, 1, "Balanced", json!({}));
    let path = f.root.join("source.wav");
    let original = fs::read(&path).unwrap();
    let mut tampered = original.clone();
    *tampered.last_mut().unwrap() ^= 1;
    fs::write(&path, tampered).unwrap();
    for (tool, args) in [
        (
            "leviathan_onset_v2_evidence",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"]}),
        ),
        (
            "leviathan_onset_v2_repick",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"picker":{}}),
        ),
        (
            "leviathan_onset_v2_results",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"kind":"suggestions"}),
        ),
    ] {
        let failed = c.call(tool, args);
        assert_eq!(failed["ok"], false);
        assert!(failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("SHA-256"));
        assert_eq!(c.document(), before);
    }
    fs::write(&path, &original).unwrap();
    onset_page(&mut c, 1, &prepared, 0, 3);
    let reused = c.ok(
        "leviathan_onset_v2_repick",
        json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"picker":{}}),
    );
    assert_eq!(reused["data"]["fft_recomputed"], false);
    assert_eq!(
        reused["data"]["source_analysis_summary"],
        prepared["data"]["source_analysis_summary"]
    );
    assert_eq!(c.document(), before);
}

#[cfg(unix)]
#[test]
fn experimental_source_cache_tools_reauthorize_paths_even_when_symlink_bytes_match() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    write_onset_source(&f.root.join("source.wav"), 48000, 48000, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    let prepared = onset_prepare(&mut c, 1, "Balanced", json!({}));
    let path = f.root.join("source.wav");
    let outside = f.base.join("outside.wav");
    fs::copy(&path, &outside).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&outside, &path).unwrap();
    for (tool, args) in [
        (
            "leviathan_onset_v2_prepare",
            json!({"expected_revision":1,"sample_id":7,"engine":"experimental_v2","preset":"Balanced"}),
        ),
        (
            "leviathan_onset_v2_evidence",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"]}),
        ),
        (
            "leviathan_onset_v2_repick",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"picker":{}}),
        ),
        (
            "leviathan_onset_v2_results",
            json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"kind":"suggestions"}),
        ),
    ] {
        assert_eq!(c.call(tool, args)["error"]["code"], "path_denied");
        assert_eq!(c.document(), before);
    }
    fs::remove_file(&path).unwrap();
    fs::copy(&outside, &path).unwrap();
    onset_page(&mut c, 1, &prepared, 0, 1);
}

#[test]
fn experimental_pages_preserve_collision_lists_exact_totals_and_empty_end_semantics() {
    use aqua_pattern_core::{onset_service, onset_v2, song::SampleAsset};
    let f = Fixture::new();
    let rate = 96000u32;
    let mut pcm = vec![0.0f32; rate as usize * 2];
    for (start, hz) in [
        (0.12, 70.0),
        (0.45, 600.0),
        (0.81, 6000.0),
        (1.22, 170.0),
        (1.67, 3200.0),
    ] {
        let frame = (start * f64::from(rate)).round() as usize;
        for (i, sample) in pcm
            .iter_mut()
            .skip(frame)
            .take((f64::from(rate) * 0.12) as usize)
            .enumerate()
        {
            let t = i as f64 / f64::from(rate);
            let attack = (t / 0.00075).min(1.0);
            *sample +=
                (0.4 * attack * (-t / 0.015).exp() * (t * hz * std::f64::consts::TAU).cos()) as f32;
        }
    }
    let path = f.root.join("source.wav");
    let mut wav = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for sample in pcm {
        wav.write_sample(sample).unwrap();
    }
    wav.finalize().unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    let prepared = onset_prepare(&mut c, 1, "GhostHunter", json!({"enable_phase":true}));
    let summary = &prepared["data"]["source_analysis_summary"];
    assert!(summary["refinement_collisions"].as_u64().unwrap()>0,"Expected the judged96k sparse fixture to exercise distinct candidate/suggestion list lengths");
    assert!(
        summary["candidate_count"].as_u64().unwrap()
            > summary["suggestion_count"].as_u64().unwrap()
    );
    let asset: SampleAsset =
        serde_json::from_value(before["data"]["song"]["samples"]["7"].clone()).unwrap();
    let config: onset_v2::Config =
        serde_json::from_value(prepared["data"]["effective_config"].clone()).unwrap();
    let shared = onset_service::prepare_verified(&asset, &path, &config).unwrap();
    for kind in ["candidates", "suggestions"] {
        let mut concatenated = Vec::new();
        let mut start = 0;
        loop {
            let page=c.ok("leviathan_onset_v2_results",json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"kind":kind,"start":start,"limit":1}));
            assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
            assert_eq!(page["data"]["config_sha256"], summary["config_sha256"]);
            concatenated.extend(page["data"]["items"].as_array().unwrap().clone());
            if let Some(next) = page["data"]["next_start"].as_u64() {
                start = next;
            } else {
                break;
            }
        }
        let expected = if kind == "candidates" {
            serde_json::to_value(&shared.result.analysis.detection.candidates).unwrap()
        } else {
            serde_json::to_value(&shared.result.analysis.suggestions).unwrap()
        };
        assert_eq!(Value::Array(concatenated.clone()), expected);
        let end=c.ok("leviathan_onset_v2_results",json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"kind":kind,"start":concatenated.len(),"limit":64}));
        assert_eq!(end["data"]["items"], json!([]));
        assert!(end["data"]["next_start"].is_null());
        for (start, limit) in [(concatenated.len() + 1, 1), (0, 0), (0, 65)] {
            assert_eq!(c.call("leviathan_onset_v2_results",json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"kind":kind,"start":start,"limit":limit}))["ok"],false);
        }
    }
    assert_eq!(
        onset_full_result(&mut c, 1, &prepared),
        serde_json::to_value(&shared.result).unwrap()
    );
    assert_eq!(
        prepared["data"]["serialized_result_bytes"],
        serde_json::to_vec(&shared.result).unwrap().len()
    );
    let mut points = Vec::new();
    let mut start = 0;
    loop {
        let page = onset_page(&mut c, 1, &prepared, start, 256);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        points.extend(page["data"]["page"]["points"].as_array().unwrap().clone());
        if let Some(next) = page["data"]["next_start"].as_u64() {
            start = next as usize;
        } else {
            break;
        }
    }
    let all = shared.cache.curve_page(0, 2048).unwrap();
    assert_eq!(
        Value::Array(points),
        serde_json::to_value(all).unwrap()["points"]
    );
    assert_eq!(c.document(), before);
}

#[test]
fn experimental_capacity_counts_source_start_once_and_never_merges_manual_markers_implicitly() {
    let f = Fixture::new();
    let mut wav = hound::WavWriter::create(
        f.root.join("source.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..64 {
        wav.write_sample(if i == 0 { 0.8f32 } else { 0.0 }).unwrap();
    }
    wav.finalize().unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![marker_edit(31, json!(70), vec![0, 16, 32, 48, 64])]
        )["ok"],
        true
    );
    let before = c.document();
    let prepared = onset_prepare(&mut c, 2, "Balanced", json!({}));
    let full = onset_full_result(&mut c, 2, &prepared);
    let suggestions = full["analysis"]["suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0]["attack_frame"], 0);
    assert_eq!(
        prepared["data"]["marker_capacity"]["proposed_full_source_boundary_count"],
        2
    );
    assert_eq!(
        prepared["data"]["marker_capacity"]["fits_one_marker_set"],
        true
    );
    assert!(prepared["data"]["marker_capacity"]["basis"]
        .as_str()
        .unwrap()
        .contains("not a union"));
    assert_eq!(c.document(), before);
}

#[test]
fn experimental_dense_results_remain_inspectable_when_full_marker_acceptance_exceeds_capacity() {
    let f = Fixture::new();
    let mut wav = hound::WavWriter::create(
        f.root.join("source.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for frame in 0..144000 {
        wav.write_sample(if frame % 256 == 128 { 26214i16 } else { 0 })
            .unwrap();
    }
    wav.finalize().unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "source.wav");
    let before = c.document();
    let prepared = onset_prepare(
        &mut c,
        1,
        "GhostHunter",
        json!({"threshold":0.1,"mad_multiplier":0.5,"min_gap_ms":1,"dense_gap_ms":1,"refinement":{"enabled":false}}),
    );
    let count = prepared["data"]["source_analysis_summary"]["suggestion_count"]
        .as_u64()
        .unwrap();
    assert!(count > 255);
    assert_eq!(
        prepared["data"]["marker_capacity"]["fits_one_marker_set"],
        false
    );
    let mut boundaries = std::collections::BTreeSet::from([0, 144000]);
    let mut start = 0;
    let mut inspected = 0;
    loop {
        let page=c.ok("leviathan_onset_v2_results",json!({"expected_revision":1,"cache_token":prepared["data"]["cache_token"],"kind":"suggestions","start":start,"limit":64}));
        assert_eq!(page["data"]["total"], count);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        let items = page["data"]["items"].as_array().unwrap();
        inspected += items.len();
        boundaries.extend(items.iter().map(|s| s["attack_frame"].as_u64().unwrap()));
        if let Some(next) = page["data"]["next_start"].as_u64() {
            start = next;
        } else {
            break;
        }
    }
    assert_eq!(inspected as u64, count);
    assert_eq!(
        prepared["data"]["marker_capacity"]["proposed_full_source_boundary_count"],
        boundaries.len()
    );
    let failed = edit(
        &mut c,
        1,
        vec![marker_edit(31, json!(70), boundaries.into_iter().collect())],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["atomic"], true);
    assert_eq!(c.document(), before);
    onset_page(&mut c, 1, &prepared, 0, 1);
}

fn kit_metadata(id: u64, name: &str) -> Value {
    json!({"op":"upsert_drum_kit","id":id,"name":name})
}
fn kit_pad(id: u64, slot: u8, note: u8) -> Value {
    json!({"op":"upsert_drum_pad","kit_id":41,"id":id,"slot":slot,"name":format!("Pad {id}"),"note":note,"region_id":21,"root_note":60,"gain":1.0,"pan":0.0,"mode":"OneShot","new_note":"Overlap","choke_group":null})
}
fn kit_cell(track: u64, row: u32, note: Value, instrument: u64) -> Value {
    json!({"op":"set_cell","pattern_id":1,"track_id":track,"row":row,"column":0,"cell":{"note":note,"instrument":instrument}})
}
fn kit_source(f: &Fixture) {
    let mut writer = hound::WavWriter::create(
        f.root.join("kit.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..48000 {
        writer.write_sample(0.2f32).unwrap();
    }
    writer.finalize().unwrap();
}
fn kit_setup(c: &mut Client, f: &Fixture) -> Value {
    kit_source(f);
    import_source(c, 0, 7, "kit.wav");
    let before = c.document();
    let result = edit(
        c,
        1,
        vec![
            region_edit(21, 0, 48000),
            kit_metadata(41, "Drums"),
            kit_pad(301, 0, 36),
            kit_pad(302, 15, 38),
            json!({"op":"assign_kit","instrument_id":1,"kit_id":41}),
            json!({"op":"set_song","bpm":120,"lines_per_beat":4}),
            json!({"op":"upsert_pattern","id":1,"name":"Kit hits","rows":4}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["revision"], 2);
    before
}
fn kit_export(c: &mut Client, f: &Fixture, revision: u64, filename: &str) -> Vec<f32> {
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":revision,"path":filename}),
    );
    hound::WavReader::open(f.root.join(filename))
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn drum_kit_typed_crud_and_note_insertion_have_exact_editor_and_mcp_parity() {
    use aqua_pattern_core::{
        editor::Editor,
        sample_asset,
        session::{Action, Session},
        song::*,
        song_render,
    };
    let f = Fixture::new();
    kit_source(&f);
    let mut c = Client::ready(&f);
    let blank = c.document();
    assert!(blank["data"]["song"].get("drum_kits").is_none());
    for instrument in blank["data"]["song"]["instruments"]
        .as_object()
        .unwrap()
        .values()
    {
        assert!(instrument.get("kind").is_none());
    }
    let mut editor = Editor::default();
    Session::default()
        .request(&mut editor, Action::New)
        .unwrap();
    editor.edit_mode = true;
    editor
        .register_sample(
            SampleId(7),
            sample_asset::import(&f.root.join("kit.wav")).unwrap().asset,
        )
        .unwrap();
    editor
        .upsert_region(
            RegionId(21),
            SampleRegion {
                name: "Slice 21".into(),
                sample: SampleId(7),
                start_frame: 0,
                end_frame: 48000,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        )
        .unwrap();
    editor
        .upsert_drum_kit(
            DrumKitId(41),
            DrumKitInstrument {
                name: "Drums".into(),
                pads: Default::default(),
            },
        )
        .unwrap();
    let pad = DrumPad {
        slot: 0,
        name: "Pad 301".into(),
        note: 36,
        region: Some(RegionId(21)),
        velocity_zones: Vec::new(),
        root_note: 60,
        tune_semitones: 0,
        gain: 1.0,
        pan: 0.0,
        mode: PadMode::OneShot,
        new_note: PadNewNote::Overlap,
        choke_group: None,
    };
    editor
        .upsert_drum_pad(DrumKitId(41), PadId(301), pad.clone())
        .unwrap();
    editor
        .assign_kit(InstrumentId(1), Some(DrumKitId(41)))
        .unwrap();
    editor.select(0, TrackId(1)).unwrap();
    editor.insert_pad_note(InstrumentId(1), PadId(301)).unwrap();
    import_source(&mut c, 0, 7, "kit.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 0, 48000),
                kit_metadata(41, "Drums"),
                kit_pad(301, 0, 36),
                json!({"op":"assign_kit","instrument_id":1,"kit_id":41}),
                kit_cell(1, 0, json!({"Note":36}), 1),
            ]
        )["ok"],
        true
    );
    let document = c.document();
    let from_mcp: Song = serde_json::from_value(document["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    let from_mcp: song_render::Manifest =
        serde_json::from_value(evaluated["data"]["manifest"].clone()).unwrap();
    assert_eq!(from_mcp, song_render::evaluate(&editor.song, None).unwrap());
    assert_eq!(document["data"]["session"]["drum_kit_count"], 1);
    assert_eq!(document["data"]["session"]["drum_pad_count"], 1);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"drum_kits"}))["data"]["drum_kits"],
        document["data"]["song"]["drum_kits"]
    );
    let mut renamed = editor.song.drum_kits[&DrumKitId(41)].clone();
    renamed.name = "Renamed kit".into();
    editor.upsert_drum_kit(DrumKitId(41), renamed).unwrap();
    assert_eq!(
        edit(&mut c, 2, vec![kit_metadata(41, "Renamed kit")])["ok"],
        true
    );
    let renamed: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(renamed, editor.song);
    assert_eq!(renamed.drum_kits[&DrumKitId(41)].pads[&PadId(301)], pad);
    editor.row = 0; // insert_pad_note advances the shared edit cursor.
    editor.set_cell(Cell::default()).unwrap();
    editor.assign_kit(InstrumentId(1), None).unwrap();
    editor.remove_drum_pad(DrumKitId(41), PadId(301)).unwrap();
    editor.remove_drum_kit(DrumKitId(41)).unwrap();
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"clear_cell","pattern_id":1,"track_id":1,"row":0,"column":0}),
                json!({"op":"assign_kit","instrument_id":1,"kit_id":null}),
                json!({"op":"remove_drum_pad","kit_id":41,"id":301}),
                json!({"op":"remove_drum_kit","id":41}),
            ]
        )["ok"],
        true
    );
    let removed: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(removed, editor.song);
    assert!(c.document()["data"]["song"].get("drum_kits").is_none());
}

#[test]
fn drum_kit_mapping_schema_limits_and_references_reject_without_partial_edits() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 2, vec![kit_cell(1, 0, json!({"Note":36}), 1)])["ok"],
        true
    );
    let before = c.document();
    let mut invalid = vec![
        kit_metadata(0, "zero"),
        kit_metadata(42, &"x".repeat(257)),
        json!({"op":"remove_drum_kit","id":41}),
        json!({"op":"remove_drum_pad","kit_id":41,"id":301}),
        json!({"op":"remove_region","id":21}),
        json!({"op":"assign_kit","instrument_id":1,"kit_id":999}),
        kit_cell(1, 1, json!({"Note":37}), 1),
        kit_pad(303, 0, 40),
        kit_pad(303, 1, 36),
    ];
    for (key, value) in [
        ("slot", json!(16)),
        ("note", json!(128)),
        ("region_id", json!(999)),
        ("root_note", json!(128)),
        ("tune_semitones", json!(49)),
        ("tune_semitones", json!(-49)),
        ("gain", json!(1.1)),
        ("gain", json!(1.0000000001)),
        ("gain", json!(-1e-50)),
        ("pan", json!(-1.1)),
        ("pan", json!(1.0000000001)),
        ("choke_group", json!(0)),
        ("choke_group", json!(17)),
        ("mode", json!("Loop")),
        ("new_note", json!("CutPrevious")),
        ("name", json!("x".repeat(129))),
        ("layers", json!([])),
    ] {
        let mut operation = kit_pad(303, 1, 40);
        operation[key] = value;
        invalid.push(operation);
    }
    let mut unmap = kit_pad(301, 0, 36);
    unmap["region_id"] = Value::Null;
    invalid.push(unmap);
    for operation in invalid {
        let result = edit(&mut c, 3, vec![title("must roll back"), operation]);
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(result["revision"], 3);
        assert_eq!(c.document(), before);
    }
    for required in ["region_id", "choke_group"] {
        let mut operation = kit_pad(303, 1, 40);
        operation.as_object_mut().unwrap().remove(required);
        assert_eq!(
            edit(&mut c, 3, vec![operation])["error"]["code"],
            "invalid_arguments"
        );
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![json!({"op":"assign_kit","instrument_id":1})]
        )["error"]["code"],
        "invalid_arguments"
    );
    // Fully populated bank uses all16 distinct slots; a17th pad rejects atomically.
    let more: Vec<_> = (1..15)
        .map(|slot| kit_pad(400 + u64::from(slot), slot, 40 + slot))
        .collect();
    assert_eq!(edit(&mut c, 3, more)["ok"], true);
    let full = c.document();
    assert_eq!(full["data"]["session"]["drum_pad_count"], 16);
    assert_eq!(edit(&mut c, 4, vec![kit_pad(999, 1, 90)])["ok"], false);
    assert_eq!(c.document(), full);
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    // A failed edit did not erase the redo entry.
    assert_eq!(edit(&mut c, 5, vec![kit_pad(999, 16, 90)])["ok"], false);
    assert_eq!(c.document()["data"]["session"]["can_redo"], true);
}

#[test]
fn drum_kit_exports_choke_zero_gain_hits_across_tracks_but_not_other_instruments_and_roundtrip() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let imported = kit_setup(&mut c, &f);
    let setup = c.document();
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], imported["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], setup["data"]["song"]);
    let source = fs::read(f.root.join("kit.wav")).unwrap();
    let mut open = kit_pad(301, 0, 36);
    open["choke_group"] = json!(1);
    let mut closed = kit_pad(302, 15, 38);
    closed["choke_group"] = json!(1);
    closed["gain"] = json!(0.0);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                open,
                closed.clone(),
                json!({"op":"assign_kit","instrument_id":2,"kit_id":41}),
                kit_cell(1, 0, json!({"Note":36}), 1),
                kit_cell(2, 0, json!({"Note":36}), 2),
                // Different track, same InstrumentId: group choke still applies.
                kit_cell(3, 1, json!({"Note":38}), 1),
            ]
        )["ok"],
        true
    );
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":5}));
    assert_eq!(evaluated["data"]["source_identity_verified"], false);
    let choked = kit_export(&mut c, &f, 5, "choked.wav");
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"kit.leviathan.json"}),
    );
    let saved = c.document()["data"]["song"].clone();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":6,"path":"kit.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], saved);
    assert_eq!(kit_export(&mut c, &f, 7, "reopened.wav"), choked);
    closed["choke_group"] = Value::Null;
    assert_eq!(edit(&mut c, 7, vec![closed])["ok"], true);
    let overlap = kit_export(&mut c, &f, 8, "overlap.wav");
    assert_eq!(&choked[..6000 * 2], &overlap[..6000 * 2]);
    assert!(overlap[18000 * 2] > 0.1);
    assert!(
        (choked[18000 * 2] * 2.0 - overlap[18000 * 2]).abs() < 0.00001,
        "other instrument must survive same-kit group choke: {} vs {}",
        choked[18000 * 2],
        overlap[18000 * 2]
    );
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), source);
    assert_eq!(c.document()["revision"], 8); // read/evaluate/export did not mutate
}

#[test]
fn drum_kit_one_shot_gated_and_same_pad_new_note_policies_use_ordinary_events() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                kit_cell(1, 0, json!({"Note":36}), 1),
                kit_cell(1, 1, json!("Off"), 1),
            ]
        )["ok"],
        true
    );
    let one_shot = kit_export(&mut c, &f, 3, "one-shot.wav");
    let mut pad = kit_pad(301, 0, 36);
    pad["mode"] = json!("Gated");
    assert_eq!(edit(&mut c, 3, vec![pad.clone()])["ok"], true);
    let gated = kit_export(&mut c, &f, 4, "gated.wav");
    assert_eq!(&one_shot[..6000 * 2], &gated[..6000 * 2]);
    assert!(one_shot[18000 * 2] > 0.01);
    assert!(gated[18000 * 2].abs() < 0.00001);
    pad["mode"] = json!("OneShot");
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![pad.clone(), kit_cell(1, 1, json!({"Note":36}), 1)]
        )["ok"],
        true
    );
    let overlap = kit_export(&mut c, &f, 5, "same-pad-overlap.wav");
    pad["new_note"] = json!("ReleasePrevious");
    assert_eq!(edit(&mut c, 5, vec![pad])["ok"], true);
    let replaced = kit_export(&mut c, &f, 6, "same-pad-release.wav");
    assert!((replaced[18000 * 2] * 2.0 - overlap[18000 * 2]).abs() < 0.00001);
    assert!(overlap[18000 * 2] > one_shot[18000 * 2] * 1.9);
}

#[test]
fn drum_kit_unused_definitions_source_switches_and_unordered_references_preserve_semantics() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_source(&f);
    import_source(&mut c, 0, 7, "kit.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 0, 48000),
                kit_metadata(41, "Drums"),
                kit_pad(301, 0, 36),
                json!({"op":"upsert_instrument","id":4,"name":"Instrument 4","sample":"kit.wav"}),
                kit_cell(4, 0, json!({"Note":60}), 4),
            ]
        )["ok"],
        true
    );
    let mut plain: aqua_pattern_core::song::Song =
        serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    plain.drum_kits.clear();
    let plain_manifest =
        serde_json::to_value(aqua_pattern_core::song_render::evaluate(&plain, None).unwrap())
            .unwrap();
    assert_eq!(
        c.ok("leviathan_song_evaluate", json!({"expected_revision":2}))["data"]["manifest"],
        plain_manifest
    );
    let kit = c.document()["data"]["song"]["drum_kits"].clone();
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![json!({"op":"assign_kit","instrument_id":1,"kit_id":41})]
        )["ok"],
        true
    );
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_instrument","id":1,"name":"Renamed instrument","sample":null})
            ]
        )["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["song"]["instruments"]["1"]["kind"],
        json!({"type":"DrumKit","data":{"kit":41}})
    );
    assert_eq!(c.document()["data"]["song"]["drum_kits"], kit);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"assign_region","instrument_id":1,"region_id":21})]
        )["ok"],
        true
    );
    assert!(c.document()["data"]["song"]["instruments"]["1"]
        .get("kind")
        .is_none());
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![json!({"op":"assign_kit","instrument_id":1,"kit_id":41})]
        )["ok"],
        true
    );
    assert!(c.document()["data"]["song"]["instruments"]["1"]
        .get("region")
        .is_none());
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![json!({"op":"upsert_instrument","id":1,"name":"Legacy","sample":"kit.wav"})]
        )["ok"],
        true
    );
    assert!(c.document()["data"]["song"]["instruments"]["1"]
        .get("kind")
        .is_none());
    assert_eq!(c.document()["data"]["song"]["drum_kits"], kit);
    assert_eq!(
        edit(
            &mut c,
            7,
            vec![
                json!({"op":"assign_kit","instrument_id":1,"kit_id":41}),
                json!({"op":"upsert_pattern","id":2,"name":"Unordered","rows":4}),
                json!({"op":"set_cell","pattern_id":2,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":36},"instrument":1}}),
            ]
        )["ok"],
        true
    );
    let before = c.document();
    for operation in [
        kit_pad(301, 0, 37),
        json!({"op":"remove_drum_pad","kit_id":41,"id":301}),
    ] {
        assert_eq!(edit(&mut c, 8, vec![operation])["ok"], false);
        assert_eq!(c.document(), before);
    }
    // Neutral-transpose phrase can address selectors; transposed kit phrase cannot.
    assert_eq!(
        edit(
            &mut c,
            8,
            vec![
                phrase_metadata(51),
                json!({"op":"set_phrase_cell","phrase_id":51,"row":0,"column":0,"cell":{"note":{"Note":36}}}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"phrase":{"phrase":51,"cycles":1}}}),
            ]
        )["ok"],
        true
    );
    c.ok("leviathan_song_evaluate", json!({"expected_revision":9}));
    let neutral = c.document();
    assert_eq!(
        edit(
            &mut c,
            9,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":61},"instrument":1,"phrase":{"phrase":51,"cycles":1}}})
            ]
        )["ok"],
        false
    );
    assert_eq!(c.document(), neutral);
    let empty_kits: Vec<_> = (1000..1127).map(|id| kit_metadata(id, "Empty")).collect();
    assert_eq!(edit(&mut c, 9, empty_kits)["ok"], true);
    let full = c.document();
    assert_eq!(full["data"]["session"]["drum_kit_count"], 128);
    assert_eq!(
        edit(&mut c, 10, vec![kit_metadata(2000, "One too many")])["ok"],
        false
    );
    assert_eq!(c.document(), full);
}

#[test]
fn drum_pad_selector_is_not_pitch_and_explicit_tuning_uses_native_reference() {
    let f = Fixture::new();
    let mut writer = hound::WavWriter::create(
        f.root.join("kit.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..48000 {
        writer
            .write_sample(0.1f32 + frame as f32 / 100000.0)
            .unwrap();
    }
    writer.finalize().unwrap();
    let original = fs::read(f.root.join("kit.wav")).unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "kit.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 0, 48000),
                kit_metadata(41, "Drums"),
                kit_pad(301, 0, 36),
                json!({"op":"assign_kit","instrument_id":1,"kit_id":41}),
                json!({"op":"set_song","bpm":120,"lines_per_beat":4}),
                json!({"op":"upsert_pattern","id":1,"name":"Pitch test","rows":4}),
                kit_cell(1, 0, json!({"Note":36}), 1),
            ]
        )["ok"],
        true
    );
    let native = kit_export(&mut c, &f, 2, "native-pad.wav");
    let mut changed = kit_pad(301, 0, 84);
    changed["root_note"] = json!(64);
    // Remove old selector before changing mapping; every intermediate state is valid.
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"clear_cell","pattern_id":1,"track_id":1,"row":0,"column":0}),
                changed.clone(),
                kit_cell(1, 0, json!({"Note":84}), 1)
            ]
        )["ok"],
        true
    );
    assert_eq!(kit_export(&mut c, &f, 3, "other-selector.wav"), native);
    changed["tune_semitones"] = json!(12);
    assert_eq!(edit(&mut c, 3, vec![changed.clone()])["ok"], true);
    let octave = kit_export(&mut c, &f, 4, "octave-pad.wav");
    for frame in 0..1000 {
        let expected = (0.1f32 + (frame * 2) as f32 / 100000.0) * 0.5;
        assert!(
            (octave[frame * 2] - expected).abs() < 0.000002,
            "explicit tune mismatch frame{frame}: {} vs {expected}",
            octave[frame * 2]
        );
    }
    assert_ne!(octave, native);
    let before = c.document();
    changed["root_note"] = json!(120);
    assert_eq!(edit(&mut c, 4, vec![changed])["ok"], false);
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), original);
}

#[test]
fn drum_kit_sources_retain_root_hash_and_no_clobber_guards() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 2, vec![kit_cell(1, 0, json!({"Note":36}), 1)])["ok"],
        true
    );
    let before = c.document();
    let inspected = c.ok(
        "leviathan_sample_inspect",
        json!({"expected_revision":3,"sample_id":7}),
    );
    assert_eq!(
        inspected["data"]["kit_pad_mappings"],
        json!([
            {"kit_id":41,"pad_id":301,"slot":0,"selector_note":36,"region_id":21},
            {"kit_id":41,"pad_id":302,"slot":15,"selector_note":38,"region_id":21},
        ])
    );
    let source = fs::read(f.root.join("kit.wav")).unwrap();
    let mut changed = source.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fs::write(f.root.join("kit.wav"), changed).unwrap();
    let failed = c.call(
        "leviathan_song_export",
        json!({"expected_revision":3,"path":"changed.wav"}),
    );
    assert_eq!(failed["ok"], false);
    assert!(!f.root.join("changed.wav").exists());
    assert!(!f.root.join("changed.events.json").exists());
    assert_eq!(c.document(), before);
    fs::write(f.root.join("kit.wav"), &source).unwrap();
    for path in ["../escape.wav", "/tmp/mcp-kit-escape.wav"] {
        assert_eq!(
            c.call(
                "leviathan_song_export",
                json!({"expected_revision":3,"path":path})
            )["error"]["code"],
            "path_denied"
        );
    }
    #[cfg(unix)]
    {
        fs::write(f.base.join("outside.wav"), &source).unwrap();
        fs::remove_file(f.root.join("kit.wav")).unwrap();
        std::os::unix::fs::symlink(f.base.join("outside.wav"), f.root.join("kit.wav")).unwrap();
        assert_eq!(
            c.call(
                "leviathan_song_export",
                json!({"expected_revision":3,"path":"symlink.wav"})
            )["error"]["code"],
            "path_denied"
        );
        assert!(!f.root.join("symlink.wav").exists());
        assert!(!f.root.join("symlink.events.json").exists());
        fs::remove_file(f.root.join("kit.wav")).unwrap();
        fs::write(f.root.join("kit.wav"), &source).unwrap();
    }
    kit_export(&mut c, &f, 3, "protected.wav");
    let protected = fs::read(f.root.join("protected.wav")).unwrap();
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":3,"path":"protected.wav"})
        )["ok"],
        false
    );
    assert_eq!(fs::read(f.root.join("protected.wav")).unwrap(), protected);
    assert_eq!(c.document(), before);
}

#[test]
fn fresh_drum_kit_instrument_preserves_occupied_song_matches_editor_and_is_one_undo() {
    use aqua_pattern_core::{
        editor::Editor,
        song::{DrumKitId, InstrumentId, Song, TrackId},
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 2, vec![kit_cell(1, 0, json!({"Note":36}), 1)])["ok"],
        true
    );
    let before = c.document();
    let old_song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let old_manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":3}))["data"]["manifest"].clone();
    let mut editor = Editor::default();
    editor.replace_document(old_song.clone()).unwrap();
    editor.edit_mode = true;
    editor.select(0, TrackId(2)).unwrap();
    editor.row = 3;
    editor.column = 1;
    let cursor = (
        editor.occurrence,
        editor.track,
        editor.row,
        editor.column,
        editor.field,
    );
    let ids = editor
        .create_drum_kit_instrument("Fresh kit".into())
        .unwrap();
    let next_instrument_id = old_song.instruments.keys().map(|id| id.0).max().unwrap() + 1;
    assert_eq!(ids, (InstrumentId(next_instrument_id), DrumKitId(42)));
    assert_eq!(
        (
            editor.occurrence,
            editor.track,
            editor.row,
            editor.column,
            editor.field
        ),
        cursor
    );
    let create = json!({"op":"create_drum_kit_instrument","name":"Fresh kit"});
    // A later failure must not leave either half of the compound action behind.
    let failed = edit(
        &mut c,
        3,
        vec![create.clone(), json!({"op":"remove_track","id":999})],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 1);
    assert_eq!(c.document(), before);
    let result = edit(&mut c, 3, vec![create]);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["revision"], 4);
    let after = c.document();
    let from_mcp: Song = serde_json::from_value(after["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    for (id, instrument) in &old_song.instruments {
        assert_eq!(&from_mcp.instruments[id], instrument);
    }
    assert_eq!(from_mcp.patterns, old_song.patterns);
    assert_eq!(from_mcp.order, old_song.order);
    assert_eq!(from_mcp.tracks, old_song.tracks);
    assert!(from_mcp.drum_kits[&ids.1].pads.is_empty());
    assert_eq!(
        c.ok("leviathan_song_evaluate", json!({"expected_revision":4}))["data"]["manifest"],
        old_manifest
    );
    let inspected = c.ok("leviathan_project_read", json!({"section":"instruments"}));
    assert_eq!(
        inspected["data"]["instruments"][next_instrument_id.to_string()]["kind"],
        json!({"type":"DrumKit","data":{"kit":42}})
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
}

#[test]
fn fresh_drum_kit_instrument_names_and_both_id_exhaustions_fail_atomically() {
    for saturated in [
        json!({"op":"upsert_instrument","id":u64::MAX,"name":"Last instrument","sample":null}),
        kit_metadata(u64::MAX, "Last kit"),
    ] {
        let f = Fixture::new();
        let mut c = Client::ready(&f);
        assert_eq!(edit(&mut c, 0, vec![saturated])["ok"], true);
        let before = c.document();
        let failed = edit(
            &mut c,
            1,
            vec![
                title("not committed"),
                json!({"op":"create_drum_kit_instrument","name":"No wrap or hole reuse"}),
            ],
        );
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["revision"], 1);
        assert_eq!(failed["error"]["details"]["operation_index"], 1);
        assert_eq!(failed["error"]["details"]["atomic"], true);
        assert_eq!(c.document(), before);
    }
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = c.document();
    for name in [String::new(), "   ".into(), "é".repeat(129)] {
        let result = edit(
            &mut c,
            0,
            vec![json!({"op":"create_drum_kit_instrument","name":name})],
        );
        assert_eq!(result["ok"], false);
        assert_eq!(c.document(), before);
    }
    let original_instrument_count = c.document()["data"]["song"]["instruments"]
        .as_object()
        .unwrap()
        .len();
    let result = edit(
        &mut c,
        0,
        vec![json!({"op":"create_drum_kit_instrument","name":"é".repeat(128)})],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["session"]["drum_kit_count"], 1);
    assert_eq!(
        result["data"]["session"]["instrument_count"],
        original_instrument_count + 1
    );
}

fn drum_steps_inspect(
    c: &mut Client,
    revision: u64,
    division: &str,
    steps: u8,
    page: u32,
) -> Value {
    c.ok("leviathan_drum_steps_inspect",json!({"expected_revision":revision,"pattern_id":1,"track_id":1,"instrument_id":1,"division":division,"steps":steps,"page":page}))
}
fn step_field(op: &str, inspected: &Value, pad: u64, row: u32, column: u16, value: Value) -> Value {
    json!({"op":op,"projection_token":inspected["data"]["projection_token"],"pad_id":pad,"row":row,"column":column,"value":value})
}
fn step_toggle(inspected: &Value, pad: u64, step: u8) -> Value {
    json!({"op":"drum_step_toggle","projection_token":inspected["data"]["projection_token"],"pad_id":pad,"step":step})
}
fn step_source_event(
    projection: &aqua_pattern_core::drum_steps::StepProjection,
    pad: u64,
    row: u32,
    column: u16,
) -> aqua_pattern_core::drum_steps::StepEvent {
    projection
        .lanes
        .iter()
        .filter(|lane| lane.pad.0 == pad)
        .flat_map(|lane| &lane.steps)
        .flat_map(|slot| &slot.events)
        .find(|event| event.row == row && event.column == column)
        .unwrap()
        .clone()
}

#[test]
fn drum_steps_shared_editor_projection_and_mcp_edits_are_exact_one_batch_and_roundtrip() {
    use aqua_pattern_core::{
        drum_steps::{self, StepDivision, StepEdit, StepView},
        editor::Editor,
        song::*,
        song_render,
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"upsert_pattern","id":1,"name":"Steps","rows":64}),
                kit_cell(1, 0, json!({"Note":36}), 1),
                kit_cell(2, 5, json!({"Note":38}), 1)
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let view = StepView {
        pattern: PatternId(1),
        track: TrackId(1),
        instrument: InstrumentId(1),
        division: StepDivision::Sixteenth,
        steps: 16,
        page: 0,
    };
    let projection = drum_steps::project(&song, &view).unwrap();
    let inspected = drum_steps_inspect(&mut c, 3, "Sixteenth", 16, 0);
    assert_eq!(
        inspected["data"]["projection"],
        serde_json::to_value(&projection).unwrap()
    );
    assert!(inspected["data"]["projection"].get("base").is_none());
    assert_eq!(c.document(), before);
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor.row = 4;
    for kind in 0..4 {
        let current = drum_steps::project(&editor.song, &view).unwrap();
        let change = match kind {
            0 => StepEdit::SetVelocity {
                event: step_source_event(&current, 301, 0, 0),
                value: Some(0.0),
            },
            1 => StepEdit::SetDelay {
                event: step_source_event(&current, 301, 0, 0),
                value: Some(0.25),
            },
            2 => StepEdit::SetRetrigger {
                event: step_source_event(&current, 301, 0, 0),
                value: Some(Retrigger {
                    repeats: 2,
                    spacing: 0.1,
                }),
            },
            _ => StepEdit::Toggle {
                pad: PadId(302),
                step: 2,
            },
        };
        editor
            .edit_drum_step(editor.revision(), &current, &change)
            .unwrap();
    }
    assert_eq!(editor.row, 4);
    let result = edit(
        &mut c,
        3,
        vec![
            step_field("drum_step_set_velocity", &inspected, 301, 0, 0, json!(0.0)),
            step_field("drum_step_set_delay", &inspected, 301, 0, 0, json!(0.25)),
            step_field(
                "drum_step_set_retrigger",
                &inspected,
                301,
                0,
                0,
                json!({"repeats":2,"spacing":0.1}),
            ),
            step_toggle(&inspected, 302, 2),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["revision"], 4);
    let after = c.document();
    let from_mcp: Song = serde_json::from_value(after["data"]["song"].clone()).unwrap();
    assert_eq!(from_mcp, editor.song);
    assert_eq!(
        from_mcp.patterns[&PatternId(1)].tracks[&TrackId(2)],
        song.patterns[&PatternId(1)].tracks[&TrackId(2)]
    );
    let manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":4}))["data"]["manifest"].clone();
    assert_eq!(
        manifest,
        serde_json::to_value(song_render::evaluate(&editor.song, None).unwrap()).unwrap()
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    let pcm = kit_export(&mut c, &f, 6, "steps.wav");
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"steps.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"steps.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    assert_eq!(kit_export(&mut c, &f, 8, "steps-reopened.wav"), pcm);
    assert_eq!(
        c.ok("leviathan_song_evaluate", json!({"expected_revision":8}))["data"]["manifest"],
        manifest
    );
}

#[test]
fn drum_steps_conflicts_are_visible_read_only_toggles_but_exact_fields_preserve_source_cells() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"assign_kit","instrument_id":2,"kit_id":41}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":36},"instrument":1,"delay":0.10000000000000002,"pan":0.3,"effects":{"cut":0.8}}}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":1,"cell":{"note":{"Note":36},"instrument":1}}),
                kit_cell(1, 2, json!({"Note":36}), 2),
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let inspected = drum_steps_inspect(&mut c, 3, "Sixteenth", 16, 0);
    let slot = &inspected["data"]["projection"]["lanes"][0]["steps"][0];
    assert_eq!(slot["events"].as_array().unwrap().len(), 2);
    assert!(slot["conflicts"].as_array().unwrap().len() >= 3);
    let raw = slot["events"][0]["cell"]["delay"].as_f64().unwrap();
    assert_eq!(raw.to_bits(), 0.10000000000000002f64.to_bits());
    assert!(inspected["data"]["projection"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["row"] == 2));
    let failed = edit(
        &mut c,
        3,
        vec![title("rollback"), step_toggle(&inspected, 301, 0)],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(c.document(), before);
    let changed = edit(
        &mut c,
        3,
        vec![step_field(
            "drum_step_set_velocity",
            &inspected,
            301,
            0,
            0,
            json!(0.4),
        )],
    );
    assert_eq!(changed["ok"], true, "{changed}");
    let after = c.document();
    let mut expected = before["data"]["song"].clone();
    expected["patterns"]["1"]["tracks"]["1"]["0"]["0"]["velocity"] = json!(0.4f32);
    assert_eq!(after["data"]["song"], expected);
    let alias=c.ok("leviathan_drum_steps_inspect",json!({"expected_revision":4,"pattern_id":1,"track_id":1,"instrument_id":2,"division":"Sixteenth","steps":16,"page":0}));
    assert_eq!(
        alias["data"]["projection"]["lanes"][0]["steps"][2]["events"][0]["instrument"],
        2
    );
    assert!(
        alias["data"]["projection"]["lanes"][0]["steps"][0]["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![step_field(
                "drum_step_set_delay",
                &alias,
                301,
                0,
                0,
                Value::Null
            )]
        )["ok"],
        false
    );
    assert_eq!(c.document(), after);
}

#[test]
fn drum_step_tokens_nullable_fields_page_bounds_and_aba_fail_without_mutation() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 2, vec![kit_cell(1, 0, json!({"Note":36}), 1)])["ok"],
        true
    );
    let before = c.document();
    let inspected = drum_steps_inspect(&mut c, 3, "Sixteenth", 16, 0);
    for (key, value) in [
        ("page", json!(u32::MAX)),
        ("page", json!(u64::MAX)),
        ("steps", json!(17)),
        ("steps", json!(0)),
        ("division", json!("Third")),
        ("instrument_id", json!(4)),
    ] {
        let mut args = json!({"expected_revision":3,"pattern_id":1,"track_id":1,"instrument_id":1,"division":"Sixteenth","steps":16,"page":0});
        args[key] = value;
        assert_eq!(c.call("leviathan_drum_steps_inspect", args)["ok"], false);
        assert_eq!(c.document(), before);
    }
    for name in [
        "drum_step_set_velocity",
        "drum_step_set_delay",
        "drum_step_set_retrigger",
    ] {
        let mut missing = step_field(name, &inspected, 301, 0, 0, Value::Null);
        missing.as_object_mut().unwrap().remove("value");
        assert_eq!(
            edit(&mut c, 3, vec![missing])["error"]["code"],
            "invalid_arguments"
        );
    }
    for operation in [
        step_field(
            "drum_step_set_velocity",
            &inspected,
            301,
            0,
            0,
            json!(1.0000000001),
        ),
        step_field("drum_step_set_delay", &inspected, 301, 0, 0, json!(1.0)),
        step_field(
            "drum_step_set_retrigger",
            &inspected,
            301,
            0,
            0,
            json!({"repeats":16,"spacing":0.5}),
        ),
        step_toggle(&inspected, 301, 16),
        step_toggle(&inspected, 301, 4),
    ] {
        assert_eq!(edit(&mut c, 3, vec![operation])["ok"], false);
        assert_eq!(c.document(), before);
    }
    // Explicit null is a no-op here and must preserve token/history/revision.
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![step_field(
                "drum_step_set_velocity",
                &inspected,
                301,
                0,
                0,
                Value::Null
            )]
        )["data"]["changed"],
        false
    );
    let newer = drum_steps_inspect(&mut c, 3, "Eighth", 32, 0);
    assert_eq!(
        edit(&mut c, 3, vec![step_toggle(&inspected, 301, 1)])["error"]["code"],
        "invalid_drum_step_token"
    );
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![step_field(
                "drum_step_set_delay",
                &newer,
                301,
                0,
                0,
                json!(0.0)
            )]
        )["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["delay"],
        0.0
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![step_field(
                "drum_step_set_delay",
                &newer,
                301,
                0,
                0,
                Value::Null
            )]
        )["error"]["code"],
        "drum_step_projection_unavailable"
    );
    let latest = drum_steps_inspect(&mut c, 5, "Sixteenth", 16, 0);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"token.leviathan.json"}),
    );
    assert_eq!(
        edit(&mut c, 6, vec![step_toggle(&latest, 301, 1)])["ok"],
        false
    );
}

#[test]
fn drum_step_projection_budgets_fail_without_truncation_or_document_changes() {
    use aqua_pattern_core::song::*;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    let template: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    for (index, count) in [2000usize, 2049].into_iter().enumerate() {
        let mut song = template.clone();
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 256;
        let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
        pattern.rows = 64;
        for n in 0..count {
            pattern
                .tracks
                .entry(TrackId(1))
                .or_default()
                .entry((n / 256) as u32)
                .or_default()
                .insert(
                    (n % 256) as u16,
                    Cell {
                        note: Note::Note(36),
                        instrument: Some(InstrumentId(1)),
                        ..Default::default()
                    },
                );
        }
        song.validate().unwrap();
        let path = format!("many-{count}.leviathan.json");
        aqua_pattern_core::project::save(
            &song,
            &f.root.join(&path),
            aqua_pattern_core::project::SaveMode::CreateNew,
        )
        .unwrap();
        c.ok(
            "leviathan_project_open",
            json!({"expected_revision":2+index,"path":path,"discard_unsaved":true}),
        );
        let before = c.ok("leviathan_project_read", json!({}));
        let failed=c.call("leviathan_drum_steps_inspect",json!({"expected_revision":3+index,"pattern_id":1,"track_id":1,"instrument_id":1,"division":"Sixteenth","steps":16,"page":0}));
        assert_eq!(failed["ok"], false, "count{count}: {failed}");
        if count == 2000 {
            assert_eq!(failed["error"]["code"], "drum_step_projection_limit");
        } else {
            assert!(failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("2048"));
        }
        assert_eq!(c.ok("leviathan_project_read", json!({})), before);
    }
}

#[test]
fn drum_step_repeat_clone_and_all_page_divisions_match_shared_ordinary_pattern_semantics() {
    use aqua_pattern_core::{
        drum_steps::{self, StepDivision, StepView},
        song::*,
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    let mut hit = kit_cell(1, 0, json!({"Note":36}), 1);
    hit["cell"]["velocity"] = json!(0.8);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                hit,
                json!({"op":"clone_pattern","source_id":1,"new_id":2,"name":"Independent"}),
                json!({"op":"set_order","pattern_ids":[1,1,2]})
            ]
        )["ok"],
        true
    );
    let clone = c.document()["data"]["song"]["patterns"]["2"].clone();
    let inspected = drum_steps_inspect(&mut c, 3, "Sixteenth", 16, 0);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![step_field(
                "drum_step_set_velocity",
                &inspected,
                301,
                0,
                0,
                json!(0.3)
            )]
        )["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"]["patterns"]["2"], clone);
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":4}));
    let volumes: Vec<_> = manifest["data"]["manifest"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["command"].get("On").map(|on| on["volume"].clone()))
        .collect();
    assert_eq!(volumes, vec![json!(0.3f32), json!(0.3f32), json!(0.8f32)]);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"upsert_pattern","id":1,"name":"Pages","rows":64}),
                json!({"op":"set_song","lines_per_beat":7})
            ]
        )["ok"],
        true
    );
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    for (name, division) in [
        ("Quarter", StepDivision::Quarter),
        ("Eighth", StepDivision::Eighth),
        ("Sixteenth", StepDivision::Sixteenth),
        ("ThirtySecond", StepDivision::ThirtySecond),
        ("SixtyFourth", StepDivision::SixtyFourth),
        ("OneTwentyEighth", StepDivision::OneTwentyEighth),
    ] {
        for steps in [16, 32] {
            let total = (64 * division.steps_per_beat()).div_ceil(7);
            let page = total.div_ceil(u32::from(steps)) - 1;
            let expected = drum_steps::project(
                &song,
                &StepView {
                    pattern: PatternId(1),
                    track: TrackId(1),
                    instrument: InstrumentId(1),
                    division,
                    steps,
                    page,
                },
            )
            .unwrap();
            let inspected = drum_steps_inspect(&mut c, 5, name, steps, page);
            assert_eq!(
                inspected["data"]["projection"],
                serde_json::to_value(expected).unwrap()
            );
            assert_eq!(inspected["data"]["projection"]["total_steps"], total);
        }
    }
}

#[test]
fn drum_step_phrase_selection_and_full_column_conflicts_never_delete_or_infer_notes() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":36}}}),
                kit_cell(1, 1, json!({"Note":38}), 1),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":1,"cell":{"note":{"Note":60},"instrument":4}}),
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let inspected = drum_steps_inspect(&mut c, 3, "Sixteenth", 16, 0);
    assert!(
        !inspected["data"]["projection"]["lanes"][0]["steps"][0]["conflicts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for step in [0, 1] {
        assert_eq!(
            edit(&mut c, 3, vec![step_toggle(&inspected, 301, step)])["ok"],
            false
        );
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                phrase_metadata(51),
                json!({"op":"set_phrase_cell","phrase_id":51,"row":0,"column":0,"cell":{"note":{"Note":36}}}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":3,"column":0,"cell":{"note":{"Note":60},"instrument":1,"phrase":{"phrase":51,"cycles":1}}})
            ]
        )["ok"],
        true
    );
    let phrase = c.document();
    let inspected = drum_steps_inspect(&mut c, 4, "Sixteenth", 16, 0);
    assert!(inspected["data"]["projection"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["row"] == 3));
    assert_eq!(
        edit(&mut c, 4, vec![step_toggle(&inspected, 301, 2)])["ok"],
        false
    );
    assert_eq!(c.document(), phrase);
    // A reviewed existing parent remains addressable even when phrase toggles are blocked.
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![step_field(
                "drum_step_set_velocity",
                &inspected,
                302,
                1,
                0,
                json!(0.2)
            )]
        )["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["song"]["patterns"]["1"]["tracks"]["1"]["3"],
        phrase["data"]["song"]["patterns"]["1"]["tracks"]["1"]["3"]
    );
}

#[test]
fn drum_step_ordered_batches_reject_changed_ownership_new_events_and_events_moved_out_of_page() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 2, vec![kit_cell(1, 0, json!({"Note":36}), 1)])["ok"],
        true
    );
    let before = c.document();
    let inspected = drum_steps_inspect(&mut c, 3, "Sixteenth", 16, 0);
    for operations in [
        vec![
            kit_cell(1, 0, json!({"Note":38}), 1),
            step_field("drum_step_set_velocity", &inspected, 301, 0, 0, json!(0.5)),
        ],
        vec![
            json!({"op":"assign_kit","instrument_id":1,"kit_id":null}),
            step_toggle(&inspected, 301, 1),
        ],
        vec![
            step_toggle(&inspected, 302, 1),
            step_field("drum_step_set_velocity", &inspected, 302, 1, 0, json!(0.5)),
        ],
    ] {
        let failed = edit(&mut c, 3, operations);
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["error"]["details"]["operation_index"], 1);
        assert_eq!(c.document(), before);
    }
    let mut delayed = kit_cell(1, 0, json!({"Note":36}), 1);
    delayed["cell"]["delay"] = json!(0.75);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![json!({"op":"set_song","lines_per_beat":1}), delayed]
        )["ok"],
        true
    );
    let before = c.document();
    let fraction = drum_steps_inspect(&mut c, 4, "OneTwentyEighth", 16, 1);
    let failed = edit(
        &mut c,
        4,
        vec![
            step_field("drum_step_set_delay", &fraction, 301, 0, 0, json!(0.25)),
            step_field("drum_step_set_velocity", &fraction, 301, 0, 0, json!(0.5)),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 1);
    assert_eq!(c.document(), before);
}

fn matrix_alias(pattern: u64, track: u64, source: Value) -> Value {
    json!({"op":"set_track_alias","pattern_id":pattern,"track_id":track,"source":source})
}
fn matrix_inspect(c: &mut Client, revision: u64) -> Value {
    c.ok(
        "leviathan_matrix_inspect",
        json!({"expected_revision":revision}),
    )
}
fn matrix_setup(c: &mut Client, f: &Fixture) {
    kit_setup(c, f);
    assert_eq!(
        edit(
            c,
            2,
            vec![
                kit_cell(1, 0, json!({"Note":36}), 1),
                json!({"op":"upsert_pattern","id":2,"name":"Alias A","rows":4}),
                json!({"op":"upsert_pattern","id":3,"name":"Alias B","rows":4}),
                json!({"op":"set_order","pattern_ids":[1,2,3]})
            ]
        )["ok"],
        true
    );
}

#[test]
fn matrix_aliases_and_mutes_match_shared_editor_resolver_and_manifest_with_one_undo() {
    use aqua_pattern_core::{editor::Editor, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    matrix_setup(&mut c, &f);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(song).unwrap();
    editor.edit_mode = true;
    editor
        .set_track_alias(
            PatternId(2),
            TrackId(2),
            Some(TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            }),
        )
        .unwrap();
    editor
        .set_track_alias(
            PatternId(3),
            TrackId(3),
            Some(TrackAlias {
                pattern: PatternId(2),
                track: TrackId(2),
            }),
        )
        .unwrap();
    editor
        .set_occurrence_track_mute(1, TrackId(2), true)
        .unwrap();
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                matrix_alias(2, 2, json!({"pattern":1,"track":1})),
                matrix_alias(3, 3, json!({"pattern":2,"track":2})),
                json!({"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":2,"muted":true})
            ]
        )["ok"],
        true
    );
    let after = c.document();
    let actual: Song = serde_json::from_value(after["data"]["song"].clone()).unwrap();
    assert_eq!(actual, editor.song);
    let inspected = matrix_inspect(&mut c, 4);
    let expected_matrix_blocks = inspected["data"]["occurrences_total"]
        .as_u64()
        .unwrap()
        .min(16)
        * inspected["data"]["tracks_total"].as_u64().unwrap().min(16);
    assert_eq!(
        inspected["data"]["blocks"].as_array().unwrap().len() as u64,
        expected_matrix_blocks
    );
    let alias = inspected["data"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["occurrence_index"] == 2 && block["track_id"] == 3)
        .unwrap();
    assert_eq!(alias["stored_local_cell_count"], 0);
    assert_eq!(alias["resolved_cell_count"], 1);
    assert_eq!(alias["local_editable"], false);
    assert_eq!(
        alias["source_chain"],
        json!([{"pattern":3,"track":3},{"pattern":2,"track":2},{"pattern":1,"track":1}])
    );
    assert_eq!(alias["resolved_source"], json!({"pattern":1,"track":1}));
    assert_eq!(after["data"]["session"]["track_alias_count"], 2);
    assert_eq!(after["data"]["session"]["occurrence_track_mute_count"], 1);
    let manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":4}))["data"]["manifest"].clone();
    assert_eq!(
        manifest,
        serde_json::to_value(song_render::evaluate(&editor.song, None).unwrap()).unwrap()
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    let pcm = kit_export(&mut c, &f, 6, "matrix.wav");
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"matrix.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"matrix.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    assert_eq!(kit_export(&mut c, &f, 8, "matrix-reopened.wav"), pcm);
    assert_eq!(
        c.ok("leviathan_song_evaluate", json!({"expected_revision":8}))["data"]["manifest"],
        manifest
    );
}

#[test]
fn alias_targets_reject_every_direct_write_and_step_edit_until_explicit_unique_or_unlink() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    matrix_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                matrix_alias(2, 2, json!({"pattern":1,"track":1})),
                matrix_alias(3, 3, json!({"pattern":2,"track":2}))
            ]
        )["ok"],
        true
    );
    let before = c.document();
    for op in [
        json!({"op":"set_cell","pattern_id":2,"track_id":2,"row":0,"column":0,"cell":{"note":{"Note":38},"instrument":1}}),
        json!({"op":"clear_cell","pattern_id":2,"track_id":2,"row":0,"column":0}),
        json!({"op":"set_effects","pattern_id":2,"track_id":2,"row":0,"column":0,"effects":{"cut":0.5}}),
        json!({"op":"set_phrase_trigger","pattern_id":2,"track_id":2,"row":0,"column":0,"trigger":null}),
        json!({"op":"clear_pattern_track","pattern_id":2,"track_id":2}),
    ] {
        let failed = edit(&mut c, 4, vec![title("rollback"), op]);
        assert_eq!(failed["ok"], false);
        assert_eq!(c.document(), before);
    }
    let projected=c.ok("leviathan_drum_steps_inspect",json!({"expected_revision":4,"pattern_id":2,"track_id":2,"instrument_id":1,"division":"Sixteenth","steps":16}));
    assert_eq!(projected["data"]["projection"]["read_only"], true);
    assert_eq!(
        projected["data"]["projection"]["source"],
        json!({"pattern":1,"track":1})
    );
    for op in [
        step_toggle(&projected, 301, 0),
        step_field("drum_step_set_velocity", &projected, 301, 0, 0, json!(0.5)),
    ] {
        assert_eq!(edit(&mut c, 4, vec![op])["ok"], false);
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"make_track_unique","pattern_id":2,"track_id":2}),
                json!({"op":"set_cell","pattern_id":2,"track_id":2,"row":0,"column":0,"cell":{"note":{"Note":38},"instrument":1}})
            ]
        )["ok"],
        true
    );
    let unique = c.document();
    assert!(unique["data"]["song"]["patterns"]["2"]
        .get("track_aliases")
        .is_none());
    assert_eq!(
        unique["data"]["song"]["patterns"]["1"],
        before["data"]["song"]["patterns"]["1"]
    );
    // Pattern3 still shares pattern2; cloning it materializes an independent track.
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![json!({"op":"clone_pattern","source_id":3,"new_id":4,"name":"Independent"})]
        )["ok"],
        true
    );
    let clone = c.document()["data"]["song"]["patterns"]["4"].clone();
    assert!(clone.get("track_aliases").is_none());
    assert_eq!(clone["tracks"]["3"]["0"]["0"]["note"], json!({"Note":38}));
    assert_eq!(
        edit(&mut c, 6, vec![matrix_alias(3, 3, Value::Null)])["ok"],
        true
    );
    let unlinked = c.document();
    assert!(unlinked["data"]["song"]["patterns"]["3"]["tracks"]
        .as_object()
        .unwrap()
        .is_empty());
    assert_eq!(unlinked["data"]["song"]["patterns"]["4"], clone);
    // Explicit unlink on an already-local block never deletes it.
    assert_eq!(
        edit(&mut c, 7, vec![matrix_alias(2, 2, Value::Null)])["data"]["changed"],
        false
    );
}

#[test]
fn alias_reference_cycles_geometry_and_atomic_unique_failures_include_unused_sources() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    matrix_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                matrix_alias(2, 2, json!({"pattern":1,"track":1})),
                matrix_alias(3, 3, json!({"pattern":2,"track":2})),
                json!({"op":"set_order","pattern_ids":[2,3]})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    for op in [
        json!({"op":"remove_pattern","id":1}),
        json!({"op":"upsert_pattern","id":1,"name":"wrong rows","rows":8}),
        json!({"op":"upsert_track","id":1,"name":"wrong columns","note_columns":1}),
        matrix_alias(2, 2, json!({"pattern":999,"track":1})),
        matrix_alias(2, 2, json!({"pattern":1,"track":999})),
        matrix_alias(2, 2, json!({"pattern":3,"track":3})),
        matrix_alias(1, 1, json!({"pattern":2,"track":2})),
        json!({"op":"set_occurrence_track_mute","occurrence_index":u64::MAX,"track_id":1,"muted":true}),
        json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":999,"muted":true}),
    ] {
        assert_eq!(edit(&mut c, 4, vec![op])["ok"], false);
        assert_eq!(c.document(), before);
    }
    let missing = json!({"op":"set_track_alias","pattern_id":2,"track_id":2});
    assert_eq!(
        edit(&mut c, 4, vec![missing])["error"]["code"],
        "invalid_arguments"
    );
    let failed = edit(
        &mut c,
        4,
        vec![
            json!({"op":"make_track_unique","pattern_id":2,"track_id":2}),
            json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":2,"muted":true}),
            json!({"op":"remove_pattern","id":999}),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 2);
    assert_eq!(c.document(), before);
}

#[test]
fn occurrence_mute_is_per_record_releases_not_cuts_and_swaps_and_repeats_are_explicit() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                kit_cell(1, 0, json!({"Note":36}), 1),
                json!({"op":"set_order","pattern_ids":[1,1,1]}),
                json!({"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":1,"muted":true})
            ]
        )["ok"],
        true
    );
    let muted = kit_export(&mut c, &f, 3, "muted.wav");
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let starts: Vec<_> = manifest["data"]["manifest"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["command"].get("On").is_some())
        .map(|event| event["frame"].as_u64().unwrap())
        .collect();
    assert_eq!(starts, vec![0, 48000]);
    assert!(
        muted[24000 * 2] > 0.0,
        "mute must retain a de-click release, not claim hard silence"
    );
    assert!(muted[36000 * 2].abs() < 0.00001);
    assert!(muted[48000 * 2] > 0.05);
    let cells = c.document()["data"]["song"]["patterns"].clone();
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"move_occurrence","from_index":1,"to_index":2}),
                json!({"op":"append_repeat","pattern_id":1})
            ]
        )["ok"],
        true
    );
    let moved = c.document();
    assert_eq!(
        moved["data"]["song"]["order"][2]["muted_tracks"],
        json!([1])
    );
    assert!(moved["data"]["song"]["order"][1]
        .get("muted_tracks")
        .is_none());
    assert!(moved["data"]["song"]["order"][3]
        .get("muted_tracks")
        .is_none());
    assert_eq!(moved["data"]["song"]["patterns"], cells);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"move_occurrence","from_index":2,"to_index":2})]
        )["data"]["changed"],
        false
    );
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"move_occurrence","from_index":u64::MAX,"to_index":0})]
        )["ok"],
        false
    );
    assert_eq!(c.document(), moved);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"set_order","pattern_ids":[1,1]})]
        )["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["session"]["occurrence_track_mute_count"],
        0
    );
}

#[test]
fn alias_depth_and_expansion_budgets_and_matrix_pages_are_bounded_and_atomic() {
    use aqua_pattern_core::{project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    matrix_setup(&mut c, &f);
    let mut operations = Vec::new();
    let mut previous = 1;
    for id in 100..132 {
        operations.push(json!({"op":"upsert_pattern","id":id,"name":"Chain","rows":4}));
        operations.push(matrix_alias(id, 1, json!({"pattern":previous,"track":1})));
        previous = id;
    }
    assert_eq!(edit(&mut c, 3, operations)["ok"], true);
    let before = c.document();
    assert_eq!(before["data"]["session"]["track_alias_count"], 32);
    let failed = edit(
        &mut c,
        4,
        vec![
            json!({"op":"upsert_pattern","id":132,"name":"Too deep","rows":4}),
            matrix_alias(132, 1, json!({"pattern":131,"track":1})),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(c.document(), before);
    let mut operations: Vec<_> = (5..33)
        .map(|id| json!({"op":"upsert_track","id":id,"name":"Matrix track","note_columns":2}))
        .collect();
    operations.push(json!({"op":"set_order","pattern_ids":vec![131;32]}));
    assert_eq!(edit(&mut c, 4, operations)["ok"], true);
    let before = c.document();
    let oversized = c.call(
        "leviathan_matrix_inspect",
        json!({"expected_revision":5,"occurrence_limit":32,"track_limit":32}),
    );
    assert_eq!(oversized["error"]["code"], "matrix_inspection_limit");
    let mut keys = std::collections::BTreeSet::new();
    for occurrence in [0, 16] {
        for track in [0, 16] {
            let page = c.ok(
                "leviathan_matrix_inspect",
                json!({"expected_revision":5,"occurrence_start":occurrence,"track_start":track}),
            );
            assert_eq!(page["data"]["blocks"].as_array().unwrap().len(), 256);
            for block in page["data"]["blocks"].as_array().unwrap() {
                assert!(keys.insert((
                    block["occurrence_index"].as_u64().unwrap(),
                    block["track_id"].as_u64().unwrap()
                )));
                if block["track_id"] == 1 {
                    assert_eq!(block["source_chain"].as_array().unwrap().len(), 33);
                }
            }
        }
    }
    assert_eq!(keys.len(), 1024);
    assert!(c.ok(
        "leviathan_matrix_inspect",
        json!({"expected_revision":5,"occurrence_start":32,"track_start":32})
    )["data"]["blocks"]
        .as_array()
        .unwrap()
        .is_empty());
    for args in [
        json!({"expected_revision":5,"occurrence_start":u64::MAX}),
        json!({"expected_revision":5,"track_start":u64::MAX}),
        json!({"expected_revision":5,"occurrence_limit":33}),
        json!({"expected_revision":5,"track_limit":0}),
    ] {
        assert_eq!(c.call("leviathan_matrix_inspect", args)["ok"], false);
    }
    assert_eq!(c.document(), before);
    // A tiny stored block cannot amplify through unused aliases beyond the shared cap.
    let mut dense: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    dense.patterns.retain(|id, _| *id == PatternId(1));
    dense.order = vec![Occurrence {
        pattern: PatternId(1),
        muted_tracks: Default::default(),
        section: None,
    }];
    dense.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 128;
    let source = dense.patterns.get_mut(&PatternId(1)).unwrap();
    source.rows = 8;
    source.tracks.clear();
    for n in 0..1024 {
        source
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(n / 128)
            .or_default()
            .insert(
                (n % 128) as u16,
                Cell {
                    note: Note::Note(36),
                    instrument: Some(InstrumentId(1)),
                    ..Default::default()
                },
            );
    }
    for id in 100..164 {
        dense.patterns.insert(
            PatternId(id),
            Pattern {
                name: "Bounded share".into(),
                rows: 8,
                tracks: Default::default(),
                track_aliases: std::collections::BTreeMap::from([(
                    TrackId(1),
                    TrackAlias {
                        pattern: PatternId(1),
                        track: TrackId(1),
                    },
                )]),
                generation: None,
            },
        );
    }
    dense.validate().unwrap();
    project::save(
        &dense,
        &f.root.join("dense-alias.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"dense-alias.leviathan.json","discard_unsaved":true}),
    );
    let before = c.document();
    let failed = edit(
        &mut c,
        6,
        vec![
            json!({"op":"upsert_pattern","id":164,"name":"Too much resolved content","rows":8}),
            matrix_alias(164, 1, json!({"pattern":1,"track":1})),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("65536"));
    assert_eq!(c.document(), before);
}

#[test]
fn explicit_all_muted_arrangement_exports_silence_without_decoding_unused_bytes_but_keeps_root_policy(
) {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(
        c.call("leviathan_song_evaluate", json!({"expected_revision":0}))["ok"],
        false
    );
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                kit_cell(1, 0, json!({"Note":36}), 1),
                json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":1,"muted":true})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let expected = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    assert_eq!(expected["data"]["manifest"]["musical_frames"], 24000);
    let original = fs::read(f.root.join("kit.wav")).unwrap();
    // Root-authorized regular file remains present, but cannot decode/hash as this source.
    fs::write(f.root.join("kit.wav"), b"not the imported WAV anymore").unwrap();
    let silence = kit_export(&mut c, &f, 3, "all-muted.wav");
    assert!(silence.len() >= 48000);
    assert!(silence.iter().all(|value| *value == 0.0));
    assert_eq!(c.document(), before);
    fs::write(f.root.join("kit.wav"), &original).unwrap();
    #[cfg(unix)]
    {
        fs::write(f.base.join("outside-muted.wav"), &original).unwrap();
        fs::remove_file(f.root.join("kit.wav")).unwrap();
        std::os::unix::fs::symlink(f.base.join("outside-muted.wav"), f.root.join("kit.wav"))
            .unwrap();
        assert_eq!(
            c.call(
                "leviathan_song_export",
                json!({"expected_revision":3,"path":"denied-muted.wav"})
            )["error"]["code"],
            "path_denied"
        );
        assert!(!f.root.join("denied-muted.wav").exists());
        assert!(!f.root.join("denied-muted.events.json").exists());
        fs::remove_file(f.root.join("kit.wav")).unwrap();
        fs::write(f.root.join("kit.wav"), &original).unwrap();
    }
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":3,"path":"all-muted.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":4,"path":"all-muted.leviathan.json"}),
    );
    assert_eq!(kit_export(&mut c, &f, 5, "all-muted-reopened.wav"), silence);
}

fn transform_config() -> Value {
    json!({"scope":{"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[0]},
        "fields":{"notes":true,"glide_targets":true,"velocity":true,"timing":false},
        "transpose_semitones":12,"velocity_scale":0.5,"nudge_frames":0,"swing":null})
}
fn transform_fixture(c: &mut Client) -> Value {
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"tone.wav"}),
    );
    let changed = edit(
        c,
        0,
        vec![
            json!({"op":"set_song","bpm":120.0,"lines_per_beat":4}),
            json!({"op":"upsert_pattern","id":1,"name":"Transform source","rows":8}),
            json!({"op":"upsert_track","id":1,"name":"Source","note_columns":2}),
            json!({"op":"upsert_instrument","id":1,"name":"Pitched","sample":"tone.wav"}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5,"pan":-0.25,"delay":0.125,"effects":{"glide":{"target":62,"semitones_per_second":12.0}}}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":{"Note":64},"instrument":1}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":2,"column":0,"cell":{"note":"Off","pan":0.0,"delay":0.0}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":3,"column":0,"cell":{"note":{"Note":67},"instrument":1,"velocity":0.25,"delay":0.25}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":1,"cell":{"note":{"Note":72},"instrument":1,"velocity":0.75}}),
            json!({"op":"set_cell","pattern_id":1,"track_id":2,"row":0,"column":0,"cell":{"note":"Empty","pan":0.5}}),
        ],
    );
    assert_eq!(changed["ok"], true, "{changed}");
    c.document()
}
fn transform_preview(c: &mut Client, revision: u64, config: Value) -> Value {
    c.ok(
        "leviathan_transform_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn transform_commit(c: &mut Client, revision: u64, preview: &Value) -> Value {
    c.call(
        "leviathan_transform_commit",
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}
fn transform_page(
    c: &mut Client,
    revision: u64,
    preview: &Value,
    start: usize,
    limit: usize,
) -> Value {
    c.ok("leviathan_transform_inspect", json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"],"start":start,"limit":limit}))
}

#[test]
fn transform_real_stdio_exact_editor_preview_diff_manifest_undo_and_save_roundtrip() {
    use aqua_pattern_core::{editor::Editor, song::Song, song_render, transforms};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let options: transforms::TransformConfig = serde_json::from_value(transform_config()).unwrap();
    let shared = transforms::preview(&base, options).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let preview = transform_preview(&mut c, 1, transform_config());
    assert_eq!(preview["data"]["transform_preview_id"], shared.id());
    assert_eq!(
        preview["data"]["effective_config"],
        serde_json::to_value(shared.config()).unwrap()
    );
    assert_eq!(
        preview["data"]["summary"],
        serde_json::to_value(shared.summary()).unwrap()
    );
    assert_eq!(
        preview["data"]["changed_cell_count"],
        shared.changes().len()
    );
    let mut changes = Vec::new();
    for start in (0..shared.changes().len()).step_by(2) {
        let page = transform_page(&mut c, 1, &preview, start, 2);
        changes.extend(page["data"]["changes"].as_array().unwrap().clone());
    }
    assert_eq!(
        json!(changes),
        serde_json::to_value(shared.changes()).unwrap()
    );
    assert_eq!(
        transform_page(&mut c, 1, &preview, changes.len(), 2)["data"]["changes"],
        json!([])
    );
    assert_eq!(
        c.document(),
        before,
        "Review cannot change Song/history/revision"
    );
    editor
        .commit_transform_preview(editor.revision(), &shared)
        .unwrap();
    let committed = transform_commit(&mut c, 1, &preview);
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["revision"], 2);
    let after = c.document();
    let actual: Song = serde_json::from_value(after["data"]["song"].clone()).unwrap();
    assert_eq!(actual, editor.song);
    assert_eq!(actual, *shared.candidate());
    assert_eq!(
        actual.patterns[&aqua_pattern_core::song::PatternId(1)].tracks
            [&aqua_pattern_core::song::TrackId(1)][&0][&1],
        base.patterns[&aqua_pattern_core::song::PatternId(1)].tracks
            [&aqua_pattern_core::song::TrackId(1)][&0][&1]
    );
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    assert_eq!(
        manifest["data"]["manifest"],
        serde_json::to_value(song_render::evaluate(&editor.song, None).unwrap()).unwrap()
    );
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":2,"path":"transformed.wav"}),
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"transformed.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"transformed.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":6,"path":"reopened-transform.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("transformed.wav")).unwrap(),
        fs::read(f.root.join("reopened-transform.wav")).unwrap()
    );
    assert_eq!(
        fs::read(f.root.join("transformed.events.json")).unwrap(),
        fs::read(f.root.join("reopened-transform.events.json")).unwrap()
    );
}

#[test]
fn transform_tokens_supersession_failed_preview_aba_save_open_and_noop_preserve_history() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let base = transform_fixture(&mut c);
    let old = transform_preview(&mut c, 1, transform_config());
    let current = transform_preview(&mut c, 1, transform_config());
    assert_ne!(
        old["data"]["preview_token"],
        current["data"]["preview_token"]
    );
    assert_eq!(
        transform_commit(&mut c, 1, &old)["error"]["code"],
        "invalid_transform_preview_token"
    );
    let mut bad = transform_config();
    bad["velocity_scale"] = json!(4);
    let failure = c.call(
        "leviathan_transform_preview",
        json!({"expected_revision":1,"config":bad}),
    );
    assert_eq!(failure["ok"], false);
    assert_eq!(c.document(), base);
    assert_eq!(transform_page(&mut c, 1, &current, 0, 1)["ok"], true);
    assert_eq!(edit(&mut c, 1, vec![title("temporary")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], base["data"]["song"]);
    assert_eq!(
        transform_commit(&mut c, 3, &current)["error"]["code"],
        "transform_preview_unavailable"
    );
    let redo_before = c.ok("leviathan_project_read", json!({"section":"history"}));
    let mut noop = transform_config();
    noop["fields"] = json!({"notes":false,"glide_targets":false,"velocity":false,"timing":false});
    let preview = transform_preview(&mut c, 3, noop);
    assert_eq!(preview["data"]["changed_cell_count"], 0);
    let result = transform_commit(&mut c, 3, &preview);
    assert_eq!(result["data"]["changed"], false);
    assert_eq!(result["revision"], 3);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        redo_before
    );
    assert_eq!(
        transform_commit(&mut c, 3, &preview)["error"]["code"],
        "transform_preview_unavailable"
    );
    c.ok("leviathan_history_redo", json!({"expected_revision":3}));
    let preview = transform_preview(&mut c, 4, transform_config());
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"review.leviathan.json"}),
    );
    assert_eq!(
        transform_commit(&mut c, 5, &preview)["error"]["code"],
        "transform_preview_unavailable"
    );
    let preview = transform_preview(&mut c, 5, transform_config());
    let song = c.document()["data"]["song"].clone();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"review.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], song);
    assert_eq!(
        transform_commit(&mut c, 6, &preview)["error"]["code"],
        "transform_preview_unavailable"
    );
}

#[test]
fn transform_schema_runtime_bounds_masks_and_unknown_seed_are_strict_and_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let mut invalids = Vec::new();
    for scalar in [json!(4.0000000001), json!(-1e-50), json!("NaN")] {
        let mut x = transform_config();
        x["velocity_scale"] = scalar;
        invalids.push(x);
    }
    for (key, value) in [
        ("seed", json!(7)),
        ("candidate", json!({})),
        ("nudge_frames", json!(48001)),
        ("transpose_semitones", json!(49)),
    ] {
        let mut x = transform_config();
        x[key] = value;
        invalids.push(x);
    }
    for scope in [
        json!({"pattern":1,"track":1,"row_start":0,"row_end":9,"columns":[0]}),
        json!({"pattern":1,"track":1,"row_start":2,"row_end":2,"columns":[0]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[0,0]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[1,0]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[2]}),
    ] {
        let mut x = transform_config();
        x["scope"] = scope;
        invalids.push(x);
    }
    let mut x = transform_config();
    x.as_object_mut().unwrap().remove("swing");
    invalids.push(x);
    let mut x = transform_config();
    x["fields"].as_object_mut().unwrap().remove("velocity");
    invalids.push(x);
    let mut x = transform_config();
    x["swing"] = json!({"division":"Sixteenth","amount_percent":51});
    invalids.push(x);
    for config in invalids {
        let failed = c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":1,"config":config}),
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
    }
    let preview = transform_preview(&mut c, 1, transform_config());
    for (start, limit) in [(u64::MAX, 1), (0, 0), (0, 65)] {
        let failed=c.call("leviathan_transform_inspect",json!({"expected_revision":1,"preview_token":preview["data"]["preview_token"],"start":start,"limit":limit}));
        assert_eq!(failed["ok"], false);
        assert_eq!(c.document(), before);
    }
    let catalog = c.request("tools/list", json!({}));
    let schema = &catalog["result"]["tools"][25]["inputSchema"]["properties"]["config"];
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["fields"]["required"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(schema["properties"].get("seed").is_none());
    assert_eq!(
        catalog["result"]["tools"][26]["annotations"]["readOnlyHint"],
        true
    );
}

#[test]
fn transform_timing_nudge_swing_exact_frames_and_field_masks_do_not_rewrite_other_data() {
    use aqua_pattern_core::{song::Song, transforms};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    for mask in 0..16 {
        let mut config = transform_config();
        config["fields"] = json!({"notes":mask&1!=0,"glide_targets":mask&2!=0,"velocity":mask&4!=0,"timing":mask&8!=0});
        config["nudge_frames"] = json!(120);
        config["swing"] = json!({"division":"Sixteenth","amount_percent":25});
        let shared =
            transforms::preview(&base, serde_json::from_value(config.clone()).unwrap()).unwrap();
        let preview = transform_preview(&mut c, 1, config);
        let changes = transform_page(&mut c, 1, &preview, 0, 64);
        assert_eq!(
            changes["data"]["changes"],
            serde_json::to_value(shared.changes()).unwrap()
        );
        for change in shared.changes() {
            if mask & 1 == 0 {
                assert_eq!(change.before.note, change.after.note);
            }
            if mask & 2 == 0 {
                assert_eq!(change.before.effects.glide, change.after.effects.glide);
            }
            if mask & 4 == 0 {
                assert_eq!(change.before.velocity, change.after.velocity);
            }
            if mask & 8 == 0 {
                assert_eq!(change.before.delay, change.after.delay);
                assert_eq!(change.from_row, change.to_row);
            }
            assert_eq!(change.before.instrument, change.after.instrument);
            assert_eq!(change.before.pan, change.after.pan);
            assert_eq!(change.before.phrase, change.after.phrase);
            let mut allowed = change.before.clone();
            if mask & 1 != 0 {
                allowed.note = change.after.note;
            }
            if mask & 2 != 0 {
                allowed.effects.glide = change.after.effects.glide.clone();
            }
            if mask & 4 != 0 {
                allowed.velocity = change.after.velocity;
            }
            if mask & 8 != 0 {
                allowed.delay = change.after.delay;
                allowed.effects.release = change.after.effects.release;
                allowed.effects.cut = change.after.effects.cut;
            }
            assert_eq!(
                serde_json::to_vec(&allowed).unwrap(),
                serde_json::to_vec(&change.after).unwrap(),
                "unselected scalar fields must remain byte-identical"
            );
        }
        assert_eq!(c.document(), before);
    }
    for (division, swing_frames, expected) in [
        ("Sixteenth", 1500, [870, 7620, 12120, 21120]),
        ("ThirtySecond", 750, [870, 6120, 12120, 19620]),
        ("SixtyFourth", 375, [870, 6120, 12120, 19995]),
        ("OneTwentyEighth", 188, [1058, 6120, 12120, 19620]),
    ] {
        let mut config = transform_config();
        config["fields"] =
            json!({"notes":false,"glide_targets":false,"velocity":false,"timing":true});
        config["nudge_frames"] = json!(120);
        config["swing"] = json!({"division":division,"amount_percent":25});
        let preview = transform_preview(&mut c, 1, config);
        assert_eq!(preview["data"]["summary"]["swing_frames"], swing_frames);
        let page = transform_page(&mut c, 1, &preview, 0, 64);
        let frames: Vec<_> = page["data"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|change| {
                ((change["to_row"].as_u64().unwrap() as f64
                    + change["after"]["delay"].as_f64().unwrap())
                    * 6000.0)
                    .round() as u64
            })
            .collect();
        assert_eq!(frames, expected, "division {division}");
        assert_eq!(c.document(), before);
    }
    let mut config = transform_config();
    config["fields"] = json!({"notes":false,"glide_targets":false,"velocity":false,"timing":true});
    config["nudge_frames"] = json!(120);
    config["swing"] = json!({"division":"Sixteenth","amount_percent":25});
    let preview = transform_preview(&mut c, 1, config);
    assert_eq!(transform_commit(&mut c, 1, &preview)["ok"], true);
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    let starts: Vec<_> = evaluated["data"]["manifest"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| {
            event["command"].get("On").map(|on| {
                (
                    event["frame"].as_u64().unwrap(),
                    on["pitch"].as_u64().unwrap(),
                )
            })
        })
        .collect();
    assert_eq!(starts, vec![(0, 72), (870, 60), (7620, 64), (21120, 67)]);
    assert_eq!(c.document()["data"]["song"]["bpm"], 120.0);
}

#[test]
fn transform_aliases_repeats_kit_rejection_collisions_and_outside_selection_are_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    transform_fixture(&mut c);
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                json!({"op":"upsert_pattern","id":2,"name":"Shared alias","rows":8}),
                matrix_alias(2, 1, json!({"pattern":1,"track":1})),
                json!({"op":"set_order","pattern_ids":[1,2,1]})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let mut alias = transform_config();
    alias["scope"]["pattern"] = json!(2);
    let failed = c.call(
        "leviathan_transform_preview",
        json!({"expected_revision":2,"config":alias}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("alias"));
    let preview = transform_preview(&mut c, 2, transform_config());
    assert!(!preview["data"]["summary"]["affected_aliases"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        preview["data"]["summary"]["affected_occurrences"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let mut outside = transform_config();
    outside["scope"]["row_end"] = json!(1);
    outside["fields"] = json!({"notes":false,"glide_targets":false,"velocity":false,"timing":true});
    outside["nudge_frames"] = json!(48000);
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":2,"config":outside})
        )["ok"],
        false
    );
    // Exactly at the exclusive end is out of scope, even when an adjacent f64
    // could encode the same rounded frame just below the boundary.
    outside["nudge_frames"] = json!(5250);
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":2,"config":outside})
        )["ok"],
        false
    );
    // Two selected cells mapping to one row/column are a collision, not a merge.
    let mut collision = transform_config();
    collision["scope"]["row_end"] = json!(2);
    collision["fields"] =
        json!({"notes":false,"glide_targets":false,"velocity":false,"timing":true});
    collision["nudge_frames"] = json!(5250);
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":2,"config":collision})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        transform_commit(&mut c, 2, &preview)["ok"],
        true,
        "Failed previews must preserve valid review"
    );
    let f2 = Fixture::new();
    let mut kit = Client::ready(&f2);
    kit_setup(&mut kit, &f2);
    assert_eq!(
        edit(
            &mut kit,
            2,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":36},"instrument":1,"velocity":0.5}})
            ]
        )["ok"],
        true
    );
    let before = kit.document();
    let mut config = transform_config();
    config["scope"]["row_end"] = json!(4);
    assert_eq!(
        kit.call(
            "leviathan_transform_preview",
            json!({"expected_revision":3,"config":config})
        )["ok"],
        false
    );
    assert_eq!(kit.document(), before);
    config["fields"] = json!({"notes":false,"glide_targets":false,"velocity":true,"timing":false});
    let preview = transform_preview(&mut kit, 3, config);
    assert_eq!(transform_commit(&mut kit, 3, &preview)["ok"], true);
    assert_eq!(
        kit.document()["data"]["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["note"],
        json!({"Note":36})
    );
}

#[test]
fn transform_pages_are_exact_bounded_and_selection_limits_do_not_silently_truncate() {
    use aqua_pattern_core::{
        song::{Cell, Note, PatternId, Song, TrackId},
        transforms,
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let mut song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 129;
    pattern.tracks.clear();
    for row in 0..129 {
        pattern
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(row)
            .or_default()
            .insert(
                0,
                Cell {
                    note: Note::Empty,
                    velocity: Some(0.5),
                    ..Default::default()
                },
            );
    }
    fs::write(
        f.root.join("many.leviathan.json"),
        serde_json::to_vec(&json!({"format":"leviathan-song","version":1,"song":song})).unwrap(),
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":1,"path":"many.leviathan.json","discard_unsaved":true}),
    );
    let mut config = transform_config();
    config["scope"]["row_end"] = json!(129);
    config["fields"] = json!({"notes":false,"glide_targets":false,"velocity":true,"timing":false});
    let shared =
        transforms::preview(&song, serde_json::from_value(config.clone()).unwrap()).unwrap();
    let preview = transform_preview(&mut c, 2, config.clone());
    assert_eq!(preview["data"]["changed_cell_count"], 129);
    let mut collected = Vec::new();
    for start in [0, 64, 128] {
        let page = transform_page(&mut c, 2, &preview, start, 64);
        collected.extend(page["data"]["changes"].as_array().unwrap().clone());
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
    }
    assert_eq!(
        json!(collected),
        serde_json::to_value(shared.changes()).unwrap()
    );
    let end = transform_page(&mut c, 2, &preview, 129, 64);
    assert_eq!(end["data"]["changes"], json!([]));
    assert_eq!(end["data"]["next_start"], Value::Null);
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 4097;
    for row in 129..4097 {
        pattern
            .tracks
            .get_mut(&TrackId(1))
            .unwrap()
            .entry(row)
            .or_default()
            .insert(
                0,
                Cell {
                    velocity: Some(0.5),
                    ..Default::default()
                },
            );
    }
    fs::write(
        f.root.join("too-many.leviathan.json"),
        serde_json::to_vec(&json!({"format":"leviathan-song","version":1,"song":song})).unwrap(),
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":2,"path":"too-many.leviathan.json"}),
    );
    config["scope"]["row_end"] = json!(4097);
    let base = c.document();
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":3,"config":config})
        )["ok"],
        false
    );
    assert_eq!(c.document(), base);
}

#[cfg(unix)]
#[test]
fn transform_review_reauthorizes_current_paths_without_claiming_audio_hash_verification() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let base = transform_fixture(&mut c);
    let preview = transform_preview(&mut c, 1, transform_config());
    let source = f.root.join("tone.wav");
    let outside = f.base.join("outside.wav");
    fs::rename(&source, &outside).unwrap();
    symlink(&outside, &source).unwrap();
    assert_eq!(
        c.call(
            "leviathan_transform_inspect",
            json!({"expected_revision":1,"preview_token":preview["data"]["preview_token"]})
        )["error"]["code"],
        "path_denied"
    );
    assert_eq!(
        transform_commit(&mut c, 1, &preview)["error"]["code"],
        "path_denied"
    );
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":1,"config":transform_config()})
        )["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), base);
    fs::remove_file(&source).unwrap();
    fs::rename(&outside, &source).unwrap();
    // Pure scalar transforms never claim decode/hash verification. Changing bytes is
    // not a selection/base change; export remains the actual audio verification gate.
    fs::write(&source, b"not a WAV anymore").unwrap();
    let page = transform_page(&mut c, 1, &preview, 0, 64);
    assert_eq!(page["data"]["source_identity_verified"], false);
    assert_eq!(transform_commit(&mut c, 1, &preview)["ok"], true);
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":2,"path":"bad-source.wav"})
        )["ok"],
        false
    );
    assert!(!f.root.join("bad-source.wav").exists());
    assert!(!f.root.join("bad-source.events.json").exists());
}

#[test]
fn transform_midi_bounds_unknown_inheritance_and_phrase_timing_fail_without_partial_edits() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    transform_fixture(&mut c);
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":127},"instrument":1,"effects":{"glide":{"target":0,"semitones_per_second":12}}}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let mut config = transform_config();
    config["scope"]["row_end"] = json!(1);
    config["fields"] = json!({"notes":true,"glide_targets":false,"velocity":false,"timing":false});
    config["transpose_semitones"] = json!(1);
    let failed = c.call(
        "leviathan_transform_preview",
        json!({"expected_revision":2,"config":config}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("MIDI"));
    config["fields"]["notes"] = json!(false);
    config["fields"]["glide_targets"] = json!(true);
    config["transpose_semitones"] = json!(-1);
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":2,"config":config})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"upsert_pattern","id":2,"name":"No local ownership proof","rows":8}),
                json!({"op":"set_cell","pattern_id":2,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60}}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    config = transform_config();
    config["scope"]["pattern"] = json!(2);
    assert_eq!(
        c.call(
            "leviathan_transform_preview",
            json!({"expected_revision":3,"config":config})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_phrase","id":81,"name":"Pitched phrase","instrument_id":1,"rows":4,"lines_per_beat":4,"note_columns":1,"root_note":60}),
                json!({"op":"set_phrase_cell","phrase_id":81,"row":0,"column":0,"cell":{"note":{"Note":60}}}),
                json!({"op":"set_cell","pattern_id":2,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"phrase":{"phrase":81,"cycles":1}}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    config["fields"] = json!({"notes":false,"glide_targets":false,"velocity":false,"timing":true});
    config["nudge_frames"] = json!(1);
    let failed = c.call(
        "leviathan_transform_preview",
        json!({"expected_revision":4,"config":config}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("phrase"));
    assert_eq!(c.document(), before);
}

fn generation_config() -> Value {
    json!({"version":1,"name":"Seeded Euclidean","bars":2,"instrument":1,"seed":42,
        "preset":{"Euclidean":{"steps":16,"pulses":7,"rotation":3,"probability":0.75}},
        "sequence":[{"note":60,"velocity":0.5},{"note":64,"velocity":0.75},{"note":67,"velocity":1.0}]})
}
fn generation_preview(c: &mut Client, revision: u64, config: Value) -> Value {
    c.ok(
        "leviathan_generation_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn generation_commit(c: &mut Client, revision: u64, preview: &Value) -> Value {
    c.call(
        "leviathan_generation_commit",
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}
fn generation_page(
    c: &mut Client,
    revision: u64,
    preview: &Value,
    start: usize,
    limit: usize,
) -> Value {
    c.ok("leviathan_generation_inspect",json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"],"start":start,"limit":limit}))
}

#[test]
fn generation_actual_shared_editor_and_stdio_candidate_cells_provenance_manifest_and_roundtrip_match(
) {
    use aqua_pattern_core::{editor::Editor, generation, song::Song, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let plan =
        generation::preview(&base, serde_json::from_value(generation_config()).unwrap()).unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let preview = generation_preview(&mut c, 1, generation_config());
    assert_eq!(preview["data"]["generation_preview_id"], plan.id());
    assert_eq!(
        preview["data"]["effective_config"],
        serde_json::to_value(plan.config()).unwrap()
    );
    assert_eq!(
        preview["data"]["summary"],
        serde_json::to_value(plan.summary()).unwrap()
    );
    assert_eq!(
        preview["data"]["provenance"],
        serde_json::to_value(plan.provenance()).unwrap()
    );
    assert_eq!(
        generation_page(&mut c, 1, &preview, 0, 64)["data"]["cells"],
        serde_json::to_value(plan.cells()).unwrap()
    );
    assert_eq!(c.document(), before);
    editor
        .commit_generation_preview(editor.revision(), &plan)
        .unwrap();
    assert_eq!(generation_commit(&mut c, 1, &preview)["ok"], true);
    let after = c.document();
    let actual: Song = serde_json::from_value(after["data"]["song"].clone()).unwrap();
    assert_eq!(actual, editor.song);
    assert_eq!(actual, *plan.candidate());
    for (id, pattern) in &base.patterns {
        assert_eq!(&actual.patterns[id], pattern);
    }
    for (id, track) in &base.tracks {
        assert_eq!(&actual.tracks[id], track);
    }
    assert_eq!(actual.order[..base.order.len()], base.order);
    assert_eq!(actual.bpm, base.bpm);
    assert_eq!(actual.lines_per_beat, base.lines_per_beat);
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":2}));
    assert_eq!(
        manifest["data"]["manifest"],
        serde_json::to_value(song_render::evaluate(&editor.song, None).unwrap()).unwrap()
    );
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":2,"path":"generated.wav"}),
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"generated.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"generated.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], after["data"]["song"]);
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":6,"path":"generated-reopened.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("generated.wav")).unwrap(),
        fs::read(f.root.join("generated-reopened.wav")).unwrap()
    );
    assert_eq!(
        fs::read(f.root.join("generated.events.json")).unwrap(),
        fs::read(f.root.join("generated-reopened.events.json")).unwrap()
    );
}

#[test]
fn generation_fixed_seed_replays_exactly_and_probability_gate_sequence_and_clock_are_explicit() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let first = generation_preview(&mut c, 1, generation_config());
    let first_cells = generation_page(&mut c, 1, &first, 0, 64)["data"]["cells"].clone();
    for (index, cell) in first_cells.as_array().unwrap().iter().enumerate() {
        assert_eq!(
            cell["cell"]["note"],
            json!({"Note": ([60,64,67][index%3])}),
            "Sequence advances emitted hits, not gated-out ticks"
        );
    }
    let second = generation_preview(&mut c, 1, generation_config());
    assert_ne!(
        first["data"]["preview_token"],
        second["data"]["preview_token"]
    );
    assert_eq!(
        first["data"]["generation_preview_id"],
        second["data"]["generation_preview_id"]
    );
    assert_eq!(
        first_cells,
        generation_page(&mut c, 1, &second, 0, 64)["data"]["cells"]
    );
    let f2 = Fixture::new();
    let mut other = Client::ready(&f2);
    transform_fixture(&mut other);
    let across = generation_preview(&mut other, 1, generation_config());
    assert_eq!(
        across["data"]["summary"]["raw_sha256"],
        first["data"]["summary"]["raw_sha256"]
    );
    assert_eq!(
        generation_page(&mut other, 1, &across, 0, 64)["data"]["cells"],
        first_cells
    );
    let mut config = generation_config();
    config["bars"] = json!(1);
    config["preset"] = json!({"Euclidean":{"steps":8,"pulses":8,"rotation":0,"probability":1.0}});
    let every = generation_preview(&mut c, 1, config.clone());
    let cells = generation_page(&mut c, 1, &every, 0, 64);
    assert_eq!(cells["data"]["total"], 8);
    for (index, cell) in cells["data"]["cells"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(cell["step"], index);
        assert_eq!(cell["virtual_time"], index * 1024);
        assert_eq!(cell["frame"], 48000 + index * 12000);
        assert_eq!(cell["cell"]["note"], json!({"Note":([60,64,67][index%3])}));
        assert_eq!(
            cell["cell"]["velocity"],
            json!(([0.5, 0.75, 1.0][index % 3]))
        );
        assert_eq!(cell["cell"]["instrument"], 1);
        assert_eq!(cell["cell"]["pan"], 0.0);
    }
    assert_eq!(every["data"]["summary"]["raw_ticks"], 8);
    assert_eq!(every["data"]["summary"]["start_frame"], 48000);
    assert_eq!(every["data"]["summary"]["end_frame"], 144000);
    config["bars"] = json!(4);
    config["preset"]["Euclidean"]["steps"] = json!(16);
    config["preset"]["Euclidean"]["pulses"] = json!(16);
    config["preset"]["Euclidean"]["probability"] = json!(0.5);
    let mut hashes = std::collections::BTreeSet::new();
    for seed in 0..8 {
        config["seed"] = json!(seed);
        let preview = generation_preview(&mut c, 1, config.clone());
        hashes.insert(
            preview["data"]["summary"]["raw_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert!(
        hashes.len() > 1,
        "Seed must actually affect the pinned gate, not be decorative metadata"
    );
    config["seed"] = json!(u64::MAX);
    let maximum = generation_preview(&mut c, 1, config);
    assert_eq!(maximum["data"]["effective_config"]["seed"], json!(u64::MAX));
    assert_eq!(c.document(), before);
}

#[test]
fn generation_version_seed_script_float_ranges_sequence_and_closed_gate_fail_strictly() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let current = generation_preview(&mut c, 1, generation_config());
    let mut invalids = Vec::new();
    for key in ["version", "seed", "preset", "sequence", "instrument"] {
        let mut x = generation_config();
        x.as_object_mut().unwrap().remove(key);
        invalids.push(x);
    }
    for (key, value) in [
        ("version", json!(0)),
        ("version", json!(2)),
        ("bars", json!(0)),
        ("bars", json!(17)),
        ("instrument", json!(999)),
        ("seed", json!(-1)),
        ("seed", json!(1.5)),
        ("code", json!("return os.execute('no')")),
        ("lua", json!("return 1")),
        ("candidate", json!({})),
    ] {
        let mut x = generation_config();
        x[key] = value;
        invalids.push(x);
    }
    for (key, value) in [
        ("steps", json!(0)),
        ("steps", json!(129)),
        ("pulses", json!(0)),
        ("pulses", json!(17)),
        ("rotation", json!(16)),
        ("probability", json!(1.0000000001)),
        ("probability", json!(-1e-50)),
        ("probability", json!(0.0)),
        ("script", json!("arbitrary")),
    ] {
        let mut x = generation_config();
        x["preset"]["Euclidean"][key] = value;
        invalids.push(x);
    }
    for value in [
        json!([]),
        json!(vec![json!({"note":60,"velocity":0.5}); 17]),
        json!([{"note":11,"velocity":0.5}]),
        json!([{"note":128,"velocity":0.5}]),
        json!([{"note":60,"velocity":1.0000000001}]),
        json!([{"note":60,"velocity":-1e-50}]),
        json!([{"note":60}]),
        json!([{"note":60,"velocity":0.5,"code":"x"}]),
    ] {
        let mut x = generation_config();
        x["sequence"] = value;
        invalids.push(x);
    }
    for config in invalids {
        let failed = c.call(
            "leviathan_generation_preview",
            json!({"expected_revision":1,"config":config}),
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        generation_page(&mut c, 1, &current, 0, 1)["ok"],
        true,
        "Failures preserve prior valid plan"
    );
    assert_eq!(generation_commit(&mut c, 1, &current)["ok"], true);
}

#[test]
fn generation_tokens_reject_superseded_cross_tool_aba_save_open_and_preserve_redo_during_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let old = generation_preview(&mut c, 1, generation_config());
    let current = generation_preview(&mut c, 1, generation_config());
    assert_eq!(
        generation_commit(&mut c, 1, &old)["error"]["code"],
        "invalid_generation_preview_token"
    );
    let transform = transform_preview(&mut c, 1, transform_config());
    assert_eq!(
        generation_commit(&mut c, 1, &transform)["error"]["code"],
        "invalid_generation_preview_token"
    );
    assert_eq!(
        transform_commit(&mut c, 1, &current)["error"]["code"],
        "invalid_transform_preview_token"
    );
    assert_eq!(edit(&mut c, 1, vec![title("temporary")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    assert_eq!(
        generation_commit(&mut c, 3, &current)["error"]["code"],
        "generation_preview_unavailable"
    );
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    let current = generation_preview(&mut c, 3, generation_config());
    generation_page(&mut c, 3, &current, 0, 64);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
    for (start, limit) in [(u64::MAX, 1), (0, 0), (0, 65)] {
        assert_eq!(c.call("leviathan_generation_inspect",json!({"expected_revision":3,"preview_token":current["data"]["preview_token"],"start":start,"limit":limit}))["ok"],false);
    }
    c.ok("leviathan_history_redo", json!({"expected_revision":3}));
    assert_eq!(
        generation_commit(&mut c, 4, &current)["error"]["code"],
        "generation_preview_unavailable"
    );
    let preview = generation_preview(&mut c, 4, generation_config());
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"generation-base.leviathan.json"}),
    );
    assert_eq!(
        generation_commit(&mut c, 5, &preview)["error"]["code"],
        "generation_preview_unavailable"
    );
    let preview = generation_preview(&mut c, 5, generation_config());
    let song = c.document()["data"]["song"].clone();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"generation-base.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], song);
    assert_eq!(
        generation_commit(&mut c, 6, &preview)["error"]["code"],
        "generation_preview_unavailable"
    );
}

#[test]
fn generation_pages_limits_and_checked_fresh_id_allocation_are_atomic_and_inspectable() {
    use aqua_pattern_core::{generation, song::Song};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let mut config = generation_config();
    config["bars"] = json!(4);
    config["preset"] =
        json!({"Euclidean":{"steps":128,"pulses":128,"rotation":0,"probability":1.0}});
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let shared =
        generation::preview(&base, serde_json::from_value(config.clone()).unwrap()).unwrap();
    let preview = generation_preview(&mut c, 1, config.clone());
    assert_eq!(preview["data"]["generated_cell_count"], 512);
    let next_track_id = base.tracks.keys().map(|id| id.0).max().unwrap() + 1;
    assert_eq!(preview["data"]["summary"]["track"], next_track_id);
    assert_eq!(preview["data"]["summary"]["pattern"], 2);
    let mut cells = Vec::new();
    for start in (0..512).step_by(64) {
        let page = generation_page(&mut c, 1, &preview, start, 64);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        cells.extend(page["data"]["cells"].as_array().unwrap().clone());
    }
    assert_eq!(json!(cells), serde_json::to_value(shared.cells()).unwrap());
    let end = generation_page(&mut c, 1, &preview, 512, 64);
    assert_eq!(end["data"]["cells"], json!([]));
    assert_eq!(end["data"]["next_start"], Value::Null);
    config["bars"] = json!(16);
    let failed = c.call(
        "leviathan_generation_preview",
        json!({"expected_revision":1,"config":config}),
    );
    assert_eq!(
        failed["ok"], false,
        "Existing starts plus2048 generated starts must exceed shared aggregate budget"
    );
    assert_eq!(c.document(), before);
    assert_eq!(generation_page(&mut c, 1, &preview, 0, 1)["ok"], true);
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                json!({"op":"upsert_track","id":u64::MAX,"name":"Last track ID","note_columns":1})
            ]
        )["ok"],
        true
    );
    let base = c.document();
    let failed = c.call(
        "leviathan_generation_preview",
        json!({"expected_revision":2,"config":generation_config()}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"].as_str().unwrap().contains("ID"));
    assert_eq!(c.document(), base);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"remove_track","id":u64::MAX}),
                json!({"op":"upsert_pattern","id":u64::MAX,"name":"Last pattern ID","rows":4})
            ]
        )["ok"],
        true
    );
    let base = c.document();
    assert_eq!(
        c.call(
            "leviathan_generation_preview",
            json!({"expected_revision":3,"config":generation_config()})
        )["ok"],
        false
    );
    assert_eq!(c.document(), base);
}

#[test]
fn generated_content_is_ordinary_editable_and_historical_provenance_matches_without_live_references(
) {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    transform_fixture(&mut c);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"generation"}))["data"]["patterns"],
        json!([])
    );
    let mut config = generation_config();
    config["bars"] = json!(1);
    config["preset"] = json!({"Euclidean":{"steps":4,"pulses":4,"rotation":0,"probability":1.0}});
    let preview = generation_preview(&mut c, 1, config);
    let cell = generation_page(&mut c, 1, &preview, 0, 1)["data"]["cells"][0].clone();
    assert_eq!(generation_commit(&mut c, 1, &preview)["ok"], true);
    let read = c.ok("leviathan_project_read", json!({"section":"generation"}));
    assert_eq!(read["data"]["patterns"][0]["content_matches"], true);
    let provenance = read["data"]["patterns"][0]["provenance"].clone();
    let pattern = preview["data"]["summary"]["pattern"].clone();
    let track = preview["data"]["summary"]["track"].clone();
    let rows = preview["data"]["summary"]["rows"].clone();
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"upsert_pattern","id":pattern,"name":"Renamed ordinary output","rows":rows})
            ]
        )["ok"],
        true
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"generation"}))["data"]["patterns"][0]
            ["content_matches"],
        true,
        "Name is not content identity"
    );
    let mut changed = cell["cell"].clone();
    changed["velocity"] = json!(0.25);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"set_cell","pattern_id":pattern,"track_id":track,"row":cell["row"],"column":cell["column"],"cell":changed})
            ]
        )["ok"],
        true
    );
    let read = c.ok("leviathan_project_read", json!({"section":"generation"}));
    assert_eq!(read["data"]["patterns"][0]["content_matches"], false);
    assert_eq!(read["data"]["patterns"][0]["provenance"], provenance);
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"generation"}))["data"]["patterns"][0]
            ["content_matches"],
        true
    );
    // Explicit clearing removes actual cell references; historical config instrument
    // does not become a new live dependency or regenerate removed notes.
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![
                json!({"op":"clear_pattern_track","pattern_id":1,"track_id":1}),
                json!({"op":"clear_pattern_track","pattern_id":pattern,"track_id":track}),
                json!({"op":"remove_instrument","id":1})
            ]
        )["ok"],
        true
    );
    let read = c.ok("leviathan_project_read", json!({"section":"generation"}));
    assert_eq!(read["data"]["patterns"][0]["content_matches"], false);
    assert_eq!(read["data"]["patterns"][0]["provenance"], provenance);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"historical.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"historical.leviathan.json"}),
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"generation"}))["data"],
        read["data"]
    );
}

#[test]
fn generation_kit_selectors_are_validated_even_when_gated_out_and_existing_modes_render_normally() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    let mut config = generation_config();
    config["bars"] = json!(1);
    config["preset"] = json!({"Euclidean":{"steps":4,"pulses":4,"rotation":0,"probability":1.0}});
    config["sequence"] = json!([{"note":36,"velocity":1.0},{"note":38,"velocity":0.0}]);
    let preview = generation_preview(&mut c, 2, config.clone());
    assert_eq!(generation_commit(&mut c, 2, &preview)["ok"], true);
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let starts: Vec<_> = manifest["data"]["manifest"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["command"].get("On").map(|on| on["pitch"].clone()))
        .collect();
    assert_eq!(
        starts,
        vec![json!(60); 4],
        "Kit selectors do not transpose native-root playback"
    );
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":3,"path":"kit-generated.wav"}),
    );
    let before = c.document();
    config["sequence"][1]["note"] = json!(39);
    config["preset"]["Euclidean"]["probability"] = json!(0.0);
    let failed = c.call(
        "leviathan_generation_preview",
        json!({"expected_revision":3,"config":config}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"].as_str().unwrap().contains("39"));
    assert_eq!(c.document(), before);
    config = generation_config();
    config["instrument"] = json!(2);
    let failed = c.call(
        "leviathan_generation_preview",
        json!({"expected_revision":3,"config":config}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("assigned"));
    assert_eq!(c.document(), before);
}

#[cfg(unix)]
#[test]
fn generation_review_commit_reauthorize_paths_and_never_accept_audio_hash_or_client_candidate_authority(
) {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = transform_fixture(&mut c);
    let preview = generation_preview(&mut c, 1, generation_config());
    let source = f.root.join("tone.wav");
    let outside = f.base.join("outside.wav");
    fs::rename(&source, &outside).unwrap();
    symlink(&outside, &source).unwrap();
    assert_eq!(
        c.call(
            "leviathan_generation_preview",
            json!({"expected_revision":1,"config":generation_config()})
        )["error"]["code"],
        "path_denied"
    );
    assert_eq!(
        c.call(
            "leviathan_generation_inspect",
            json!({"expected_revision":1,"preview_token":preview["data"]["preview_token"]})
        )["error"]["code"],
        "path_denied"
    );
    assert_eq!(
        generation_commit(&mut c, 1, &preview)["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), before);
    fs::remove_file(&source).unwrap();
    fs::rename(&outside, &source).unwrap();
    assert_eq!(c.call("leviathan_generation_commit",json!({"expected_revision":1,"preview_token":preview["data"]["preview_token"],"candidate":before["data"]["song"]}))["error"]["code"],"invalid_arguments");
    fs::write(&source, b"changed unused scalar source bytes").unwrap();
    assert_eq!(
        generation_page(&mut c, 1, &preview, 0, 1)["data"]["source_identity_verified"],
        false
    );
    assert_eq!(generation_commit(&mut c, 1, &preview)["ok"], true);
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":2,"path":"no-fake-audio.wav"})
        )["ok"],
        false
    );
    assert!(!f.root.join("no-fake-audio.wav").exists());
}

#[test]
fn generation_rejects_pretty_project_save_budget_before_commit_and_preserves_small_review() {
    use aqua_pattern_core::{
        project,
        song::{InstrumentId, PatternId, Song},
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"tone.wav"}),
    );
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    song.order.clear();
    song.patterns.get_mut(&PatternId(1)).unwrap().name = "x".repeat(7_900_000);
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.root.join("tone.wav"));
    let base_path = f.root.join("large-base.leviathan.json");
    project::save(&song, &base_path, project::SaveMode::CreateNew).unwrap();
    assert!(fs::metadata(&base_path).unwrap().len() < 8 * 1024 * 1024);
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":0,"path":"large-base.leviathan.json"}),
    );
    let before = c.ok("leviathan_project_read", json!({"section":"history"}));
    let mut small = generation_config();
    small["bars"] = json!(1);
    small["preset"] = json!({"Euclidean":{"steps":1,"pulses":1,"rotation":0,"probability":1.0}});
    small["sequence"] = json!([{"note":60,"velocity":0.1}]);
    let retained = generation_preview(&mut c, 1, small.clone());
    let mut large = small;
    large["bars"] = json!(16);
    large["preset"] =
        json!({"Euclidean":{"steps":128,"pulses":128,"rotation":0,"probability":1.0}});
    // Its compact Song is below MCP's8MiB cap, but the wrapped pretty project
    // would not be saveable. The shared pure persistence preflight must reject it.
    let failed = c.call(
        "leviathan_generation_preview",
        json!({"expected_revision":1,"config":large}),
    );
    assert_eq!(
        failed["ok"], false,
        "Compact-only checks accepted unsaveable generated content: {failed}"
    );
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("8 MiB"),
        "{failed}"
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        before
    );
    assert_eq!(generation_page(&mut c, 1, &retained, 0, 1)["ok"], true);
    assert_eq!(generation_commit(&mut c, 1, &retained)["ok"], true);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":2,"path":"still-saveable.leviathan.json"}),
    );
    assert!(
        fs::metadata(f.root.join("still-saveable.leviathan.json"))
            .unwrap()
            .len()
            <= 8 * 1024 * 1024
    );
}

#[test]
fn generation_projected_matrix_work_limit_is_atomic_and_undo_recovers_reviewability() {
    use aqua_pattern_core::{
        project,
        song::{InstrumentId, Occurrence, PatternId, Song, TrackId},
    };
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"tone.wav"}),
    );
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let track_template = song.tracks[&TrackId(1)].clone();
    let mut pattern_template = song.patterns[&PatternId(1)].clone();
    pattern_template.rows = 4;
    pattern_template.tracks.clear();
    pattern_template.track_aliases.clear();
    song.order.clear();
    song.patterns.clear();
    song.tracks.clear();
    song.track_order.clear();
    for id in 1..=255 {
        let mut track = track_template.clone();
        track.name = format!("Track {id}");
        track.note_columns = 1;
        song.tracks.insert(TrackId(id), track);
        song.track_order.push(TrackId(id));
    }
    song.patterns.insert(PatternId(1), pattern_template);
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: Default::default(),
            section: None,
        };
        255
    ];
    song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.root.join("tone.wav"));
    project::save(
        &song,
        &f.root.join("matrix-base.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":0,"path":"matrix-base.leviathan.json"}),
    );
    let base = c.document();
    let mut config = generation_config();
    config["bars"] = json!(1);
    config["preset"] = json!({"Euclidean":{"steps":1,"pulses":1,"rotation":0,"probability":1.0}});
    // Projected (255 occurrences +1)*(255 tracks +1)=65,536 exactly.
    let old = generation_preview(&mut c, 1, config.clone());
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![json!({"op":"upsert_track","id":256,"name":"Extra track","note_columns":1})]
        )["ok"],
        true
    );
    // The explicit edit invalidates the old token before a failed generation.
    assert_eq!(
        generation_commit(&mut c, 2, &old)["error"]["code"],
        "generation_preview_unavailable"
    );
    let before = c.ok("leviathan_project_read", json!({"section":"history"}));
    // Projected (255+1)*(256+1)=65,792: no generation work/partial allocation.
    let failed = c.call(
        "leviathan_generation_preview",
        json!({"expected_revision":2,"config":config}),
    );
    assert_eq!(failed["ok"], false, "{failed}");
    let message = failed["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("65536") || message.contains("65,536"),
        "{failed}"
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        before
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], base["data"]["song"]);
    let fresh = generation_preview(&mut c, 3, config);
    assert_eq!(
        fresh["data"]["generation_preview_id"],
        old["data"]["generation_preview_id"]
    );
    assert_ne!(fresh["data"]["preview_token"], old["data"]["preview_token"]);
    assert_eq!(generation_commit(&mut c, 3, &fresh)["ok"], true);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"bounded-matrix-generation.leviathan.json"}),
    );
}

#[test]
fn generation_ordered_empty_rows_and_default_cells_count_toward_shared_visit_budgets() {
    use aqua_pattern_core::{
        project,
        song::{Cell, InstrumentId, Occurrence, PatternId, Song, TrackId},
    };
    for rows_case in [true, false] {
        let f = Fixture::new();
        let mut c = Client::ready(&f);
        c.ok(
            "leviathan_sample_generate",
            json!({"expected_revision":0,"path":"tone.wav"}),
        );
        let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
        song.bpm = 400.0;
        song.lines_per_beat = 256;
        song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 2;
        let (rows, repeats) = if rows_case { (257, 255) } else { (255, 257) };
        let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
        pattern.rows = rows;
        pattern.tracks.clear();
        for row in 0..rows {
            let columns = pattern
                .tracks
                .entry(TrackId(1))
                .or_default()
                .entry(row)
                .or_default();
            if !rows_case {
                columns.insert(0, Cell::default());
            }
        }
        song.order = vec![
            Occurrence {
                pattern: PatternId(1),
                muted_tracks: Default::default(),
                section: None,
            };
            repeats
        ];
        song.instruments.get_mut(&InstrumentId(1)).unwrap().sample = Some(f.root.join("tone.wav"));
        project::save(
            &song,
            &f.root.join("visits-base.leviathan.json"),
            project::SaveMode::CreateNew,
        )
        .unwrap();
        c.ok(
            "leviathan_project_open",
            json!({"expected_revision":0,"path":"visits-base.leviathan.json"}),
        );
        let base = c.document();
        let mut config = generation_config();
        config["bars"] = json!(1);
        config["preset"] =
            json!({"Euclidean":{"steps":1,"pulses":1,"rotation":0,"probability":1.0}});
        //65535 visited stored rows/cells plus one actual generated row/cell fits.
        let reviewed = generation_preview(&mut c, 1, config.clone());
        let operation = if rows_case {
            json!({"op":"append_repeat","pattern_id":1})
        } else {
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":1,"cell":{"note":"Empty","pan":0.0}})
        };
        assert_eq!(edit(&mut c, 1, vec![operation])["ok"], true);
        let before = c.ok("leviathan_project_read", json!({"section":"history"}));
        let failed = c.call(
            "leviathan_generation_preview",
            json!({"expected_revision":2,"config":config}),
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains(if rows_case {
                    "row-container"
                } else {
                    "cell visits"
                }),
            "{failed}"
        );
        assert_eq!(
            c.ok("leviathan_project_read", json!({"section":"history"})),
            before
        );
        assert_eq!(
            generation_commit(&mut c, 2, &reviewed)["error"]["code"],
            "generation_preview_unavailable"
        );
        c.ok("leviathan_history_undo", json!({"expected_revision":2}));
        assert_eq!(c.document()["data"]["song"], base["data"]["song"]);
        let fresh = generation_preview(&mut c, 3, config);
        assert_eq!(generation_commit(&mut c, 3, &fresh)["ok"], true);
    }
}

fn region_playback(envelope: Value, loop_: Value) -> Value {
    json!({"op":"set_region_playback","region_id":21,"amp_envelope":envelope,"forward_loop":loop_})
}
fn sampler_envelope(attack: u16, decay: u16, sustain: u8, release: u16) -> Value {
    json!({"attack_ms":attack,"decay_ms":decay,"sustain_percent":sustain,"release_ms":release})
}
fn sampler_setup(c: &mut Client, f: &Fixture) {
    kit_source(f);
    import_source(c, 0, 7, "kit.wav");
    let result = edit(
        c,
        1,
        vec![
            region_edit(21, 0, 48000),
            json!({"op":"assign_region","instrument_id":1,"region_id":21}),
            json!({"op":"set_song","bpm":120,"lines_per_beat":4}),
            json!({"op":"upsert_pattern","id":1,"name":"Sampler envelope","rows":4}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":1.0,"pan":0.0}}),
            kit_cell(1, 1, json!("Off"), 1),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
}

#[test]
fn sampler_options_match_editor_manifest_pcm_one_undo_and_save_reopen() {
    use aqua_pattern_core::{editor::Editor, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let source_bytes = fs::read(f.root.join("kit.wav")).unwrap();
    let base = c.document()["data"]["song"].clone();
    let mut editor = Editor::default();
    editor
        .replace_document(serde_json::from_value(base.clone()).unwrap())
        .unwrap();
    editor.edit_mode = true;
    let envelope = AmpEnvelope {
        attack_ms: 10,
        decay_ms: 20,
        sustain_percent: 25,
        release_ms: 30,
    };
    let loop_ = ForwardLoop {
        start_frame: 48,
        end_frame: 192,
    };
    editor
        .set_region_playback(RegionId(21), Some(envelope), Some(loop_))
        .unwrap();
    let applied = edit(
        &mut c,
        2,
        vec![region_playback(json!(envelope), json!(loop_))],
    );
    assert_eq!(applied["ok"], true, "{applied}");
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    assert_eq!(song, editor.song);
    let regions = c.ok("leviathan_project_read", json!({"section":"regions"}));
    assert_eq!(
        regions["data"]["regions"]["21"]["amp_envelope"],
        json!(envelope)
    );
    let source = c.ok(
        "leviathan_sample_inspect",
        json!({"expected_revision":3,"sample_id":7}),
    );
    assert_eq!(
        source["data"]["regions"]["21"]["forward_loop"],
        json!(loop_)
    );
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    assert_eq!(evaluated["data"]["manifest"]["schema"], 8);
    assert_eq!(evaluated["data"]["source_identity_verified"], false);
    assert_eq!(
        evaluated["data"]["manifest"],
        json!(song_render::evaluate(&editor.song, None).unwrap())
    );
    let pcm = kit_export(&mut c, &f, 3, "sampler.wav");
    // Independent linear output-clock oracle, not source-rate/BPM ADSR.
    for frame in 0..pcm.len() / 2 {
        let level = if frame < 480 {
            frame as f64 / 480.0
        } else if frame < 1440 {
            1.0 - 0.75 * (frame - 480) as f64 / 960.0
        } else if frame < 6000 {
            0.25
        } else if frame < 7440 {
            0.25 * (1.0 - (frame - 6000) as f64 / 1440.0)
        } else {
            0.0
        };
        for channel in 0..2 {
            assert!(
                (pcm[frame * 2 + channel] - (0.1 * level) as f32).abs() < 1e-7,
                "frame {frame}: {} vs {}",
                pcm[frame * 2 + channel],
                0.1 * level
            );
        }
    }
    song_render::export(&editor.song, None, &f.root.join("shared.wav")).unwrap();
    assert_eq!(
        fs::read(f.root.join("sampler.wav")).unwrap(),
        fs::read(f.root.join("shared.wav")).unwrap()
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], base);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"sampler.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":6,"path":"sampler.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    kit_export(&mut c, &f, 7, "reopened.wav");
    for suffix in ["wav", "events.json"] {
        assert_eq!(
            fs::read(f.root.join(format!("sampler.{suffix}"))).unwrap(),
            fs::read(f.root.join(format!("reopened.{suffix}"))).unwrap()
        );
    }
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), source_bytes);
}

#[test]
fn sampler_schema_required_nullable_integer_units_and_invalid_options_are_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let catalog = c.request("tools/list", json!({}));
    let batch = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "leviathan_edit_batch")
        .unwrap();
    let schema = batch["inputSchema"]["properties"]["operations"]["items"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["properties"]["op"]["const"] == "set_region_playback")
        .unwrap();
    assert_eq!(
        schema["required"],
        json!(["op", "region_id", "amp_envelope", "forward_loop"])
    );
    for field in ["amp_envelope", "forward_loop"] {
        assert_eq!(
            schema["properties"][field]["oneOf"][1]["additionalProperties"],
            false
        );
    }
    let valid = region_playback(
        sampler_envelope(1, 2, 3, 4),
        json!({"start_frame":10,"end_frame":12}),
    );
    let before = c.document();
    let mut invalid = Vec::new();
    for field in ["region_id", "amp_envelope", "forward_loop"] {
        let mut op = valid.clone();
        op.as_object_mut().unwrap().remove(field);
        invalid.push(op);
    }
    for (field, value) in [
        ("attack_ms", json!(10001)),
        ("decay_ms", json!(10001)),
        ("sustain_percent", json!(101)),
        ("release_ms", json!(501)),
        ("release_ms", json!(0.5)),
        ("attack_ms", json!(-1)),
        ("sustain_percent", Value::Null),
    ] {
        let mut op = valid.clone();
        op["amp_envelope"][field] = value;
        invalid.push(op);
    }
    for field in ["attack_ms", "decay_ms", "sustain_percent", "release_ms"] {
        let mut op = valid.clone();
        op["amp_envelope"].as_object_mut().unwrap().remove(field);
        invalid.push(op);
    }
    for loop_ in [
        json!({"start_frame":10,"end_frame":10}),
        json!({"start_frame":10,"end_frame":11}),
        json!({"start_frame":10,"end_frame":9}),
        json!({"start_frame":0,"end_frame":48001}),
        json!({"start_frame":u64::MAX,"end_frame":u64::MAX}),
        json!({"start_frame":-1,"end_frame":10}),
        json!({"start_frame":0.5,"end_frame":10}),
        json!({"start_frame":10}),
        json!({"start_frame":0,"end_frame":10,"ping_pong":true}),
    ] {
        let mut op = valid.clone();
        op["forward_loop"] = loop_;
        invalid.push(op);
    }
    let mut unknown = valid.clone();
    unknown["amp_envelope"]["curve"] = json!("exponential");
    invalid.push(unknown);
    let mut unknown_id = valid.clone();
    unknown_id["region_id"] = json!(99);
    invalid.push(unknown_id);
    let mut bypass = region_edit(21, 0, 48000);
    bypass["forward_loop"] = json!({"start_frame":10,"end_frame":12});
    invalid.push(bypass);
    for op in invalid {
        let failed = edit(&mut c, 2, vec![title("must roll back"), op.clone()]);
        assert_eq!(failed["ok"], false, "accepted {op}: {failed}");
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![region_playback(
                sampler_envelope(10000, 10000, 100, 500),
                json!({"start_frame":0,"end_frame":2})
            )]
        )["ok"],
        true
    );
    let capabilities = c.ok("leviathan_capabilities", json!({}));
    assert_eq!(capabilities["data"]["sampler"]["manifest_schema"], 8);
    assert!(capabilities["data"]["sampler"]["legacy"]
        .as_str()
        .unwrap()
        .contains("None"));
}

#[test]
fn sampler_old_upsert_preserves_options_invalid_shrink_rolls_back_and_clear_restores_legacy() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let legacy = c.document()["data"]["song"].clone();
    for field in ["amp_envelope", "forward_loop"] {
        assert!(legacy["regions"]["21"].get(field).is_none());
    }
    let legacy_manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":2}))["data"]["manifest"].clone();
    kit_export(&mut c, &f, 2, "legacy.wav");
    assert_eq!(
        edit(&mut c, 2, vec![region_playback(Value::Null, Value::Null)])["revision"],
        2
    );
    let options = region_playback(
        sampler_envelope(0, 0, 100, 50),
        json!({"start_frame":100,"end_frame":200}),
    );
    assert_eq!(edit(&mut c, 2, vec![options])["revision"], 3);
    let mut rename = region_edit(21, 0, 48000);
    rename["name"] = json!("Renamed, options retained");
    rename["root_note"] = json!(61);
    assert_eq!(edit(&mut c, 3, vec![rename])["ok"], true);
    let configured = c.document();
    assert_eq!(
        configured["data"]["song"]["regions"]["21"]["amp_envelope"],
        sampler_envelope(0, 0, 100, 50)
    );
    assert_eq!(
        configured["data"]["song"]["regions"]["21"]["forward_loop"],
        json!({"start_frame":100,"end_frame":200})
    );
    for bad in [region_edit(21, 101, 48000), region_edit(21, 0, 199)] {
        assert_eq!(edit(&mut c, 4, vec![title("rollback"), bad])["ok"], false);
        assert_eq!(c.document(), configured);
    }
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                region_playback(Value::Null, Value::Null),
                region_edit(21, 0, 99),
                json!({"op":"remove_region","id":21})
            ]
        )["ok"],
        false
    );
    assert_eq!(c.document(), configured);
    // Explicit clearing followed by an otherwise-invalid shrink is one ordered transaction.
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                region_playback(Value::Null, Value::Null),
                region_edit(21, 0, 99)
            ]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], configured["data"]["song"]);
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![region_playback(
                sampler_envelope(0, 0, 100, 50),
                json!({"start_frame":100,"end_frame":200})
            )]
        )["revision"],
        6
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history,
        "no-op preserves redo"
    );
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![
                region_playback(Value::Null, Value::Null),
                region_edit(21, 0, 48000)
            ]
        )["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"], legacy);
    let cleared_manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":7}))["data"]["manifest"].clone();
    assert_eq!(cleared_manifest, legacy_manifest);
    kit_export(&mut c, &f, 7, "cleared.wav");
    assert_eq!(
        fs::read(f.root.join("legacy.wav")).unwrap(),
        fs::read(f.root.join("cleared.wav")).unwrap()
    );
}

#[test]
fn sampler_loop_controls_fail_before_decode_or_outputs_but_envelope_only_keeps_effects() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let source_bytes = fs::read(f.root.join("kit.wav")).unwrap();
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![region_playback(
                sampler_envelope(0, 0, 100, 0),
                json!({"start_frame":0,"end_frame":128})
            )]
        )["ok"],
        true
    );
    let mut revision = 3;
    for effects in [
        json!({"offset":0}),
        json!({"direction":"Reverse"}),
        json!({"offset":10,"direction":"Forward"}),
    ] {
        assert_eq!(
            edit(
                &mut c,
                revision,
                vec![
                    json!({"op":"set_effects","pattern_id":1,"track_id":1,"row":0,"column":0,"effects":effects})
                ]
            )["ok"],
            true
        );
        revision += 1;
        let before = c.document();
        // Existing, root-authorized regular file has changed bytes. Unsupported loop controls
        // must be reported by shared event validation before the decoder/hash gate.
        fs::write(f.root.join("kit.wav"), b"not audio").unwrap();
        for tool in ["leviathan_song_evaluate", "leviathan_song_export"] {
            let args = if tool.ends_with("export") {
                json!({"expected_revision":revision,"path":"invalid.wav"})
            } else {
                json!({"expected_revision":revision})
            };
            let failed = c.call(tool, args);
            assert_eq!(failed["ok"], false);
            assert!(
                failed["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("forward loop"),
                "{failed}"
            );
        }
        assert!(!f.root.join("invalid.wav").exists());
        assert!(!f.root.join("invalid.events.json").exists());
        assert_eq!(c.document(), before);
        fs::write(f.root.join("kit.wav"), &source_bytes).unwrap();
    }
    assert_eq!(
        edit(
            &mut c,
            revision,
            vec![region_playback(sampler_envelope(0, 0, 100, 0), Value::Null)]
        )["ok"],
        true
    );
    revision += 1;
    c.ok(
        "leviathan_song_evaluate",
        json!({"expected_revision":revision}),
    );
    kit_export(&mut c, &f, revision, "envelope-effects.wav");
}

#[test]
fn sampler_forward_loop_prefix_wrap_and_release_zero_use_source_frames_not_song_tempo() {
    let f = Fixture::new();
    let mut writer = hound::WavWriter::create(
        f.root.join("loop.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..32 {
        writer.write_sample(frame as f32 / 100.0).unwrap();
    }
    writer.finalize().unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "loop.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 4, 24),
                region_playback(
                    sampler_envelope(0, 0, 100, 0),
                    json!({"start_frame":8,"end_frame":16})
                ),
                json!({"op":"assign_region","instrument_id":1,"region_id":21}),
                json!({"op":"set_song","bpm":120,"lines_per_beat":4}),
                json!({"op":"upsert_pattern","id":1,"name":"Loop","rows":4}),
                kit_cell(1, 0, json!({"Note":60}), 1),
                kit_cell(1, 1, json!("Off"), 1)
            ]
        )["ok"],
        true
    );
    let pcm = kit_export(&mut c, &f, 2, "loop.wav.export.wav");
    for frame in 0..pcm.len() / 2 {
        let source = if frame < 12 {
            4 + frame
        } else {
            8 + (frame - 12) % 8
        };
        let expected = if frame < 6000 {
            source as f32 / 200.0
        } else {
            0.0
        };
        for ch in 0..2 {
            assert!(
                (pcm[frame * 2 + ch] - expected).abs() < 1e-7,
                "frame {frame}: {} vs {expected}",
                pcm[frame * 2 + ch]
            );
        }
    }
    assert_eq!(
        edit(&mut c, 2, vec![json!({"op":"set_song","bpm":240})])["ok"],
        true
    );
    let faster = kit_export(&mut c, &f, 3, "fast.wav");
    assert_eq!(&pcm[..6000], &faster[..6000]);
    assert!(faster[6000..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn sampler_kit_shared_region_oneshot_gate_choke_release_and_zero_gain_are_real_pcm() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    let mut first = kit_pad(301, 0, 36);
    first["choke_group"] = json!(1);
    let mut second = kit_pad(302, 15, 38);
    second["choke_group"] = json!(1);
    second["gain"] = json!(0.0);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                first.clone(),
                second,
                region_playback(
                    sampler_envelope(0, 0, 100, 500),
                    json!({"start_frame":8,"end_frame":20})
                ),
                kit_cell(1, 0, json!({"Note":36}), 1),
                kit_cell(1, 1, json!("Off"), 1),
                kit_cell(1, 2, json!({"Note":38}), 1)
            ]
        )["ok"],
        true
    );
    let manifest = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    assert_eq!(manifest["data"]["manifest"]["schema"], 8);
    for voice in manifest["data"]["manifest"]["pad_voices"]
        .as_object()
        .unwrap()
        .values()
    {
        assert_eq!(voice["region"]["amp_envelope"]["release_ms"], 500);
        assert_eq!(
            voice["region"]["forward_loop"],
            json!({"start_frame":8,"end_frame":20})
        );
    }
    let pcm = kit_export(&mut c, &f, 3, "oneshot-choke.wav");
    for frame in 0..pcm.len() / 2 {
        let expected = if frame < 12000 {
            0.1
        } else if frame < 14400 {
            0.1 * (1.0 - (frame - 12000) as f64 / 2400.0)
        } else {
            0.0
        };
        assert!(
            (pcm[frame * 2] - expected as f32).abs() < 1e-7,
            "frame {frame}: {} vs {expected}",
            pcm[frame * 2]
        );
    }
    first["mode"] = json!("Gated");
    assert_eq!(edit(&mut c, 3, vec![first])["ok"], true);
    let pcm = kit_export(&mut c, &f, 4, "gated-existing-release.wav");
    for frame in 0..pcm.len() / 2 {
        let expected = if frame < 6000 {
            0.1
        } else if frame < 30000 {
            0.1 * (1.0 - (frame - 6000) as f64 / 24000.0)
        } else {
            0.0
        };
        assert!(
            (pcm[frame * 2] - expected as f32).abs() < 1e-7,
            "frame {frame}: {} vs {expected}",
            pcm[frame * 2]
        );
    }
}

#[cfg(unix)]
#[test]
fn sampler_option_edits_reauthorize_source_paths_and_export_still_verifies_identity() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let before = c.document();
    let source = f.root.join("kit.wav");
    let bytes = fs::read(&source).unwrap();
    let outside = f.base.join("outside.wav");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(&outside, &source).unwrap();
    let op = region_playback(
        sampler_envelope(0, 0, 100, 0),
        json!({"start_frame":0,"end_frame":2}),
    );
    let failed = edit(&mut c, 2, vec![op.clone()]);
    assert_eq!(failed["error"]["code"], "path_denied");
    assert_eq!(c.document(), before);
    fs::remove_file(&source).unwrap();
    fs::write(&source, &bytes).unwrap();
    assert_eq!(
        edit(&mut c, 1, vec![op.clone()])["error"]["code"],
        "revision_conflict"
    );
    fs::write(&source, b"changed regular file").unwrap();
    assert_eq!(
        edit(&mut c, 2, vec![op])["ok"],
        true,
        "metadata edits authorize paths but do not claim byte verification"
    );
    let state = c.document();
    let failure = c.call(
        "leviathan_song_export",
        json!({"expected_revision":3,"path":"tampered.wav"}),
    );
    assert_eq!(failure["ok"], false);
    assert!(!f.root.join("tampered.wav").exists());
    assert!(!f.root.join("tampered.events.json").exists());
    assert_eq!(c.document(), state);
    fs::write(&source, &bytes).unwrap();
    kit_export(&mut c, &f, 3, "restored.wav");
    assert_eq!(fs::read(outside).unwrap(), bytes);
}

fn pad_sources(region: Value, zones: Value) -> Value {
    json!({"op":"set_pad_sources","kit_id":41,"pad_id":301,"region_id":region,"velocity_zones":zones})
}
fn velocity_zones() -> Value {
    json!([
        {"min_velocity":0,"max_velocity":63,"alternatives":[{"id":11,"region":21},{"id":12,"region":22}]},
        {"min_velocity":64,"max_velocity":127,"alternatives":[{"id":21,"region":23},{"id":22,"region":24}]}
    ])
}
fn zone_setup(c: &mut Client, f: &Fixture) {
    let mut writer = hound::WavWriter::create(
        f.root.join("layers.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for segment in 1..=4 {
        for _ in 0..48 {
            writer.write_sample(segment as f32 * 0.2).unwrap();
        }
    }
    writer.finalize().unwrap();
    import_source(c, 0, 7, "layers.wav");
    let mut ops = vec![kit_metadata(41, "Layered drums")];
    for segment in 0..4 {
        ops.push(region_edit(21 + segment, segment * 48, (segment + 1) * 48));
    }
    ops.extend([
        kit_pad(301, 0, 36),
        json!({"op":"assign_kit","instrument_id":1,"kit_id":41}),
        json!({"op":"set_song","bpm":120,"lines_per_beat":4}),
        json!({"op":"upsert_pattern","id":1,"name":"Velocity zones","rows":8}),
    ]);
    assert_eq!(edit(c, 1, ops)["ok"], true);
}
fn zone_cell(track: u64, row: u32, instrument: u64, velocity: f64) -> Value {
    json!({"op":"set_cell","pattern_id":1,"track_id":track,"row":row,"column":0,"cell":{"note":{"Note":36},"instrument":instrument,"velocity":velocity}})
}
fn layer_ons(manifest: &Value) -> Vec<(u64, Value)> {
    manifest["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| {
            event["command"].get("On").map(|on| {
                let voice = on["voice"].as_u64().unwrap().to_string();
                (
                    event["frame"].as_u64().unwrap(),
                    manifest["pad_voices"][voice]["layer_selection"].clone(),
                )
            })
        })
        .collect()
}

#[test]
fn pad_sources_match_editor_and_render_deterministic_velocity_rr_with_inspectable_ids() {
    use aqua_pattern_core::{editor::Editor, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let before = c.document()["data"]["song"].clone();
    let mut editor = Editor::default();
    editor
        .replace_document(serde_json::from_value(before.clone()).unwrap())
        .unwrap();
    editor.edit_mode = true;
    editor
        .set_pad_sources(
            DrumKitId(41),
            PadId(301),
            None,
            serde_json::from_value(velocity_zones()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        edit(&mut c, 2, vec![pad_sources(Value::Null, velocity_zones())])["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let metadata = c.ok("leviathan_project_read", json!({"section":"drum_kits"}));
    assert_eq!(
        metadata["data"]["drum_kits"]["41"]["pads"]["301"]["velocity_zones"],
        velocity_zones()
    );
    let inspected = c.ok(
        "leviathan_sample_inspect",
        json!({"expected_revision":3,"sample_id":7}),
    );
    let mappings = inspected["data"]["kit_pad_mappings"].as_array().unwrap();
    assert_eq!(mappings.len(), 4);
    for (index, (region, layer)) in [(21, 11), (22, 12), (23, 21), (24, 22)]
        .into_iter()
        .enumerate()
    {
        assert_eq!(mappings[index]["region_id"], region);
        assert_eq!(mappings[index]["layer_id"], layer);
        assert_eq!(mappings[index]["zone_index"], index / 2);
        assert_eq!(mappings[index]["alternative_index"], index % 2);
    }
    let mut concatenated = Vec::new();
    for start in 0..4 {
        let page=c.ok("leviathan_sample_inspect",json!({"expected_revision":3,"sample_id":7,"regions_start":4,"regions_limit":1,"mappings_start":start,"mappings_limit":1}));
        assert!(page["data"]["regions"].as_object().unwrap().is_empty());
        concatenated.extend(page["data"]["kit_pad_mappings"].as_array().unwrap().clone());
    }
    assert_eq!(&concatenated, mappings);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let values = [0.25, 0.75, 0.25, 0.75, 0.0, 0.25];
    let ops = values
        .iter()
        .enumerate()
        .map(|(row, value)| zone_cell(1, row as u32, 1, *value))
        .collect();
    assert_eq!(edit(&mut c, 5, ops)["ok"], true);
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":6}));
    let manifest = &evaluated["data"]["manifest"];
    assert_eq!(
        *manifest,
        json!(song_render::evaluate(&song, None).unwrap())
    );
    let ons = layer_ons(manifest);
    assert_eq!(ons.len(), 6);
    for (index, (_, selection)) in ons.iter().enumerate() {
        assert_eq!(selection["layer"], [11, 21, 12, 22, 11, 12][index]);
        assert_eq!(selection["velocity_bin"], [32, 95, 32, 95, 0, 32][index]);
        assert_eq!(selection["input_velocity"], values[index]);
    }
    let pcm = kit_export(&mut c, &f, 6, "layered.wav");
    for (index, (frame, _)) in ons.iter().enumerate() {
        let expected = [0.2, 0.6, 0.4, 0.8, 0.2, 0.4][index] * values[index] * 0.5;
        for offset in 0..48 {
            assert!((pcm[(*frame as usize + offset) * 2] - expected as f32).abs() < 1e-7);
        }
    }
    assert_eq!(
        c.ok("leviathan_song_evaluate", json!({"expected_revision":6})),
        evaluated,
        "evaluation resets selection counters"
    );
    kit_export(&mut c, &f, 6, "again.wav");
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"layered.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"layered.leviathan.json"}),
    );
    kit_export(&mut c, &f, 8, "reopened.wav");
    for suffix in ["wav", "events.json"] {
        let original = fs::read(f.root.join(format!("layered.{suffix}"))).unwrap();
        for name in ["again", "reopened"] {
            assert_eq!(
                original,
                fs::read(f.root.join(format!("{name}.{suffix}"))).unwrap()
            );
        }
    }
}

#[test]
fn pad_source_modes_are_explicit_legacy_metadata_preserves_zones_and_clear_is_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let legacy = c.document()["data"]["song"].clone();
    assert!(legacy["drum_kits"]["41"]["pads"]["301"]
        .get("velocity_zones")
        .is_none());
    assert_eq!(
        edit(&mut c, 2, vec![pad_sources(Value::Null, velocity_zones())])["ok"],
        true
    );
    let mut metadata = kit_pad(301, 0, 36);
    metadata["region_id"] = Value::Null;
    metadata["name"] = json!("Renamed without source loss");
    metadata["gain"] = json!(0.5);
    assert_eq!(edit(&mut c, 3, vec![metadata])["ok"], true);
    let before = c.document();
    assert_eq!(
        before["data"]["song"]["drum_kits"]["41"]["pads"]["301"]["velocity_zones"],
        velocity_zones()
    );
    let failed = edit(&mut c, 4, vec![title("rollback"), kit_pad(301, 0, 36)]);
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("set_pad_sources"));
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(&mut c, 4, vec![pad_sources(json!(22), json!([]))])["ok"],
        true
    );
    let pad = c.document()["data"]["song"]["drum_kits"]["41"]["pads"]["301"].clone();
    assert_eq!(pad["region"], 22);
    assert!(pad.get("velocity_zones").is_none());
    assert_eq!(
        edit(&mut c, 5, vec![pad_sources(Value::Null, json!([]))])["ok"],
        true
    );
    let cleared = c.document();
    assert!(cleared["data"]["song"]["drum_kits"]["41"]["pads"]["301"]["region"].is_null());
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    c.ok("leviathan_history_undo", json!({"expected_revision":7}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    assert_eq!(
        edit(&mut c, 8, vec![pad_sources(Value::Null, velocity_zones())])["revision"],
        8
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
    assert_eq!(
        edit(&mut c, 7, vec![pad_sources(Value::Null, json!([]))])["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn pad_zone_partition_ids_limits_and_required_fields_reject_atomically_without_hidden_sources() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let before = c.document();
    let valid = pad_sources(Value::Null, velocity_zones());
    let mut invalid = Vec::new();
    for field in ["kit_id", "pad_id", "region_id", "velocity_zones"] {
        let mut op = valid.clone();
        op.as_object_mut().unwrap().remove(field);
        invalid.push(op);
    }
    for (field, value) in [
        ("min_velocity", json!(1)),
        ("max_velocity", json!(62)),
        ("min_velocity", json!(-1)),
        ("max_velocity", json!(128)),
        ("max_velocity", json!(63.5)),
    ] {
        let mut op = valid.clone();
        op["velocity_zones"][0][field] = value;
        invalid.push(op);
    }
    let mut overlap = valid.clone();
    overlap["velocity_zones"][1]["min_velocity"] = json!(63);
    invalid.push(overlap);
    let mut reversed = valid.clone();
    reversed["velocity_zones"].as_array_mut().unwrap().reverse();
    invalid.push(reversed);
    let mut empty = valid.clone();
    empty["velocity_zones"][0]["alternatives"] = json!([]);
    invalid.push(empty);
    for (field, value) in [
        ("id", json!(0)),
        ("region", json!(999)),
        ("region", Value::Null),
        ("weight", json!(2)),
    ] {
        let mut op = valid.clone();
        op["velocity_zones"][0]["alternatives"][0][field] = value;
        invalid.push(op);
    }
    let mut duplicate = valid.clone();
    duplicate["velocity_zones"][1]["alternatives"][1]["id"] = json!(11);
    invalid.push(duplicate);
    let mut both = valid.clone();
    both["region_id"] = json!(21);
    invalid.push(both);
    let mut missing = valid.clone();
    missing["pad_id"] = json!(999);
    invalid.push(missing);
    let mut missing = valid.clone();
    missing["kit_id"] = json!(999);
    invalid.push(missing);
    let mut random = valid.clone();
    random["seed"] = json!(42);
    invalid.push(random);
    let mut bypass = kit_pad(301, 0, 36);
    bypass["velocity_zones"] = velocity_zones();
    invalid.push(bypass);
    let zones:Vec<_>=(0..8).map(|zone|json!({"min_velocity":zone*16,"max_velocity":zone*16+15,"alternatives":(0..4).map(|n|json!({"id":1+zone*4+n,"region":21})).collect::<Vec<_>>()})).collect();
    let mut too_many = zones.clone();
    too_many[0]["alternatives"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":100,"region":21}));
    invalid.push(pad_sources(Value::Null, json!(too_many)));
    let nine:Vec<_>=(0..9).map(|index|json!({"min_velocity":index,"max_velocity":if index==8 {127}else{index},"alternatives":[{"id":index+1,"region":21}]})).collect();
    invalid.push(pad_sources(Value::Null, json!(nine)));
    invalid.push(pad_sources(Value::Null,json!([{"min_velocity":0,"max_velocity":127,"alternatives":(0..9).map(|index|json!({"id":index+1,"region":21})).collect::<Vec<_>>()}])));
    for op in invalid {
        let failed = edit(&mut c, 2, vec![title("rollback"), op.clone()]);
        assert_eq!(failed["ok"], false, "{op}: {failed}");
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        edit(&mut c, 2, vec![pad_sources(Value::Null, json!(zones))])["ok"],
        true,
        "exact eight-zone/32-alternative boundary"
    );
    let full = c.document();
    assert_eq!(
        edit(&mut c, 3, vec![json!({"op":"remove_region","id":21})])["ok"],
        false
    );
    assert_eq!(c.document(), full);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":3,"path":"all32.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":4,"path":"all32.leviathan.json"}),
    );
    assert_eq!(
        c.document()["data"]["song"],
        full["data"]["song"],
        "ordinary32-alternative pad persists every ID and vector position exactly"
    );
}

#[test]
fn pad_rr_counters_are_per_instrument_zone_and_unsuppressed_on_including_retriggers() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let mut roll = zone_cell(1, 0, 1, 0.25);
    roll["cell"]["effects"] = json!({"retrigger":{"repeats":2,"spacing":0.25}});
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                pad_sources(Value::Null, velocity_zones()),
                json!({"op":"assign_kit","instrument_id":2,"kit_id":41}),
                roll,
                zone_cell(2, 0, 2, 0.25),
                zone_cell(1, 1, 1, 0.75),
                zone_cell(1, 2, 1, 0.25),
                zone_cell(1, 3, 1, 0.0),
                zone_cell(1, 4, 1, 0.25)
            ]
        )["ok"],
        true
    );
    let result = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let ons = layer_ons(&result["data"]["manifest"]);
    let layers: Vec<_> = ons
        .iter()
        .map(|(_, s)| s["layer"].as_u64().unwrap())
        .collect();
    assert_eq!(layers, vec![11, 11, 12, 11, 21, 12, 11, 12]);
    let ordinals: Vec<_> = ons
        .iter()
        .map(|(_, s)| s["round_robin_ordinal"].as_u64().unwrap())
        .collect();
    assert_eq!(ordinals, vec![0, 0, 1, 2, 0, 3, 4, 5]);
    for (_, selection) in &ons {
        assert_eq!(
            selection["round_robin_index"].as_u64().unwrap(),
            selection["round_robin_ordinal"].as_u64().unwrap() % 2
        );
    }
    // The first occurrence is suppressed entirely; it must not consume either instrument's choices.
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"append_repeat","pattern_id":1}),
                json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":1,"muted":true}),
                json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":2,"muted":true})
            ]
        )["ok"],
        true
    );
    let muted = c.ok("leviathan_song_evaluate", json!({"expected_revision":4}));
    let shifted = layer_ons(&muted["data"]["manifest"]);
    assert_eq!(shifted.len(), ons.len());
    for ((frame, selection), (old_frame, old_selection)) in shifted.iter().zip(&ons) {
        assert_eq!(*frame, *old_frame + 48000);
        assert_eq!(selection, old_selection);
    }
}

#[test]
fn pad_sources_reference_deletion_and_pre_gain_bins_remain_explicit_after_atomic_switches() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let mut pad = kit_pad(301, 0, 36);
    pad["gain"] = json!(0.01);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                pad,
                pad_sources(Value::Null, velocity_zones()),
                zone_cell(1, 0, 1, 0.5),
                zone_cell(1, 1, 1, f64::from(f32::from_bits(0.5f32.to_bits() - 1)))
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let eval = c.ok("leviathan_song_evaluate", json!({"expected_revision":3}));
    let ons = layer_ons(&eval["data"]["manifest"]);
    assert_eq!(ons[0].1["velocity_bin"], 64);
    assert_eq!(ons[1].1["velocity_bin"], 63);
    assert_eq!(ons[1].1["layer"], 11);
    assert_eq!(
        ons[0].1["layer"], 21,
        "pad gain must not change selected velocity zone"
    );
    for op in [
        json!({"op":"remove_region","id":24}),
        pad_sources(Value::Null, json!([])),
        json!({"op":"remove_sample","id":7}),
    ] {
        assert_eq!(edit(&mut c, 3, vec![title("rollback"), op])["ok"], false);
        assert_eq!(c.document(), before);
    }
    // Mode switch then an invalid reference deletion rolls back the source switch too.
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                pad_sources(json!(21), json!([])),
                json!({"op":"remove_region","id":21})
            ]
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"clear_pattern_track","pattern_id":1,"track_id":1}),
                pad_sources(Value::Null, json!([])),
                json!({"op":"remove_region","id":24})
            ]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn source_inspection_pages_bound_maximum_rr_mappings_and_keep_independent_regions() {
    use aqua_pattern_core::song::*;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let template = song.drum_kits[&DrumKitId(41)].pads[&PadId(301)].clone();
    let zones: Vec<VelocityZone> = (0..8u8)
        .map(|zone| VelocityZone {
            min_velocity: zone * 16,
            max_velocity: zone * 16 + 15,
            alternatives: (0..4u64)
                .map(|n| PadLayer {
                    id: PadLayerId(1 + u64::from(zone) * 4 + n),
                    region: RegionId(21 + n),
                })
                .collect(),
        })
        .collect();
    song.drum_kits.clear();
    for kit in 1..=128 {
        let pads = (0..16u8)
            .map(|slot| {
                let mut pad = template.clone();
                pad.slot = slot;
                pad.note = slot;
                pad.region = None;
                pad.velocity_zones = zones.clone();
                (PadId(u64::from(slot) + 1), pad)
            })
            .collect();
        song.drum_kits.insert(
            DrumKitId(kit),
            DrumKitInstrument {
                name: format!("Bank {kit}"),
                pads,
            },
        );
    }
    song.validate().unwrap();
    // Compact project pins the full valid model bound without assuming pretty-format
    // persistence overhead fits the separate 8 MiB save budget.
    let document =
        serde_json::to_vec(&json!({"format":"leviathan-song","version":1,"song":song})).unwrap();
    assert!(document.len() < 8 * 1024 * 1024);
    fs::write(f.root.join("max-mappings.leviathan.json"), document).unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":2,"path":"max-mappings.leviathan.json","discard_unsaved":true}),
    );
    let before = c.document();
    let independent=c.ok("leviathan_sample_inspect",json!({"expected_revision":3,"sample_id":7,"regions_start":1,"regions_limit":1,"mappings_start":0,"mappings_limit":1}));
    assert!(independent["data"]["regions"].get("21").is_none());
    assert_eq!(independent["data"]["kit_pad_mappings"][0]["region_id"], 21);
    for start in [0usize, 63, 127, 128, 65535, 65536] {
        let page=c.ok("leviathan_sample_inspect",json!({"expected_revision":3,"sample_id":7,"regions_start":1,"regions_limit":1,"mappings_start":start,"mappings_limit":128}));
        assert_eq!(
            page["data"]["regions_page"],
            json!({"start":1,"total":4,"next_start":2})
        );
        assert_eq!(
            page["data"]["regions"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["22"]
        );
        assert_eq!(page["data"]["mappings_page"]["total"], 65536);
        let mappings = page["data"]["kit_pad_mappings"].as_array().unwrap();
        assert_eq!(mappings.len(), (65536 - start).min(128));
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        for (offset, mapping) in mappings.iter().enumerate() {
            let index = start + offset;
            let within_pad = index % 32;
            assert_eq!(mapping["kit_id"], index / (16 * 32) + 1);
            assert_eq!(mapping["pad_id"], (index / 32) % 16 + 1);
            assert_eq!(mapping["zone_index"], within_pad / 4);
            assert_eq!(mapping["alternative_index"], within_pad % 4);
            assert_eq!(mapping["layer_id"], within_pad + 1);
            assert_eq!(
                mapping["region_id"],
                21 + within_pad % 4,
                "mapping membership is independent of paged region subset"
            );
        }
        assert_eq!(
            page["data"]["mappings_page"]["next_start"],
            if start + mappings.len() < 65536 {
                json!(start + mappings.len())
            } else {
                Value::Null
            }
        );
    }
    for args in [
        json!({"mappings_start":65537}),
        json!({"regions_start":5}),
        json!({"mappings_start":u64::MAX}),
        json!({"mappings_limit":129}),
        json!({"regions_limit":0}),
    ] {
        let mut input = json!({"expected_revision":3,"sample_id":7});
        input
            .as_object_mut()
            .unwrap()
            .extend(args.as_object().unwrap().clone());
        assert_eq!(c.call("leviathan_sample_inspect", input)["ok"], false);
    }
    assert_eq!(
        c.document(),
        before,
        "read pages/errors preserve full song and history"
    );
}

#[test]
fn pad_alternative_source_paths_hashes_and_inspection_byte_limit_keep_existing_guards() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                pad_sources(Value::Null, velocity_zones()),
                zone_cell(1, 0, 1, 0.75)
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let source = f.root.join("layers.wav");
    let bytes = fs::read(&source).unwrap();
    fs::write(&source, b"changed source identity").unwrap();
    for tool in ["leviathan_sample_inspect", "leviathan_song_export"] {
        let args = if tool.ends_with("inspect") {
            json!({"expected_revision":3,"sample_id":7,"mappings_start":1,"mappings_limit":1})
        } else {
            json!({"expected_revision":3,"path":"bad-layer.wav"})
        };
        assert_eq!(c.call(tool, args)["ok"], false);
    }
    assert!(!f.root.join("bad-layer.wav").exists());
    assert!(!f.root.join("bad-layer.events.json").exists());
    assert_eq!(c.document(), before);
    fs::write(&source, &bytes).unwrap();
    #[cfg(unix)]
    {
        let outside = f.base.join("outside.wav");
        fs::write(&outside, &bytes).unwrap();
        fs::remove_file(&source).unwrap();
        std::os::unix::fs::symlink(&outside, &source).unwrap();
        assert_eq!(
            edit(&mut c, 3, vec![pad_sources(json!(21), json!([]))])["error"]["code"],
            "path_denied"
        );
        assert_eq!(
            c.call(
                "leviathan_sample_inspect",
                json!({"expected_revision":3,"sample_id":7})
            )["error"]["code"],
            "path_denied"
        );
        assert_eq!(c.document(), before);
        fs::remove_file(&source).unwrap();
        fs::write(&source, &bytes).unwrap();
    }
    let mut large = region_edit(21, 0, 48);
    large["name"] = json!("x".repeat(256 * 1024));
    assert_eq!(edit(&mut c, 3, vec![large])["ok"], true);
    let before = c.document();
    assert_eq!(
        c.call(
            "leviathan_sample_inspect",
            json!({"expected_revision":4,"sample_id":7})
        )["error"]["code"],
        "inspection_too_large"
    );
    let smaller = c.ok(
        "leviathan_sample_inspect",
        json!({"expected_revision":4,"sample_id":7,"regions_start":1,"regions_limit":1}),
    );
    assert_eq!(
        smaller["data"]["kit_pad_mappings"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        c.call(
            "leviathan_sample_inspect",
            json!({"expected_revision":3,"sample_id":7})
        )["error"]["code"],
        "revision_conflict"
    );
}

fn groove_capture_config() -> Value {
    json!({"version":1,"name":"Accepted late feel","marker_set":31,"start_marker":0,"end_marker":4,"include_start_anchor":true,"division":"Sixteenth","cycle_steps":4})
}
fn groove_apply_config(strength: u8) -> Value {
    json!({"groove":51,"scope":{"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[0]},"strength_percent":strength})
}
fn groove_setup(c: &mut Client, f: &Fixture) {
    kit_source(f);
    import_source(c, 0, 7, "kit.wav");
    let mut ops = vec![
        marker_edit(31, json!(120), vec![0, 6300, 12600, 18300, 24000, 48000]),
        json!({"op":"upsert_instrument","id":1,"name":"Groove voice","sample":"kit.wav"}),
        json!({"op":"set_song","bpm":240,"lines_per_beat":4}),
        json!({"op":"upsert_pattern","id":1,"name":"Straight notes","rows":8}),
    ];
    for row in 0..8 {
        ops.push(json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":row,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5,"pan":0.25}}));
    }
    assert_eq!(edit(c, 1, ops)["ok"], true);
}
fn groove_capture(c: &mut Client, revision: u64, id: u64, config: Value) -> Value {
    c.ok(
        "leviathan_groove_capture_preview",
        json!({"expected_revision":revision,"groove_id":id,"config":config}),
    )
}
fn groove_apply(c: &mut Client, revision: u64, config: Value) -> Value {
    c.ok(
        "leviathan_groove_apply_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn groove_commit(c: &mut Client, revision: u64, preview: &Value, kind: &str) -> Value {
    c.call(
        &format!("leviathan_groove_{kind}_commit"),
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}
fn groove_page(
    c: &mut Client,
    revision: u64,
    preview: &Value,
    start: usize,
    limit: usize,
) -> Value {
    c.ok("leviathan_groove_preview_inspect",json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"],"start":start,"limit":limit}))
}
fn groove_capture_commit(c: &mut Client, revision: u64) -> Value {
    let preview = groove_capture(c, revision, 51, groove_capture_config());
    let applied = groove_commit(c, revision, &preview, "capture");
    assert_eq!(applied["ok"], true, "{applied}");
    applied
}

#[test]
fn groove_shared_capture_apply_editor_exact_frames_one_undo_and_save_render_roundtrip() {
    use aqua_pattern_core::{editor::Editor, groove, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    let before = c.document();
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let capture = groove::capture_preview(
        &base,
        GrooveId(51),
        serde_json::from_value(groove_capture_config()).unwrap(),
    )
    .unwrap();
    let mut editor = Editor::default();
    editor.replace_document(base.clone()).unwrap();
    editor.edit_mode = true;
    let reviewed = groove_capture(&mut c, 2, 51, groove_capture_config());
    assert_eq!(c.document(), before);
    assert_eq!(reviewed["data"]["summary"], json!(capture.summary()));
    assert_eq!(reviewed["data"]["source_identity_verified"], false);
    let page = groove_page(&mut c, 2, &reviewed, 0, 64);
    assert_eq!(
        page["data"]["records"],
        json!(groove::inspect(capture.template()).unwrap())
    );
    assert_eq!(page["data"]["template"]["source_bpm"], 120.0);
    editor
        .commit_groove_capture_preview(editor.revision(), &capture)
        .unwrap();
    assert_eq!(groove_commit(&mut c, 2, &reviewed, "capture")["ok"], true);
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"summary"}))["data"]["groove_count"],
        1
    );
    let stored = c.ok(
        "leviathan_groove_template_inspect",
        json!({"expected_revision":3,"groove_id":51}),
    );
    assert_eq!(stored["data"]["anchors"], page["data"]["records"]);
    let captured = editor.song.clone();
    let plan = groove::preview(
        &captured,
        serde_json::from_value(groove_apply_config(100)).unwrap(),
    )
    .unwrap();
    let applied = groove_apply(&mut c, 3, groove_apply_config(100));
    assert_eq!(applied["data"]["summary"], json!(plan.summary()));
    let changes = groove_page(&mut c, 3, &applied, 0, 64);
    assert_eq!(changes["data"]["records"], json!(plan.changes()));
    assert_eq!(c.document()["data"]["song"], json!(captured));
    editor
        .commit_groove_preview(editor.revision(), &plan)
        .unwrap();
    assert_eq!(groove_commit(&mut c, 3, &applied, "apply")["ok"], true);
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let evaluated = c.ok("leviathan_song_evaluate", json!({"expected_revision":4}));
    assert_eq!(
        evaluated["data"]["manifest"],
        json!(song_render::evaluate(&editor.song, None).unwrap())
    );
    let frames: Vec<_> = evaluated["data"]["manifest"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["command"].get("On").is_some())
        .map(|e| e["frame"].as_u64().unwrap())
        .collect();
    assert_eq!(
        frames,
        vec![0, 3150, 6300, 9150, 12000, 15150, 18300, 21150],
        "100% exact captured rational phase at target240BPM, source120BPM"
    );
    for row in 0..8u32 {
        let old = &captured.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0];
        let new = &editor.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&row][&0];
        let mut expected = old.clone();
        expected.delay = new.delay;
        assert_eq!(new, &expected, "only timing changes");
    }
    kit_export(&mut c, &f, 4, "groove.wav");
    song_render::export(&editor.song, None, &f.root.join("shared-groove.wav")).unwrap();
    assert_eq!(
        fs::read(f.root.join("groove.wav")).unwrap(),
        fs::read(f.root.join("shared-groove.wav")).unwrap()
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(captured));
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"groove.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"groove.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    kit_export(&mut c, &f, 8, "groove-reopened.wav");
    for suffix in ["wav", "events.json"] {
        assert_eq!(
            fs::read(f.root.join(format!("groove.{suffix}"))).unwrap(),
            fs::read(f.root.join(format!("groove-reopened.{suffix}"))).unwrap()
        );
    }
}

#[test]
fn groove_capture_schema_version_grid_windows_and_immutable_ids_fail_without_losing_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    let before = c.document();
    let reviewed = groove_capture(&mut c, 2, 51, groove_capture_config());
    let mut invalid = Vec::new();
    for (field, value) in [
        ("version", json!(2)),
        ("name", json!("   ")),
        ("marker_set", json!(99)),
        ("start_marker", json!(4)),
        ("end_marker", json!(0)),
        ("end_marker", json!(6)),
        ("include_start_anchor", json!("yes")),
        ("division", json!("Quarter")),
        ("cycle_steps", json!(0)),
        ("cycle_steps", json!(1025)),
        ("seed", json!(42)),
        ("anchors", json!([])),
    ] {
        let mut cfg = groove_capture_config();
        cfg[field] = value;
        invalid.push(cfg);
    }
    for field in [
        "version",
        "name",
        "marker_set",
        "start_marker",
        "end_marker",
        "include_start_anchor",
        "division",
        "cycle_steps",
    ] {
        let mut cfg = groove_capture_config();
        cfg.as_object_mut().unwrap().remove(field);
        invalid.push(cfg);
    }
    let mut no_anchors = groove_capture_config();
    no_anchors["end_marker"] = json!(1);
    no_anchors["include_start_anchor"] = json!(false);
    invalid.push(no_anchors);
    for config in invalid {
        let failed = c.call(
            "leviathan_groove_capture_preview",
            json!({"expected_revision":2,"groove_id":52,"config":config}),
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
    }
    assert_eq!(
        c.call(
            "leviathan_groove_capture_preview",
            json!({"expected_revision":2,"groove_id":0,"config":groove_capture_config()})
        )["ok"],
        false
    );
    assert_eq!(groove_commit(&mut c, 2, &reviewed, "capture")["ok"], true);
    let stored = c.document();
    let valid_apply = groove_apply(&mut c, 3, groove_apply_config(0));
    assert_eq!(
        c.call(
            "leviathan_groove_capture_preview",
            json!({"expected_revision":3,"groove_id":51,"config":groove_capture_config()})
        )["ok"],
        false,
        "even identical capture requires fresh ID"
    );
    assert_eq!(c.document(), stored);
    let noop = groove_commit(&mut c, 3, &valid_apply, "apply");
    assert_eq!(noop["ok"], true);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(c.document(), stored);
    let mut unknown_bpm = marker_edit(31, Value::Null, vec![0, 6300, 12600, 18300, 24000, 48000]);
    unknown_bpm["name"] = json!("No inferred BPM");
    assert_eq!(edit(&mut c, 3, vec![unknown_bpm])["ok"], true);
    assert_eq!(
        c.call(
            "leviathan_groove_capture_preview",
            json!({"expected_revision":4,"groove_id":52,"config":groove_capture_config()})
        )["ok"],
        false
    );
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![marker_edit(
                31,
                json!(120),
                vec![0, 1, 6300, 12600, 24000, 48000]
            )]
        )["ok"],
        true
    );
    assert_eq!(
        c.call(
            "leviathan_groove_capture_preview",
            json!({"expected_revision":5,"groove_id":52,"config":groove_capture_config()})
        )["ok"],
        false,
        "duplicate nearest slots never merge"
    );
}

#[test]
fn groove_shared_plan_supersession_kind_aba_save_open_and_zero_noop_preserve_history() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    let capture = groove_capture(&mut c, 2, 51, groove_capture_config());
    assert_eq!(
        groove_commit(&mut c, 2, &capture, "apply")["error"]["code"],
        "groove_preview_kind_mismatch"
    );
    let capture2 = groove_capture(&mut c, 2, 52, groove_capture_config());
    assert_eq!(
        groove_commit(&mut c, 2, &capture, "capture")["error"]["code"],
        "invalid_groove_preview_token"
    );
    assert_eq!(edit(&mut c, 2, vec![title("ABA")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(
        groove_commit(&mut c, 4, &capture2, "capture")["error"]["code"],
        "groove_preview_unavailable"
    );
    groove_capture_commit(&mut c, 4);
    let capture3 = groove_capture(&mut c, 5, 52, groove_capture_config());
    let apply = groove_apply(&mut c, 5, groove_apply_config(100));
    assert_eq!(
        groove_commit(&mut c, 5, &capture3, "capture")["error"]["code"],
        "invalid_groove_preview_token"
    );
    assert_eq!(
        groove_commit(&mut c, 5, &apply, "capture")["error"]["code"],
        "groove_preview_kind_mismatch"
    );
    assert_eq!(groove_commit(&mut c, 5, &apply, "apply")["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    let before = c.document();
    let zero = groove_apply(&mut c, 7, groove_apply_config(0));
    assert_eq!(zero["data"]["record_count"], 0);
    let empty = groove_page(&mut c, 7, &zero, 0, 1);
    assert_eq!(empty["data"]["records"], json!([]));
    let noop = groove_commit(&mut c, 7, &zero, "apply");
    assert_eq!(noop["ok"], true);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["revision"], 7);
    assert_eq!(c.document(), before);
    assert_eq!(before["data"]["session"]["can_redo"], true);
    assert_eq!(
        groove_commit(&mut c, 7, &zero, "apply")["error"]["code"],
        "groove_preview_unavailable"
    );
    let apply = groove_apply(&mut c, 7, groove_apply_config(100));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":7,"path":"groove-aba.leviathan.json"}),
    );
    assert_eq!(
        groove_commit(&mut c, 8, &apply, "apply")["error"]["code"],
        "groove_preview_unavailable"
    );
    let apply = groove_apply(&mut c, 8, groove_apply_config(100));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":8,"path":"groove-aba.leviathan.json"}),
    );
    assert_eq!(
        groove_commit(&mut c, 9, &apply, "apply")["error"]["code"],
        "groove_preview_unavailable"
    );
    let capture = groove_capture(&mut c, 9, 52, groove_capture_config());
    let wrong=c.call("leviathan_groove_capture_commit",json!({"expected_revision":9,"preview_token":capture["data"]["preview_token"],"candidate":{}}));
    assert_eq!(wrong["ok"], false);
    assert_eq!(groove_commit(&mut c, 9, &capture, "capture")["ok"], true);
}

#[test]
fn groove_frozen_recipe_survives_source_and_marker_deletion_without_reinference() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    groove_capture_commit(&mut c, 2);
    let recipe = c.ok(
        "leviathan_groove_template_inspect",
        json!({"expected_revision":3,"groove_id":51}),
    )["data"]
        .clone();
    fs::copy(f.root.join("kit.wav"), f.root.join("replacement.wav")).unwrap();
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_instrument","id":1,"name":"Current independent source","sample":"replacement.wav"}),
                json!({"op":"remove_marker_set","id":31}),
                json!({"op":"remove_sample","id":7})
            ]
        )["ok"],
        true
    );
    fs::remove_file(f.root.join("kit.wav")).unwrap();
    assert_eq!(
        c.ok(
            "leviathan_groove_template_inspect",
            json!({"expected_revision":4,"groove_id":51})
        )["data"],
        recipe
    );
    let apply = groove_apply(&mut c, 4, groove_apply_config(100));
    assert_eq!(groove_commit(&mut c, 4, &apply, "apply")["ok"], true);
    kit_export(&mut c, &f, 5, "historical-groove.wav");
    let baked = c.document()["data"]["song"]["patterns"].clone();
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![
                json!({"op":"remove_groove","id":51}),
                json!({"op":"remove_groove","id":99})
            ]
        )["ok"],
        false
    );
    assert_eq!(
        c.ok(
            "leviathan_groove_template_inspect",
            json!({"expected_revision":5,"groove_id":51})
        )["data"],
        recipe
    );
    assert_eq!(
        edit(&mut c, 5, vec![json!({"op":"remove_groove","id":51})])["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"]["patterns"], baked);
    assert!(c.document()["data"]["song"].get("grooves").is_none());
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    assert_eq!(
        c.ok(
            "leviathan_groove_template_inspect",
            json!({"expected_revision":7,"groove_id":51})
        )["data"],
        recipe
    );
}

#[test]
fn groove_strength_blends_existing_phase_is_idempotent_and_sparse_slots_are_not_invented() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    groove_capture_commit(&mut c, 2);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5,"pan":0.25,"delay":0.2}})
            ]
        )["ok"],
        true
    );
    let half = groove_apply(&mut c, 4, groove_apply_config(50));
    assert_eq!(groove_commit(&mut c, 4, &half, "apply")["ok"], true);
    let manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":5}))["data"]["manifest"].clone();
    let frames: Vec<_> = manifest["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["command"].get("On").is_some())
        .map(|e| e["frame"].as_u64().unwrap())
        .collect();
    assert_eq!(
        frames,
        vec![0, 3375, 6150, 9075, 12000, 15075, 18150, 21075],
        "blend current phase toward template, not additive offset"
    );
    let full = groove_apply(&mut c, 5, groove_apply_config(100));
    assert_eq!(groove_commit(&mut c, 5, &full, "apply")["ok"], true);
    let before = c.document();
    let again = groove_apply(&mut c, 6, groove_apply_config(100));
    assert_eq!(again["data"]["record_count"], 0);
    assert_eq!(
        groove_commit(&mut c, 6, &again, "apply")["data"]["changed"],
        false
    );
    assert_eq!(c.document(), before);
    let mut sparse = groove_capture_config();
    sparse["include_start_anchor"] = json!(false);
    let captured = groove_capture(&mut c, 6, 52, sparse);
    assert_eq!(captured["data"]["record_count"], 3);
    assert_eq!(groove_commit(&mut c, 6, &captured, "capture")["ok"], true);
    assert_eq!(
        edit(
            &mut c,
            7,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5,"pan":0.25,"delay":0.1}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let mut cfg = groove_apply_config(100);
    cfg["groove"] = json!(52);
    let sparse = groove_apply(&mut c, 8, cfg);
    assert_eq!(sparse["data"]["summary"]["unmatched_notes"], 2);
    assert_eq!(sparse["data"]["record_count"], 0);
    assert_eq!(
        groove_commit(&mut c, 8, &sparse, "apply")["data"]["changed"],
        false
    );
    assert_eq!(c.document(), before);
}

#[test]
fn groove_bad_scope_collisions_phrase_and_unrepresentable_actions_reject_without_alias_writes() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    groove_capture_commit(&mut c, 2);
    let before = c.document();
    let valid = groove_apply(&mut c, 3, groove_apply_config(100));
    let mut invalid = Vec::new();
    for (field, value) in [
        ("strength_percent", json!(101)),
        ("strength_percent", json!(-1)),
        ("strength_percent", json!(0.5)),
        ("groove", json!(99)),
        ("seed", json!(42)),
        ("candidate", json!({})),
    ] {
        let mut cfg = groove_apply_config(100);
        cfg[field] = value;
        invalid.push(cfg);
    }
    for (field, value) in [
        ("row_end", json!(9)),
        ("row_start", json!(8)),
        ("columns", json!([])),
        ("columns", json!([0, 0])),
        ("track", json!(99)),
    ] {
        let mut cfg = groove_apply_config(100);
        cfg["scope"][field] = value;
        invalid.push(cfg);
    }
    for cfg in invalid {
        assert_eq!(
            c.call(
                "leviathan_groove_apply_preview",
                json!({"expected_revision":3,"config":cfg})
            )["ok"],
            false
        );
        assert_eq!(c.document(), before);
    }
    groove_page(&mut c, 3, &valid, 0, 64);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_pattern","id":2,"name":"Alias","rows":8}),
                matrix_alias(2, 1, json!({"pattern":1,"track":1})),
                json!({"op":"append_repeat","pattern_id":2})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let mut cfg = groove_apply_config(100);
    cfg["scope"]["pattern"] = json!(2);
    assert_eq!(
        c.call(
            "leviathan_groove_apply_preview",
            json!({"expected_revision":4,"config":cfg})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    let source = groove_apply(&mut c, 4, groove_apply_config(100));
    assert_eq!(
        source["data"]["summary"]["affected_occurrences"],
        json!([0, 1])
    );
    assert!(!source["data"]["summary"]["affected_aliases"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":{"Note":60},"instrument":1,"delay":0.9}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let zero = groove_apply(&mut c, 5, groove_apply_config(0));
    assert_eq!(
        c.call(
            "leviathan_groove_apply_preview",
            json!({"expected_revision":5,"config":groove_apply_config(100)})
        )["ok"],
        false,
        "two notes map same grid slot, never merge"
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        groove_commit(&mut c, 5, &zero, "apply")["data"]["changed"],
        false
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![
                phrase_metadata(81),
                json!({"op":"set_phrase_cell","phrase_id":81,"row":0,"column":0,"cell":{"note":{"Note":60}}}),
                json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":1,"column":0,"trigger":{"phrase":81,"cycles":1}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    assert_eq!(
        c.call(
            "leviathan_groove_apply_preview",
            json!({"expected_revision":7,"config":groove_apply_config(100)})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    c.ok("leviathan_history_undo", json!({"expected_revision":7}));
    assert_eq!(
        edit(
            &mut c,
            8,
            vec![
                json!({"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":1,"muted":true}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":7,"column":0,"cell":{"note":{"Note":60},"instrument":1,"delay":0.1,"effects":{"release":0.9999999999999999}}})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let failed = c.call(
        "leviathan_groove_apply_preview",
        json!({"expected_revision":9,"config":groove_apply_config(100)}),
    );
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot preserve the exact integer displacement"),
        "{failed}"
    );
    assert_eq!(c.document(), before);
}

#[test]
fn groove_anchor_and_diff_pages_are_exact_bounded_and_templates_enforce_capacity() {
    use aqua_pattern_core::{groove, song::*};
    let f = Fixture::new();
    write_onset_source(&f.root.join("grid.wav"), 48000, 57600, 1);
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "grid.wav");
    let markers = (0..=256)
        .map(|i| {
            if i == 0 || i == 256 {
                i * 225
            } else {
                i * 225 + 45
            }
        })
        .collect();
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                marker_edit(31, json!(400), markers),
                json!({"op":"set_song","bpm":400,"lines_per_beat":128})
            ]
        )["ok"],
        true
    );
    let cfg = json!({"version":1,"name":"256 exact anchors","marker_set":31,"start_marker":0,"end_marker":256,"include_start_anchor":true,"division":"OneTwentyEighth","cycle_steps":256});
    let base: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let plan = groove::capture_preview(
        &base,
        GrooveId(51),
        serde_json::from_value(cfg.clone()).unwrap(),
    )
    .unwrap();
    let capture = groove_capture(&mut c, 2, 51, cfg);
    let mut anchors = Vec::new();
    for start in [0, 64, 128, 192, 256] {
        let page = groove_page(&mut c, 2, &capture, start, 64);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        anchors.extend(page["data"]["records"].as_array().unwrap().clone());
        assert_eq!(page["data"]["total"], 256);
    }
    assert_eq!(
        json!(anchors),
        json!(groove::inspect(plan.template()).unwrap())
    );
    for (start, limit) in [(257, 1), (u64::MAX, 1), (0, 0), (0, 65)] {
        assert_eq!(c.call("leviathan_groove_preview_inspect",json!({"expected_revision":2,"preview_token":capture["data"]["preview_token"],"start":start,"limit":limit}))["ok"],false);
    }
    assert_eq!(groove_commit(&mut c, 2, &capture, "capture")["ok"], true);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_instrument","id":1,"name":"Grid voice","sample":"grid.wav"}),
                json!({"op":"upsert_pattern","id":1,"name":"256 notes","rows":1024})
            ]
        )["ok"],
        true
    );
    let cells=(0..256).map(|i|json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":i*4,"column":0,"cell":{"note":{"Note":60},"instrument":1}})).collect();
    assert_eq!(edit(&mut c, 4, cells)["ok"], true);
    let mut cfg = groove_apply_config(100);
    cfg["scope"]["row_end"] = json!(1024);
    let apply = groove_apply(&mut c, 5, cfg.clone());
    assert_eq!(apply["data"]["record_count"], 255);
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let expected = groove::preview(&song, serde_json::from_value(cfg).unwrap()).unwrap();
    let mut changes = Vec::new();
    for start in [0, 64, 128, 192, 255] {
        changes.extend(
            groove_page(&mut c, 5, &apply, start, 64)["data"]["records"]
                .as_array()
                .unwrap()
                .clone(),
        );
    }
    assert_eq!(json!(changes), json!(expected.changes()));
    let mut full = song.clone();
    let template = full.grooves[&GrooveId(51)].clone();
    for id in 100..163 {
        full.register_groove(GrooveId(id), template.clone())
            .unwrap();
    }
    assert_eq!(full.grooves.len(), 64);
    aqua_pattern_core::project::save(
        &full,
        &f.root.join("groove-capacity.leviathan.json"),
        aqua_pattern_core::project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok("leviathan_project_open",json!({"expected_revision":5,"path":"groove-capacity.leviathan.json","discard_unsaved":true}));
    let before = c.document();
    let mut cfg = groove_capture_config();
    cfg["end_marker"] = json!(4);
    cfg["division"] = json!("OneTwentyEighth");
    assert_eq!(
        c.call(
            "leviathan_groove_capture_preview",
            json!({"expected_revision":6,"groove_id":999,"config":cfg})
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    for (start, limit) in [(257, 1), (u64::MAX, 1), (0, 0), (0, 65)] {
        assert_eq!(
            c.call(
                "leviathan_groove_template_inspect",
                json!({"expected_revision":6,"groove_id":51,"start":start,"limit":limit})
            )["ok"],
            false
        );
    }
}

#[cfg(unix)]
#[test]
fn groove_review_reauthorizes_paths_without_claiming_fresh_source_identity() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    let source = f.root.join("kit.wav");
    let bytes = fs::read(&source).unwrap();
    fs::write(&source, b"changed but root-authorized regular file").unwrap();
    let capture = groove_capture(&mut c, 2, 51, groove_capture_config());
    assert_eq!(capture["data"]["source_identity_verified"], false);
    groove_page(&mut c, 2, &capture, 0, 64);
    let before = c.document();
    let outside = f.base.join("outside.wav");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&outside, &source).unwrap();
    assert_eq!(
        groove_commit(&mut c, 2, &capture, "capture")["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        c.call(
            "leviathan_groove_preview_inspect",
            json!({"expected_revision":2,"preview_token":capture["data"]["preview_token"]})
        )["error"]["code"],
        "path_denied"
    );
    fs::remove_file(&source).unwrap();
    fs::write(&source, &bytes).unwrap();
    assert_eq!(groove_commit(&mut c, 2, &capture, "capture")["ok"], true);
    let apply = groove_apply(&mut c, 3, groove_apply_config(100));
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&outside, &source).unwrap();
    assert_eq!(
        groove_commit(&mut c, 3, &apply, "apply")["error"]["code"],
        "path_denied"
    );
    fs::remove_file(&source).unwrap();
    fs::write(&source, b"changed bytes no decoder in timing plan").unwrap();
    assert_eq!(groove_commit(&mut c, 3, &apply, "apply")["ok"], true);
    let failed = c.call(
        "leviathan_song_export",
        json!({"expected_revision":4,"path":"invalid-groove.wav"}),
    );
    assert_eq!(failed["ok"], false);
    assert!(!f.root.join("invalid-groove.wav").exists());
    assert!(!f.root.join("invalid-groove.events.json").exists());
    fs::write(&source, &bytes).unwrap();
    kit_export(&mut c, &f, 4, "verified-groove.wav");
    assert_eq!(fs::read(outside).unwrap(), bytes);
}

#[test]
fn groove_pretty_save_budget_failure_preserves_a_reviewed_zero_strength_plan() {
    use aqua_pattern_core::{project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    groove_setup(&mut c, &f);
    groove_capture_commit(&mut c, 2);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let mut unused = song.patterns[&PatternId(1)].clone();
    unused.name = String::new();
    unused.tracks.clear();
    unused.track_aliases.clear();
    unused.generation = None;
    song.patterns.insert(PatternId(99), unused);
    let original = project::check_serialized_size(&song).unwrap();
    let target = 8 * 1024 * 1024 - 512;
    song.patterns.get_mut(&PatternId(99)).unwrap().name = "x".repeat(target - original);
    assert_eq!(project::check_serialized_size(&song).unwrap(), target);
    project::save(
        &song,
        &f.root.join("groove-near-budget.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok("leviathan_project_open",json!({"expected_revision":3,"path":"groove-near-budget.leviathan.json","discard_unsaved":true}));
    let before = c.ok("leviathan_project_read", json!({"section":"history"}));
    let zero = groove_apply(&mut c, 4, groove_apply_config(0));
    let failed = c.call(
        "leviathan_groove_capture_preview",
        json!({"expected_revision":4,"groove_id":52,"config":groove_capture_config()}),
    );
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("8 MiB"),
        "{failed}"
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        before
    );
    let committed = groove_commit(&mut c, 4, &zero, "apply");
    assert_eq!(committed["ok"], true);
    assert_eq!(committed["data"]["changed"], false);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        before
    );
}

#[test]
fn groove_retrigger_child_crossing_mute_boundary_rejects_even_when_parent_stays_audible() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_source(&f);
    import_source(&mut c, 0, 7, "kit.wav");
    let configured = edit(
        &mut c,
        1,
        vec![
            marker_edit(31, json!(120), vec![0, 93, 48000]),
            json!({"op":"upsert_instrument","id":1,"name":"One-frame crossing","sample":"kit.wav"}),
            json!({"op":"set_song","bpm":120,"lines_per_beat":256}),
            json!({"op":"upsert_pattern","id":1,"name":"Tiny repeat","rows":1}),
            json!({"op":"set_order","pattern_ids":[1,1]}),
            json!({"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":1,"muted":true}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"delay":0.98,"effects":{"retrigger":{"repeats":1,"spacing":0.008}}}}),
        ],
    );
    assert_eq!(configured["ok"], true, "{configured}");
    let config = json!({"version":1,"name":"One-frame late","marker_set":31,"start_marker":0,"end_marker":2,"include_start_anchor":false,"division":"Sixteenth","cycle_steps":16});
    let capture = groove_capture(&mut c, 2, 51, config);
    assert_eq!(groove_commit(&mut c, 2, &capture, "capture")["ok"], true);
    let before = c.document();
    let mut config = groove_apply_config(0);
    config["scope"]["row_end"] = json!(1);
    let reviewed = groove_apply(&mut c, 3, config.clone());
    config["strength_percent"] = json!(100);
    // At 93.75 frames/row: parent 92 -> 93 stays before mute at 94,
    // but its retrigger 93 -> 94 would be newly suppressed.
    let rejected = c.call(
        "leviathan_groove_apply_preview",
        json!({"expected_revision":3,"config":config}),
    );
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("mute eligibility"),
        "{rejected}"
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        groove_commit(&mut c, 3, &reviewed, "apply")["data"]["changed"],
        false
    );
    assert_eq!(c.document(), before);
}

#[test]
fn groove_encoded_half_tie_cannot_change_slot_identity_and_preserves_prior_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_source(&f);
    import_source(&mut c, 0, 7, "kit.wav");
    let setup = edit(
        &mut c,
        1,
        vec![
            marker_edit(31, json!(120), vec![0, 3000, 48000]),
            json!({"op":"upsert_instrument","id":1,"name":"Exact half-tie source","sample":"kit.wav"}),
            json!({"op":"set_song","bpm":123,"lines_per_beat":4}),
            json!({"op":"upsert_pattern","id":1,"name":"Half-tie selection","rows":16}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":{"Note":60},"instrument":1}}),
        ],
    );
    assert_eq!(setup["ok"], true, "{setup}");
    let capture = groove_capture(
        &mut c,
        2,
        51,
        json!({"version":1,"name":"Half-tie anchor","marker_set":31,"start_marker":0,"end_marker":2,"include_start_anchor":true,"division":"Sixteenth","cycle_steps":16}),
    );
    assert_eq!(groove_commit(&mut c, 2, &capture, "capture")["ok"], true);
    let before = c.document();
    let mut config = groove_apply_config(0);
    config["scope"]["row_end"] = json!(16);
    let zero = groove_apply(&mut c, 3, config.clone());
    config["strength_percent"] = json!(100);
    // Source delta3000 is exactly half a Sixteenth at120BPM. At123BPM
    // the rounded output target2927 can encode below the slot1 half-tie.
    // Do not publish a plan which silently reclassifies the note into slot0.
    let rejected = c.call(
        "leviathan_groove_apply_preview",
        json!({"expected_revision":3,"config":config}),
    );
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nearest-slot boundary"),
        "{rejected}"
    );
    assert_eq!(c.document(), before);
    assert_eq!(groove_page(&mut c, 3, &zero, 0, 64)["data"]["total"], 0);
    assert_eq!(
        groove_commit(&mut c, 3, &zero, "apply")["data"]["changed"],
        false
    );
    assert_eq!(c.document(), before);
}

fn keyzone(
    id: u64,
    note_min: u8,
    note_max: u8,
    velocity_min: u8,
    velocity_max: u8,
    region: u64,
) -> Value {
    json!({"id":id,"name":format!("Zone {id}"),"note_min":note_min,"note_max":note_max,"velocity_min":velocity_min,"velocity_max":velocity_max,"region":region,"root_note":60})
}
fn melodic_zones() -> Value {
    json!([
        keyzone(11, 0, 63, 0, 63, 21),
        keyzone(12, 0, 63, 64, 127, 22),
        keyzone(13, 64, 127, 0, 63, 23),
        keyzone(14, 64, 127, 64, 127, 24)
    ])
}
fn set_keyzones(map: u64, zones: Value) -> Value {
    json!({"op":"set_keyzones","map_id":map,"zones":zones})
}
fn melodic_setup(c: &mut Client, f: &Fixture) {
    zone_setup(c, f);
    let result = edit(
        c,
        2,
        vec![
            json!({"op":"upsert_multisample","id":51,"name":"Shared melodic map"}),
            set_keyzones(51, melodic_zones()),
            json!({"op":"assign_multisample","instrument_id":1,"map_id":51}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
}
fn melodic_cell(row: u32, note: u8, velocity: f64) -> Value {
    json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":row,"column":0,"cell":{"note":{"Note":note},"instrument":1,"velocity":velocity,"pan":0.0}})
}
fn melodic_ons(manifest: &Value) -> Vec<(u64, Value)> {
    manifest["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| {
            event["command"].get("On").map(|on| {
                (
                    event["frame"].as_u64().unwrap(),
                    manifest["multisample_voices"][on["voice"].as_u64().unwrap().to_string()]
                        .clone(),
                )
            })
        })
        .collect()
}

#[test]
fn multisample_shared_editor_operations_pitch_velocity_pcm_one_undo_and_roundtrip_match() {
    use aqua_pattern_core::{editor::Editor, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let before = c.document()["data"]["song"].clone();
    let mut editor = Editor::default();
    editor
        .replace_document(serde_json::from_value(before.clone()).unwrap())
        .unwrap();
    editor.edit_mode = true;
    editor
        .upsert_multisample(
            MultisampleId(51),
            MultisampleInstrument {
                name: "Shared melodic map".into(),
                zones: Default::default(),
            },
        )
        .unwrap();
    let zones = melodic_zones()
        .as_array()
        .unwrap()
        .iter()
        .map(|z| {
            let mut z = z.clone();
            let id = z
                .as_object_mut()
                .unwrap()
                .remove("id")
                .unwrap()
                .as_u64()
                .unwrap();
            (KeyZoneId(id), serde_json::from_value::<KeyZone>(z).unwrap())
        })
        .collect();
    editor.set_keyzones(MultisampleId(51), zones).unwrap();
    editor
        .assign_multisample(InstrumentId(1), Some(MultisampleId(51)))
        .unwrap();
    let result = edit(
        &mut c,
        2,
        vec![
            json!({"op":"upsert_multisample","id":51,"name":"Shared melodic map"}),
            set_keyzones(51, melodic_zones()),
            json!({"op":"assign_multisample","instrument_id":1,"map_id":51}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let notes = [(60, 0.25), (60, 0.75), (72, 0.25), (48, 0.75)];
    let mut ops: Vec<_> = notes
        .iter()
        .enumerate()
        .map(|(r, (n, v))| melodic_cell(r as u32, *n, *v))
        .collect();
    let mut region = region_edit(21, 0, 48);
    region["root_note"] = json!(12);
    ops.push(region);
    assert_eq!(edit(&mut c, 5, ops)["ok"], true);
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":6}))["data"]["manifest"].clone();
    assert_eq!(manifest, json!(song_render::evaluate(&song, None).unwrap()));
    let ons = melodic_ons(&manifest);
    assert_eq!(ons.len(), 4);
    for (i, (_, voice)) in ons.iter().enumerate() {
        assert_eq!(voice["map"], 51);
        assert_eq!(voice["zone"], [11, 12, 13, 12][i]);
        assert_eq!(voice["note"], notes[i].0);
        assert_eq!(voice["velocity_bin"], [32, 95, 32, 95][i]);
        assert_eq!(voice["region"]["root_note"], 60);
    }
    let pcm = kit_export(&mut c, &f, 6, "melodic.wav");
    for (i, (frame, _)) in ons.iter().enumerate() {
        // Existing melodic regions retain phonic's f32 exponential gain smoothing,
        // updated per interleaved sample. This is not the kit immediate-start path.
        let mut gain = 1.0f32;
        for k in 0..[48, 48, 24, 95][i] {
            for ch in 0..2 {
                gain += (notes[i].1 as f32 - gain) * (1.0f32 / 256.0) * (44100.0f32 / 48000.0);
                let expected = [0.2f32, 0.4, 0.6, 0.4][i] * gain * 0.5;
                assert!(
                    (pcm[(*frame as usize + k) * 2 + ch] - expected).abs() < 1e-7,
                    "voice{i} frame{k} channel{ch}: {} vs {expected}",
                    pcm[(*frame as usize + k) * 2 + ch]
                );
            }
        }
        assert_eq!(pcm[(*frame as usize + [48, 48, 24, 96][i]) * 2], 0.0);
    }
    song_render::export(&song, None, &f.root.join("shared.wav")).unwrap();
    assert_eq!(
        fs::read(f.root.join("melodic.wav")).unwrap(),
        fs::read(f.root.join("shared.wav")).unwrap()
    );
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"melodic.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"melodic.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(c.document()["data"]["song"], json!(song));
    kit_export(&mut c, &f, 8, "reopened.wav");
    assert_eq!(
        fs::read(f.root.join("melodic.wav")).unwrap(),
        fs::read(f.root.join("reopened.wav")).unwrap()
    );
    assert_eq!(
        fs::read(f.root.join("melodic.events.json")).unwrap(),
        fs::read(f.root.join("reopened.events.json")).unwrap()
    );
}

#[test]
fn multisample_modes_metadata_shared_users_and_complete_zone_replacement_are_explicit() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    melodic_setup(&mut c, &f);
    let zones = c.document()["data"]["song"]["multisamples"]["51"]["zones"].clone();
    let result = edit(
        &mut c,
        3,
        vec![
            json!({"op":"upsert_multisample","id":51,"name":"Renamed map"}),
            json!({"op":"upsert_instrument","id":1,"name":"Renamed voice","sample":null}),
            json!({"op":"assign_multisample","instrument_id":2,"map_id":51}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let doc = c.document();
    assert_eq!(doc["data"]["song"]["multisamples"]["51"]["zones"], zones);
    assert_eq!(
        doc["data"]["song"]["instruments"]["1"]["kind"],
        json!({"type":"Multisample","data":{"map":51}})
    );
    let inspect=c.ok("leviathan_multisample_inspect",json!({"expected_revision":4,"map_id":51,"zones_start":4,"instruments_start":1,"instruments_limit":1}));
    assert_eq!(inspect["data"]["zones"], json!({}));
    assert_eq!(inspect["data"]["instruments_page"]["total"], 2);
    assert_eq!(
        inspect["data"]["assigned_instruments"][0]["instrument_id"],
        2
    );
    assert_eq!(inspect["data"]["source_identity_verified"], false);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"assign_multisample","instrument_id":3,"map_id":null})]
        )["revision"],
        4
    );
    assert_eq!(c.document(), doc, "null on Pitched is a true no-op");
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"assign_kit","instrument_id":3,"kit_id":41})]
        )["ok"],
        true
    );
    let before = c.document();
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![json!({"op":"assign_multisample","instrument_id":3,"map_id":null})]
        )["revision"],
        5
    );
    assert_eq!(c.document(), before, "null cannot detach a kit");
    let result = edit(
        &mut c,
        5,
        vec![
            json!({"op":"assign_multisample","instrument_id":1,"map_id":null}),
            json!({"op":"assign_region","instrument_id":2,"region_id":21}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let doc = c.document();
    assert!(doc["data"]["song"]["instruments"]["1"]
        .get("kind")
        .is_none());
    assert_eq!(doc["data"]["song"]["instruments"]["2"]["region"], 21);
    assert_eq!(
        edit(&mut c, 6, vec![set_keyzones(51, json!([]))])["ok"],
        true
    );
    assert_eq!(
        c.document()["data"]["song"]["multisamples"]["51"]["zones"],
        json!({})
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":7}));
    let before = c.document();
    assert_eq!(
        edit(
            &mut c,
            8,
            vec![
                json!({"op":"upsert_multisample","id":51,"name":"Renamed map"}),
                set_keyzones(51, melodic_zones())
            ]
        )["revision"],
        8
    );
    assert_eq!(c.document(), before, "exact replacements preserve redo");
    assert_eq!(
        edit(&mut c, 8, vec![json!({"op":"remove_multisample","id":51})])["ok"],
        true
    );
}

#[test]
fn multisample_schema_rectangles_references_and_invalid_batches_are_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    melodic_setup(&mut c, &f);
    let before = c.document();
    let mut bad = vec![
        json!({"op":"assign_multisample","instrument_id":1}),
        json!({"op":"set_keyzones","map_id":51}),
        json!({"op":"upsert_multisample","id":0,"name":"zero"}),
        json!({"op":"upsert_multisample","id":52,"name":" "}),
        json!({"op":"remove_multisample","id":51}),
        json!({"op":"remove_region","id":24}),
        set_keyzones(999, melodic_zones()),
    ];
    for (field, value) in [
        ("id", json!(0)),
        ("name", json!(" ")),
        ("name", json!("x".repeat(129))),
        ("note_min", json!(64)),
        ("note_max", json!(128)),
        ("velocity_min", json!(64)),
        ("velocity_max", json!(128)),
        ("root_note", json!(128)),
        ("region", json!(999)),
        ("seed", json!(42)),
    ] {
        let mut z = keyzone(11, 0, 63, 0, 63, 21);
        z[field] = value;
        bad.push(set_keyzones(51, json!([z])));
    }
    for field in [
        "id",
        "name",
        "note_min",
        "note_max",
        "velocity_min",
        "velocity_max",
        "region",
        "root_note",
    ] {
        let mut z = keyzone(11, 0, 63, 0, 63, 21);
        z.as_object_mut().unwrap().remove(field);
        bad.push(set_keyzones(51, json!([z])));
    }
    bad.push(set_keyzones(
        51,
        json!([
            keyzone(11, 0, 63, 0, 63, 21),
            keyzone(11, 64, 127, 64, 127, 22)
        ]),
    ));
    bad.push(set_keyzones(
        51,
        json!([
            keyzone(11, 0, 63, 0, 63, 21),
            keyzone(12, 63, 127, 63, 127, 22)
        ]),
    ));
    bad.push(set_keyzones(
        51,
        json!((0..129)
            .map(|i| keyzone(i + 1, 0, 127, 0, 127, 21))
            .collect::<Vec<_>>()),
    ));
    for op in bad {
        let failed = edit(&mut c, 3, vec![op]);
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
    }
    let failed = edit(
        &mut c,
        3,
        vec![
            set_keyzones(51, json!([keyzone(11, 0, 127, 0, 127, 21)])),
            json!({"op":"remove_multisample","id":999}),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 1);
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(&mut c, 2, vec![set_keyzones(51, json!([]))])["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(c.document(), before);
    // Adjacent edge changes are one full replacement, not temporarily overlapping partial edits.
    let result = edit(
        &mut c,
        3,
        vec![set_keyzones(
            51,
            json!([
                keyzone(11, 0, 62, 0, 127, 21),
                keyzone(12, 63, 127, 0, 127, 22)
            ]),
        )],
    );
    assert_eq!(result["ok"], true, "{result}");
}

#[test]
fn multisample_fresh_creation_preserves_occupied_song_and_both_checked_id_limits() {
    use aqua_pattern_core::{editor::Editor, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 2, vec![kit_cell(1, 0, json!({"Note":36}), 1)])["ok"],
        true
    );
    let before = c.document()["data"]["song"].clone();
    let mut editor = Editor::default();
    editor
        .replace_document(serde_json::from_value(before.clone()).unwrap())
        .unwrap();
    editor.edit_mode = true;
    let next_instrument_id = editor.song.instruments.keys().map(|id| id.0).max().unwrap() + 1;
    let ids = editor
        .create_multisample_instrument("New melodic map".into())
        .unwrap();
    assert_eq!(ids, (InstrumentId(next_instrument_id), MultisampleId(1)));
    let result = edit(
        &mut c,
        3,
        vec![json!({"op":"create_multisample_instrument","name":"New melodic map"})],
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    c.ok("leviathan_song_evaluate", json!({"expected_revision":4}));
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before);
    let history = c.document();
    for name in [" ".into(), "x".repeat(257)] {
        assert_eq!(
            edit(
                &mut c,
                5,
                vec![json!({"op":"create_multisample_instrument","name":name})]
            )["ok"],
            false
        );
        assert_eq!(c.document(), history);
    }
    for (op, revision) in [
        (
            json!({"op":"upsert_multisample","id":u64::MAX,"name":"Last map ID"}),
            5,
        ),
        (
            json!({"op":"upsert_instrument","id":u64::MAX,"name":"Last instrument ID","sample":null}),
            7,
        ),
    ] {
        assert_eq!(edit(&mut c, revision, vec![op])["ok"], true);
        let before = c.document();
        let failed = edit(
            &mut c,
            revision + 1,
            vec![json!({"op":"create_multisample_instrument","name":"Exhausted"})],
        );
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(c.document(), before);
        c.ok(
            "leviathan_history_undo",
            json!({"expected_revision":revision+1}),
        );
    }
}

#[test]
fn multisample_actual_gaps_reject_before_decode_but_muted_notes_do_not_select_and_gain_glide_do_not_remap(
) {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    melodic_setup(&mut c, &f);
    let source = f.root.join("layers.wav");
    let bytes = fs::read(&source).unwrap();
    let result = edit(
        &mut c,
        3,
        vec![
            set_keyzones(51, json!([keyzone(11, 60, 60, 0, 63, 21)])),
            melodic_cell(0, 62, 0.25),
        ],
    );
    assert_eq!(result["ok"], true, "gap is a valid draft: {result}");
    fs::write(&source, b"changed but regular and authorized").unwrap();
    let before = c.document();
    for tool in ["leviathan_song_evaluate", "leviathan_song_export"] {
        let args = if tool.ends_with("export") {
            json!({"expected_revision":4,"path":"gap.wav"})
        } else {
            json!({"expected_revision":4})
        };
        let failed = c.call(tool, args);
        assert_eq!(failed["ok"], false);
        assert!(
            failed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no keyzone"),
            "{failed}"
        );
    }
    assert!(!f.root.join("gap.wav").exists());
    assert!(!f.root.join("gap.events.json").exists());
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":1,"muted":true})
            ]
        )["ok"],
        true
    );
    let silent = kit_export(&mut c, &f, 5, "muted.wav");
    assert!(silent.iter().all(|v| *v == 0.0));
    fs::write(&source, &bytes).unwrap();
    let result = edit(
        &mut c,
        5,
        vec![
            json!({"op":"set_occurrence_track_mute","occurrence_index":0,"track_id":1,"muted":false}),
            melodic_cell(0, 60, 0.25),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":"Empty","velocity":0.75,"effects":{"glide":{"target":72,"semitones_per_second":12}}}}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":6}))["data"]["manifest"].clone();
    let ons = melodic_ons(&manifest);
    assert_eq!(ons.len(), 1);
    assert_eq!(ons[0].1["zone"], 11);
    assert_eq!(ons[0].1["input_velocity"], 0.25);
    kit_export(&mut c, &f, 6, "no-remap.wav");
    // Actual transposed phrase children, not the root trigger's note, choose zones.
    let result = edit(
        &mut c,
        6,
        vec![
            json!({"op":"clear_pattern_track","pattern_id":1,"track_id":1}),
            phrase_metadata(81),
            json!({"op":"set_phrase_cell","phrase_id":81,"row":0,"column":0,"cell":{"note":{"Note":60},"velocity":0.25}}),
            melodic_cell(0, 62, 0.25),
            json!({"op":"set_phrase_trigger","pattern_id":1,"track_id":1,"row":0,"column":0,"trigger":{"phrase":81,"cycles":1}}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let failed = c.call("leviathan_song_evaluate", json!({"expected_revision":7}));
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no keyzone"));
    assert_eq!(
        edit(
            &mut c,
            7,
            vec![set_keyzones(51, json!([keyzone(11, 62, 62, 0, 127, 21)]))]
        )["ok"],
        true
    );
    let m =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":8}))["data"]["manifest"].clone();
    assert_eq!(melodic_ons(&m)[0].1["note"], 62);
}

#[test]
fn multisample_inspection_pages_bound_all_defined_zones_without_assignment_expansion() {
    use aqua_pattern_core::{project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    melodic_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    song.multisamples.clear();
    for map in 1..=32 {
        let zones = (0..128)
            .map(|note| {
                (
                    KeyZoneId(note as u64 + 1),
                    KeyZone {
                        name: format!("Note {note}"),
                        note_min: note,
                        note_max: note,
                        velocity_min: 0,
                        velocity_max: 127,
                        region: RegionId(21),
                        root_note: 60,
                    },
                )
            })
            .collect();
        song.multisamples.insert(
            MultisampleId(map),
            MultisampleInstrument {
                name: format!("Map {map}"),
                zones,
            },
        );
    }
    song.instruments.get_mut(&InstrumentId(1)).unwrap().kind = InstrumentKind::Multisample {
        map: MultisampleId(1),
    };
    for id in 10..210 {
        song.instruments.insert(
            InstrumentId(id),
            Instrument {
                name: format!("Alias {id}"),
                sample: None,
                region: None,
                kind: InstrumentKind::Multisample {
                    map: MultisampleId(1),
                },
            },
        );
    }
    song.validate().unwrap();
    project::save(
        &song,
        &f.root.join("all-zones.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":3,"path":"all-zones.leviathan.json","discard_unsaved":true}),
    );
    let before = c.document();
    for start in [0, 127, 128, 2048, 4095, 4096] {
        let page=c.ok("leviathan_sample_inspect",json!({"expected_revision":4,"sample_id":7,"regions_start":4,"mappings_start":1,"keyzones_start":start,"keyzones_limit":1}));
        assert_eq!(page["data"]["keyzones_page"]["total"], 4096);
        assert_eq!(page["data"]["regions"], json!({}));
        if start < 4096 {
            assert_eq!(
                page["data"]["multisample_mappings"][0]["map_id"],
                start / 128 + 1
            );
            assert_eq!(
                page["data"]["multisample_mappings"][0]["zone_id"],
                start % 128 + 1
            );
        } else {
            assert_eq!(page["data"]["multisample_mappings"], json!([]));
        }
    }
    for start in [0, 64, 128, 200, 201] {
        let page=c.ok("leviathan_multisample_inspect",json!({"expected_revision":4,"map_id":1,"zones_start":128,"instruments_start":start,"instruments_limit":64}));
        assert_eq!(page["data"]["zones"], json!({}));
        assert_eq!(page["data"]["instruments_page"]["total"], 201);
        assert_eq!(
            page["data"]["assigned_instruments"]
                .as_array()
                .unwrap()
                .len(),
            64.min(201 - start)
        );
    }
    for extra in [
        json!({"keyzones_start":4097}),
        json!({"keyzones_start":u64::MAX}),
        json!({"keyzones_limit":0}),
        json!({"keyzones_limit":129}),
    ] {
        let mut args = json!({"expected_revision":4,"sample_id":7});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(c.call("leviathan_sample_inspect", args)["ok"], false);
    }
    for extra in [
        json!({"zones_start":129}),
        json!({"zones_start":u64::MAX}),
        json!({"zones_limit":0}),
        json!({"zones_limit":65}),
        json!({"instruments_start":202}),
        json!({"instruments_limit":65}),
    ] {
        let mut args = json!({"expected_revision":4,"map_id":1});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(c.call("leviathan_multisample_inspect", args)["ok"], false);
    }
    assert_eq!(c.document(), before);
    let failed = edit(
        &mut c,
        4,
        vec![
            json!({"op":"upsert_multisample","id":33,"name":"Over total"}),
            set_keyzones(33, json!([keyzone(1, 0, 127, 0, 127, 21)])),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(c.document(), before);
    let mut ops = (33..=64)
        .map(|id| json!({"op":"upsert_multisample","id":id,"name":format!("Empty {id}")}))
        .collect::<Vec<_>>();
    assert_eq!(edit(&mut c, 4, ops.clone())["ok"], true);
    ops.clear();
    ops.push(json!({"op":"upsert_multisample","id":65,"name":"Over maps"}));
    let before = c.document();
    assert_eq!(edit(&mut c, 5, ops)["ok"], false);
    assert_eq!(c.document(), before);
}

#[cfg(unix)]
#[test]
fn multisample_new_mapping_tools_keep_root_identity_and_output_protections() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    melodic_setup(&mut c, &f);
    assert_eq!(edit(&mut c, 3, vec![melodic_cell(0, 60, 0.25)])["ok"], true);
    let source = f.root.join("layers.wav");
    let bytes = fs::read(&source).unwrap();
    let outside = f.base.join("outside.wav");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(&outside, &source).unwrap();
    let before = c.document();
    let mut zones = melodic_zones();
    zones[0]["root_note"] = json!(61);
    assert_eq!(
        edit(&mut c, 4, vec![set_keyzones(51, zones.clone())])["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), before);
    c.ok(
        "leviathan_multisample_inspect",
        json!({"expected_revision":4,"map_id":51}),
    );
    assert_eq!(
        c.call(
            "leviathan_sample_inspect",
            json!({"expected_revision":4,"sample_id":7})
        )["ok"],
        false
    );
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":4,"path":"escape.wav"})
        )["ok"],
        false
    );
    assert!(!f.root.join("escape.wav").exists());
    fs::remove_file(&source).unwrap();
    fs::write(&source, b"changed regular bytes").unwrap();
    assert_eq!(edit(&mut c, 4, vec![set_keyzones(51, zones)])["ok"], true);
    assert_eq!(
        c.call(
            "leviathan_sample_inspect",
            json!({"expected_revision":5,"sample_id":7})
        )["ok"],
        false
    );
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":5,"path":"tampered.wav"})
        )["ok"],
        false
    );
    assert!(!f.root.join("tampered.wav").exists());
    assert!(!f.root.join("tampered.events.json").exists());
    fs::write(&source, &bytes).unwrap();
    kit_export(&mut c, &f, 5, "restored.wav");
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":5,"path":"restored.wav"})
        )["error"]["code"],
        "output_exists"
    );
    assert_eq!(fs::read(&outside).unwrap(), bytes);
}

#[test]
fn multisample_inspector_byte_guard_is_explicit_and_smaller_independent_pages_recover() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let long_name = "x".repeat(6000);
    let mut ops = vec![json!({"op":"upsert_multisample","id":1,"name":"Many shared users"})];
    for id in 1..=64 {
        ops.push(json!({"op":"upsert_instrument","id":id,"name":long_name,"sample":null}));
        ops.push(json!({"op":"assign_multisample","instrument_id":id,"map_id":1}));
    }
    let result = edit(&mut c, 0, ops);
    assert_eq!(result["ok"], true, "{result}");
    let before = c.document();
    let failed = c.call(
        "leviathan_multisample_inspect",
        json!({"expected_revision":1,"map_id":1}),
    );
    assert_eq!(failed["error"]["code"], "inspection_too_large");
    assert_eq!(c.document(), before);
    let page = c.ok(
        "leviathan_multisample_inspect",
        json!({"expected_revision":1,"map_id":1,"instruments_start":63,"instruments_limit":1}),
    );
    assert_eq!(
        page["data"]["assigned_instruments"],
        json!([{"instrument_id":64,"name":long_name}])
    );
    assert_eq!(page["data"]["instruments_page"]["total"], 64);
    assert_eq!(page["data"]["instruments_page"]["next_start"], Value::Null);
    assert_eq!(page["data"]["zones_page"]["total"], 0);
    let end = c.ok(
        "leviathan_multisample_inspect",
        json!({"expected_revision":1,"map_id":1,"instruments_start":64}),
    );
    assert_eq!(end["data"]["assigned_instruments"], json!([]));
    assert_eq!(c.document(), before);
    assert_eq!(
        c.call(
            "leviathan_multisample_inspect",
            json!({"expected_revision":0,"map_id":1})
        )["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(c.document(), before);
}

#[cfg(unix)]
fn resample_config(output: &str, tail: &str) -> Value {
    json!({"version":1,"occurrence":0,"name":"Printed occurrence","root_note":60,"tail":tail,"output":output})
}
#[cfg(unix)]
fn resample_prepare(c: &mut Client, revision: u64, config: Value) -> Value {
    c.ok(
        "leviathan_resample_prepare",
        json!({"expected_revision":revision,"config":config}),
    )
}
#[cfg(unix)]
fn resample_action(c: &mut Client, revision: u64, prepared: &Value, action: &str) -> Value {
    c.call(
        &format!("leviathan_resample_{action}"),
        json!({"expected_revision":revision,"workflow_token":prepared["data"]["workflow_token"]}),
    )
}
#[cfg(unix)]
fn read_pcm(path: &std::path::Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect()
}
#[cfg(unix)]
fn engine_pcm(song: &aqua_pattern_core::song::Song, frames: u64) -> Vec<f32> {
    let mut engine = aqua_pattern_core::song_render::Engine::new(song, None).unwrap();
    let mut pcm = Vec::new();
    let mut block = [0.0; 1024];
    while engine.position < frames {
        let count = engine.block(&mut block, frames).unwrap();
        assert!(count > 0);
        pcm.extend_from_slice(&block[..count * 2]);
    }
    pcm
}

#[cfg(unix)]
#[test]
fn resample_shared_plan_render_editor_registration_replay_and_roundtrip_are_exact() {
    use aqua_pattern_core::{editor::Editor, resample, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let source = fs::read(f.root.join("kit.wav")).unwrap();
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let config = resample_config("printed.wav", "ReleaseTail");
    let prepared = resample_prepare(&mut c, 2, config.clone());
    let mut direct_config = config.clone();
    direct_config["output"] = json!(f.root.join("printed.wav"));
    let plan = resample::prepare(&song, serde_json::from_value(direct_config).unwrap()).unwrap();
    assert_eq!(prepared["data"]["summary"], json!(plan.summary()));
    assert_eq!(prepared["data"]["phase"], "prepared");
    assert_eq!(
        prepared["data"]["source_identity_basis"],
        "declared_metadata_only"
    );
    assert!(!f.root.join("printed.wav").exists());
    assert_eq!(c.document(), before);
    let printed = resample_action(&mut c, 2, &prepared, "render");
    assert_eq!(printed["ok"], true, "{printed}");
    assert_eq!(
        printed["data"]["workflow_token"],
        prepared["data"]["workflow_token"]
    );
    assert_eq!(printed["data"]["phase"], "printed");
    assert_eq!(
        printed["data"]["source_identity_basis"],
        "verified_during_render"
    );
    assert_eq!(c.document(), before);
    assert!(!f.root.join("printed.events.json").exists());
    let printed_bytes = fs::read(f.root.join("printed.wav")).unwrap();
    let pcm = read_pcm(&f.root.join("printed.wav"));
    let expected = engine_pcm(&song, plan.summary().printed_frames);
    assert_eq!(pcm, expected.iter().map(|v| *v * 2.0).collect::<Vec<_>>());
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "render")["error"]["code"],
        "resample_already_printed"
    );
    assert_eq!(fs::read(f.root.join("printed.wav")).unwrap(), printed_bytes);
    // Independent shared execution in the private fixture only; no production
    // operation deletes artifacts. Replacing with byte-identical output is valid.
    fs::remove_file(f.root.join("printed.wav")).unwrap();
    let shared = resample::render(&plan, |_| Ok(()), || false).unwrap();
    assert_eq!(fs::read(f.root.join("printed.wav")).unwrap(), printed_bytes);
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    resample::verify_artifact(&shared, |_| Ok(())).unwrap();
    editor
        .commit_resample_preview(editor.revision(), &shared)
        .unwrap();
    let registered = resample_action(&mut c, 2, &prepared, "register");
    assert_eq!(registered["ok"], true, "{registered}");
    assert_eq!(registered["data"]["artifact_identity_verified_now"], true);
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    assert_eq!(editor.song.order, song.order);
    assert_eq!(editor.song.patterns, song.patterns);
    assert_eq!(editor.song.tracks, song.tracks);
    let region = &editor.song.regions[&plan.summary().region];
    assert_eq!(region.end_frame, plan.summary().printed_frames);
    assert!(region.amp_envelope.is_none() && region.forward_loop.is_none());
    let mut replay = editor.song.clone();
    for p in replay.patterns.values_mut() {
        p.tracks.clear();
        p.track_aliases.clear();
    }
    replay.order.truncate(1);
    replay.patterns.get_mut(&PatternId(1)).unwrap().rows = 8;
    replay
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default()
        .entry(0)
        .or_default()
        .insert(
            0,
            Cell {
                note: Note::Note(60),
                instrument: Some(plan.summary().instrument),
                velocity: Some(1.0),
                pan: Some(0.0),
                ..Default::default()
            },
        );
    assert_eq!(
        engine_pcm(&replay, plan.summary().printed_frames),
        expected,
        "neutral native-root replay preserves original Engine output exactly"
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], json!(song));
    assert_eq!(fs::read(f.root.join("printed.wav")).unwrap(), printed_bytes);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"resampled.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":6,"path":"resampled.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), source);
    assert_eq!(fs::read(f.root.join("printed.wav")).unwrap(), printed_bytes);
}

#[cfg(unix)]
#[test]
fn resample_schema_no_clobber_failures_preserve_prepared_or_printed_workflow() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let prepared = resample_prepare(&mut c, 2, resample_config("kept.wav", "MusicalOnly"));
    let before = c.document();
    for field in [
        "version",
        "occurrence",
        "name",
        "root_note",
        "tail",
        "output",
    ] {
        let mut cfg = resample_config("bad.wav", "MusicalOnly");
        cfg.as_object_mut().unwrap().remove(field);
        assert_eq!(
            c.call(
                "leviathan_resample_prepare",
                json!({"expected_revision":2,"config":cfg})
            )["ok"],
            false
        );
    }
    for (field, value) in [
        ("version", json!(2)),
        ("occurrence", json!(u64::MAX)),
        ("occurrence", json!(99)),
        ("name", json!(" ")),
        ("name", json!("x".repeat(257))),
        ("root_note", json!(128)),
        ("root_note", json!(60.5)),
        ("tail", json!("Auto")),
        ("gain", json!(2)),
        ("seed", json!(42)),
        ("output", json!("../escape.wav")),
        ("output", json!(f.base.join("absolute.wav"))),
        ("output", json!("bad.flac")),
    ] {
        let mut cfg = resample_config("bad.wav", "MusicalOnly");
        cfg[field] = value;
        assert_eq!(
            c.call(
                "leviathan_resample_prepare",
                json!({"expected_revision":2,"config":cfg})
            )["ok"],
            false,
            "{cfg}"
        );
    }
    assert_eq!(c.document(), before);
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "register")["error"]["code"],
        "resample_not_printed"
    );
    fs::write(f.root.join("kept.wav"), b"competing writer").unwrap();
    let failed = resample_action(&mut c, 2, &prepared, "render");
    assert_eq!(failed["error"]["code"], "output_exists");
    assert_eq!(
        fs::read(f.root.join("kept.wav")).unwrap(),
        b"competing writer"
    );
    resample_action(&mut c, 2, &prepared, "inspect");
    fs::remove_file(f.root.join("kept.wav")).unwrap();
    assert_eq!(resample_action(&mut c, 2, &prepared, "render")["ok"], true);
    let bytes = fs::read(f.root.join("kept.wav")).unwrap();
    assert_eq!(
        c.call(
            "leviathan_resample_prepare",
            json!({"expected_revision":2,"config":resample_config("kept.wav","ReleaseTail")})
        )["error"]["code"],
        "output_exists"
    );
    let injected=c.call("leviathan_resample_register",json!({"expected_revision":2,"workflow_token":prepared["data"]["workflow_token"],"candidate":{},"sha256":"fake"}));
    assert_eq!(injected["error"]["code"], "invalid_arguments");
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "register")["ok"],
        true
    );
    assert_eq!(fs::read(f.root.join("kept.wav")).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn resample_tokens_supersession_aba_new_open_and_artifact_lifetime_are_separate_from_history() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let first = resample_prepare(&mut c, 2, resample_config("first.wav", "MusicalOnly"));
    assert_eq!(resample_action(&mut c, 2, &first, "render")["ok"], true);
    let bytes = fs::read(f.root.join("first.wav")).unwrap();
    let second = resample_prepare(&mut c, 2, resample_config("second.wav", "MusicalOnly"));
    assert_eq!(
        resample_action(&mut c, 2, &first, "register")["error"]["code"],
        "invalid_resample_token"
    );
    assert_eq!(fs::read(f.root.join("first.wav")).unwrap(), bytes);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![json!({"op":"set_song","title":"Changed review"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(
        resample_action(&mut c, 4, &second, "render")["error"]["code"],
        "resample_unavailable"
    );
    assert!(!f.root.join("second.wav").exists());
    let fresh = resample_prepare(&mut c, 4, resample_config("saved.wav", "MusicalOnly"));
    assert_eq!(resample_action(&mut c, 4, &fresh, "render")["ok"], true);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"review.leviathan.json"}),
    );
    assert_eq!(
        resample_action(&mut c, 5, &fresh, "register")["error"]["code"],
        "resample_stale"
    );
    assert!(f.root.join("saved.wav").exists());
    let stale = resample_action(&mut c, 5, &fresh, "inspect");
    assert_eq!(stale["ok"], true);
    assert_eq!(stale["data"]["registration_current"], false);
    assert_eq!(stale["data"]["artifact_identity_verified_now"], false);
    let fresh = resample_prepare(&mut c, 5, resample_config("opened.wav", "MusicalOnly"));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"review.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(
        resample_action(&mut c, 6, &fresh, "render")["error"]["code"],
        "resample_unavailable"
    );
    let printed = resample_prepare(&mut c, 6, resample_config("new.wav", "MusicalOnly"));
    assert_eq!(resample_action(&mut c, 6, &printed, "render")["ok"], true);
    c.ok(
        "leviathan_project_new",
        json!({"expected_revision":6,"discard_unsaved":true}),
    );
    assert!(f.root.join("new.wav").exists());
    assert_eq!(
        resample_action(&mut c, 7, &printed, "register")["error"]["code"],
        "resample_stale"
    );
    let retained = resample_action(&mut c, 7, &printed, "inspect");
    assert_eq!(retained["ok"], true);
    assert_eq!(retained["data"]["registration_current"], false);
    c.finish();
    assert_eq!(fs::read(f.root.join("first.wav")).unwrap(), bytes);
    assert!(f.root.join("new.wav").exists());
}

#[cfg(unix)]
#[test]
fn resample_reauthorizes_sources_output_and_parent_and_reverifies_before_registration() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    fs::create_dir(f.root.join("prints")).unwrap();
    let prepared = resample_prepare(
        &mut c,
        2,
        resample_config("prints/output.wav", "MusicalOnly"),
    );
    let before = c.document();
    let source = f.root.join("kit.wav");
    let bytes = fs::read(&source).unwrap();
    fs::write(f.root.join("same.wav"), &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(f.root.join("same.wav"), &source).unwrap();
    assert_eq!(resample_action(&mut c, 2, &prepared, "render")["ok"], false);
    assert_eq!(c.document(), before);
    assert!(!f.root.join("prints/output.wav").exists());
    fs::remove_file(&source).unwrap();
    fs::write(&source, b"changed registered source").unwrap();
    assert_eq!(resample_action(&mut c, 2, &prepared, "render")["ok"], false);
    assert!(!f.root.join("prints/output.wav").exists());
    fs::write(&source, &bytes).unwrap();
    assert_eq!(resample_action(&mut c, 2, &prepared, "render")["ok"], true);
    let output = f.root.join("prints/output.wav");
    let printed = fs::read(&output).unwrap();
    fs::write(&output, b"changed output").unwrap();
    let historical = resample_action(&mut c, 2, &prepared, "inspect");
    assert_eq!(historical["ok"], true);
    assert_eq!(historical["data"]["artifact_identity_verified_now"], false);
    assert_eq!(historical["data"]["inspection_source_access"], false);
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "register")["ok"],
        false
    );
    assert_eq!(c.document(), before);
    fs::write(&output, &printed).unwrap();
    let outside = f.base.join("outside.wav");
    fs::write(&outside, &printed).unwrap();
    fs::remove_file(&output).unwrap();
    symlink(&outside, &output).unwrap();
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "register")["ok"],
        false
    );
    assert_eq!(fs::read(&outside).unwrap(), printed);
    fs::remove_file(&output).unwrap();
    fs::write(&output, &printed).unwrap();
    // Same lexical root and same bytes do not authorize a replaced parent inode.
    fs::rename(f.root.join("prints"), f.root.join("original-parent")).unwrap();
    fs::create_dir(f.root.join("prints")).unwrap();
    fs::write(&output, &printed).unwrap();
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "register")["ok"],
        false
    );
    assert_eq!(c.document(), before);
    fs::remove_file(&output).unwrap();
    fs::remove_dir(f.root.join("prints")).unwrap();
    fs::rename(f.root.join("original-parent"), f.root.join("prints")).unwrap();
    assert_eq!(
        resample_action(&mut c, 2, &prepared, "register")["ok"],
        true
    );
    assert_eq!(fs::read(&output).unwrap(), printed);
}

#[cfg(unix)]
#[test]
fn resample_cold_scope_retains_alias_mute_and_explicit_tail_but_rejects_used_legacy_or_incoming_memory(
) {
    use aqua_pattern_core::song::*;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let result = edit(
        &mut c,
        2,
        vec![
            json!({"op":"upsert_pattern","id":2,"name":"Shared alias occurrence","rows":4}),
            matrix_alias(2, 1, json!({"pattern":1,"track":1})),
            json!({"op":"set_order","pattern_ids":[1,2]}),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let mut isolated = song.clone();
    isolated.order = vec![song.order[1].clone()];
    for (tail, extra, name) in [
        ("MusicalOnly", 0, "short.wav"),
        ("ReleaseTail", 24000, "tail.wav"),
    ] {
        let mut cfg = resample_config(name, tail);
        cfg["occurrence"] = json!(1);
        let p = resample_prepare(&mut c, 3, cfg);
        assert_eq!(p["data"]["summary"]["pattern"], 2);
        assert_eq!(p["data"]["summary"]["tail_frames"], extra);
        let n = p["data"]["summary"]["printed_frames"].as_u64().unwrap();
        assert_eq!(n, 24000 + extra);
        assert_eq!(resample_action(&mut c, 3, &p, "render")["ok"], true);
        assert_eq!(
            read_pcm(&f.root.join(name)),
            engine_pcm(&isolated, n)
                .iter()
                .map(|v| v * 2.0)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":1,"muted":true})
            ]
        )["ok"],
        true
    );
    let mut cfg = resample_config("muted-print.wav", "MusicalOnly");
    cfg["occurrence"] = json!(1);
    let p = resample_prepare(&mut c, 4, cfg);
    assert_eq!(p["data"]["source_count"], 0);
    assert_eq!(resample_action(&mut c, 4, &p, "render")["ok"], true);
    assert!(read_pcm(&f.root.join("muted-print.wav"))
        .iter()
        .all(|v| *v == 0.0));
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"upsert_instrument","id":2,"name":"Unused legacy allowed","sample":"kit.wav"})
            ]
        )["ok"],
        true
    );
    resample_prepare(
        &mut c,
        5,
        resample_config("unused-legacy.wav", "MusicalOnly"),
    );
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![
                json!({"op":"upsert_instrument","id":1,"name":"Used legacy rejects","sample":"kit.wav"})
            ]
        )["ok"],
        true
    );
    let failed = c.call(
        "leviathan_resample_prepare",
        json!({"expected_revision":6,"config":resample_config("legacy.wav","MusicalOnly")}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("import/register"));
    assert!(!f.root.join("legacy.wav").exists());
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![
                json!({"op":"assign_region","instrument_id":1,"region_id":21}),
                json!({"op":"make_track_unique","pattern_id":2,"track_id":1}),
                json!({"op":"set_cell","pattern_id":2,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"velocity":1.0}})
            ]
        )["ok"],
        true
    );
    assert_eq!(
        edit(
            &mut c,
            7,
            vec![
                json!({"op":"set_occurrence_track_mute","occurrence_index":1,"track_id":1,"muted":false})
            ]
        )["ok"],
        true
    );
    c.ok("leviathan_song_evaluate", json!({"expected_revision":8}));
    let mut cfg = resample_config("incoming.wav", "MusicalOnly");
    cfg["occurrence"] = json!(1);
    let failed = c.call(
        "leviathan_resample_prepare",
        json!({"expected_revision":8,"config":cfg}),
    );
    assert_eq!(
        failed["ok"], false,
        "cold scope must not borrow previous instrument memory"
    );
    assert!(!f.root.join("incoming.wav").exists());
}

#[cfg(unix)]
#[test]
fn resample_checked_ids_printed_frames_and_voice_work_fail_before_publication() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    for (op, revision) in [
        (
            json!({"op":"upsert_instrument","id":u64::MAX,"name":"Exhausted","sample":null}),
            2,
        ),
        (
            json!({"op":"upsert_region","id":u64::MAX,"name":"Exhausted","sample_id":7,"start_frame":0,"end_frame":48000,"root_note":60}),
            4,
        ),
    ] {
        assert_eq!(edit(&mut c, revision, vec![op])["ok"], true);
        let before = c.document();
        assert_eq!(c.call("leviathan_resample_prepare",json!({"expected_revision":revision+1,"config":resample_config("ids.wav","MusicalOnly")}))["ok"],false);
        assert_eq!(c.document(), before);
        assert!(!f.root.join("ids.wav").exists());
        c.ok(
            "leviathan_history_undo",
            json!({"expected_revision":revision+1}),
        );
    }
    assert_eq!(
        c.call(
            "leviathan_sample_import",
            json!({"expected_revision":6,"sample_id":u64::MAX,"path":"kit.wav"})
        )["ok"],
        true
    );
    assert_eq!(
        c.call(
            "leviathan_resample_prepare",
            json!({"expected_revision":7,"config":resample_config("ids.wav","MusicalOnly")})
        )["ok"],
        false
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":7}));
    assert_eq!(
        edit(
            &mut c,
            8,
            vec![
                json!({"op":"set_song","bpm":20,"lines_per_beat":1}),
                json!({"op":"upsert_pattern","id":1,"name":"Over frame budget","rows":64})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let fail = c.call(
        "leviathan_resample_prepare",
        json!({"expected_revision":9,"config":resample_config("frames.wav","MusicalOnly")}),
    );
    assert_eq!(fail["ok"], false);
    assert!(
        fail["error"]["message"]
            .as_str()
            .unwrap()
            .contains("printed-frame"),
        "{fail}"
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            9,
            vec![
                json!({"op":"upsert_pattern","id":1,"name":"Over voice work","rows":16}),
                json!({"op":"upsert_track","id":1,"name":"Many voices","note_columns":128}),
                json!({"op":"clear_pattern_track","pattern_id":1,"track_id":1})
            ]
        )["ok"],
        true
    );
    assert_eq!(edit(&mut c,10,(0..128).map(|col|json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":col,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.0}})).collect())["ok"],true);
    let before = c.document();
    let fail = c.call(
        "leviathan_resample_prepare",
        json!({"expected_revision":11,"config":resample_config("work.wav","MusicalOnly")}),
    );
    assert_eq!(fail["ok"], false);
    assert!(
        fail["error"]["message"]
            .as_str()
            .unwrap()
            .contains("voice-frames"),
        "{fail}"
    );
    assert_eq!(c.document(), before);
    assert!(!f.root.join("work.wav").exists());
    assert!(!f.root.join("frames.wav").exists());
}

#[cfg(unix)]
#[test]
fn resample_source_pages_use_shared_selected_identity_snapshot_and_explicit_empty_end() {
    use aqua_pattern_core::{project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let asset = song.samples[&SampleId(7)].clone();
    song.samples.clear();
    song.regions.clear();
    song.instruments.clear();
    song.bpm = 400.0;
    song.lines_per_beat = 256;
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 127;
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 1;
    pattern.tracks.clear();
    for id in 1..=127 {
        song.samples.insert(SampleId(id), asset.clone());
        song.regions.insert(
            RegionId(id),
            SampleRegion {
                name: format!("Region {id}"),
                sample: SampleId(id),
                start_frame: 0,
                end_frame: 48000,
                root_note: 60,
                amp_envelope: None,
                forward_loop: None,
            },
        );
        song.instruments.insert(
            InstrumentId(id),
            Instrument {
                name: format!("Source {id}"),
                sample: None,
                region: Some(RegionId(id)),
                kind: InstrumentKind::Pitched,
            },
        );
        pattern
            .tracks
            .entry(TrackId(1))
            .or_default()
            .entry(0)
            .or_default()
            .insert(
                (id - 1) as u16,
                Cell {
                    note: Note::Note(60),
                    instrument: Some(InstrumentId(id)),
                    velocity: Some(0.0),
                    ..Default::default()
                },
            );
    }
    song.validate().unwrap();
    project::save(
        &song,
        &f.root.join("sources.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":2,"path":"sources.leviathan.json","discard_unsaved":true}),
    );
    let before = c.document();
    let p = resample_prepare(&mut c, 3, resample_config("sources.wav", "MusicalOnly"));
    assert_eq!(p["data"]["source_count"], 127);
    let mut all = Vec::new();
    for start in [0, 64, 127] {
        let page=c.ok("leviathan_resample_inspect",json!({"expected_revision":3,"workflow_token":p["data"]["workflow_token"],"start":start,"limit":64}));
        assert_eq!(page["data"]["total"], 127);
        all.extend(page["data"]["source_records"].as_array().unwrap().clone());
    }
    assert_eq!(all.len(), 127);
    for (index, record) in all.iter().enumerate() {
        assert_eq!(record["sample"], index + 1);
        assert_eq!(record["asset"], json!(asset));
    }
    for (start, limit) in [(128, 1), (u64::MAX, 1), (0, 0), (0, 65)] {
        assert_eq!(c.call("leviathan_resample_inspect",json!({"expected_revision":3,"workflow_token":p["data"]["workflow_token"],"start":start,"limit":limit}))["ok"],false);
    }
    assert_eq!(resample_action(&mut c, 3, &p, "render")["ok"], true);
    let mut printed = Vec::new();
    for start in [0, 64] {
        let page=c.ok("leviathan_resample_inspect",json!({"expected_revision":3,"workflow_token":p["data"]["workflow_token"],"start":start}));
        assert!(page["data"]["receipt"].get("source_identities").is_none());
        printed.extend(page["data"]["source_records"].as_array().unwrap().clone());
    }
    assert_eq!(printed, all);
    assert_eq!(c.document(), before);
    assert!(read_pcm(&f.root.join("sources.wav"))
        .iter()
        .all(|v| *v == 0.0));
}

#[cfg(unix)]
#[test]
fn resample_fixed_premaster_gain_is_not_normalization_and_clipping_failure_leaves_no_artifact() {
    let f = Fixture::new();
    let mut wav = hound::WavWriter::create(
        f.root.join("hot.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..128 {
        wav.write_sample(1.8f32).unwrap();
    }
    wav.finalize().unwrap();
    let mut c = Client::ready(&f);
    import_source(&mut c, 0, 7, "hot.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                region_edit(21, 0, 128),
                json!({"op":"assign_region","instrument_id":1,"region_id":21}),
                json!({"op":"set_song","bpm":400,"lines_per_beat":256}),
                json!({"op":"upsert_pattern","id":1,"name":"Hot float print","rows":1}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":1.0,"pan":0.0}})
            ]
        )["ok"],
        true
    );
    let p = resample_prepare(&mut c, 2, resample_config("premaster.wav", "MusicalOnly"));
    let printed = resample_action(&mut c, 2, &p, "render");
    assert_eq!(printed["ok"], true, "{printed}");
    assert_eq!(
        printed["data"]["receipt"]["post_master_peak"]
            .as_f64()
            .unwrap(),
        f64::from(0.9f32)
    );
    assert_eq!(
        printed["data"]["receipt"]["asset_peak"].as_f64().unwrap(),
        f64::from(1.8f32)
    );
    assert!(read_pcm(&f.root.join("premaster.wav"))
        .iter()
        .all(|v| *v == 1.8f32));
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"upsert_track","id":1,"name":"Clipping","note_columns":2}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":1,"cell":{"note":{"Note":60},"instrument":1,"velocity":1.0}})
            ]
        )["ok"],
        true
    );
    let p = resample_prepare(&mut c, 3, resample_config("clipped.wav", "MusicalOnly"));
    let before = c.document();
    let entries = fs::read_dir(&f.root).unwrap().count();
    let failed = resample_action(&mut c, 3, &p, "render");
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("clipping"));
    assert!(!f.root.join("clipped.wav").exists());
    assert_eq!(
        fs::read_dir(&f.root).unwrap().count(),
        entries,
        "owned failed temporary cleaned"
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        resample_action(&mut c, 3, &p, "inspect")["data"]["phase"],
        "prepared"
    );
}

#[cfg(unix)]
#[test]
fn resample_registration_preflights_all_sources_without_undoing_a_successful_used_only_print() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let unused = f.root.join("unused.wav");
    let source = fs::read(f.root.join("kit.wav")).unwrap();
    fs::write(&unused, &source).unwrap();
    import_source(&mut c, 2, 8, "unused.wav");
    let p = resample_prepare(&mut c, 3, resample_config("durable.wav", "ReleaseTail"));
    // Print checks actually selected registered bytes, not unplayed asset identities.
    fs::write(&unused, b"tampered unused registered source").unwrap();
    let printed = resample_action(&mut c, 3, &p, "render");
    assert_eq!(printed["ok"], true, "{printed}");
    assert_eq!(printed["data"]["summary"]["source_samples"], json!([7]));
    let before = c.document();
    let bytes = fs::read(f.root.join("durable.wav")).unwrap();
    let rejected = resample_action(&mut c, 3, &p, "register");
    assert_eq!(rejected["ok"], false);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("published WAV retained"),
        "{rejected}"
    );
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(f.root.join("durable.wav")).unwrap(), bytes);
    assert_eq!(
        resample_action(&mut c, 3, &p, "inspect")["data"]["phase"],
        "printed"
    );
    fs::remove_file(&unused).unwrap();
    assert_eq!(resample_action(&mut c, 3, &p, "register")["ok"], false);
    assert_eq!(c.document(), before);
    fs::write(&unused, &source).unwrap();
    assert_eq!(resample_action(&mut c, 3, &p, "register")["ok"], true);
    assert_eq!(fs::read(f.root.join("durable.wav")).unwrap(), bytes);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"registered.leviathan.json"}),
    );
}

#[cfg(unix)]
#[test]
fn resample_printed_receipt_survives_aba_without_authorizing_registration_or_claiming_fresh_bytes()
{
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let before = c.document()["data"]["song"].clone();
    let p = resample_prepare(&mut c, 2, resample_config("historical.wav", "MusicalOnly"));
    let printed = resample_action(&mut c, 2, &p, "render");
    assert_eq!(printed["ok"], true);
    assert_eq!(printed["data"]["registration_current"], true);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![json!({"op":"set_song","title":"Intervening edit"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before);
    assert_eq!(
        resample_action(&mut c, 4, &p, "register")["error"]["code"],
        "resample_stale"
    );
    assert_eq!(
        resample_action(&mut c, 4, &p, "render")["error"]["code"],
        "resample_stale"
    );
    let bytes = fs::read(f.root.join("historical.wav")).unwrap();
    fs::remove_file(f.root.join("historical.wav")).unwrap(); // external private-fixture change, not a server operation
    let receipt = resample_action(&mut c, 4, &p, "inspect");
    assert_eq!(receipt["ok"], true);
    assert_eq!(receipt["data"]["receipt"], printed["data"]["receipt"]);
    assert_eq!(receipt["data"]["registration_current"], false);
    assert_eq!(receipt["data"]["review_current"], false);
    assert_eq!(receipt["data"]["artifact_identity_verified_now"], false);
    assert_eq!(receipt["data"]["inspection_source_access"], false);
    assert_eq!(
        c.call(
            "leviathan_resample_prepare",
            json!({"expected_revision":4,"config":resample_config("../denied.wav","MusicalOnly")})
        )["ok"],
        false
    );
    assert_eq!(resample_action(&mut c, 4, &p, "inspect")["ok"], true);
    fs::write(f.root.join("historical.wav"), bytes).unwrap();
    let next = resample_prepare(&mut c, 4, resample_config("fresh.wav", "MusicalOnly"));
    assert_eq!(
        resample_action(&mut c, 4, &p, "inspect")["error"]["code"],
        "invalid_resample_token"
    );
    assert_eq!(
        resample_action(&mut c, 4, &next, "inspect")["data"]["review_current"],
        true
    );
    assert_eq!(c.document()["data"]["song"], before);
    assert!(c.document()["data"]["session"]["can_redo"]
        .as_bool()
        .unwrap());
}

fn tempo_config(minimum: u16, maximum: u16) -> Value {
    json!({"version":1,"min_bpm_tenths":minimum,"max_bpm_tenths":maximum})
}
fn tempo_marker_evidence(start: u16, end: u16, include_start: bool) -> Value {
    json!({"kind":"markers","marker_set_id":31,"start_index":start,"end_index":end,"include_start":include_start})
}
fn tempo_setup(c: &mut Client, f: &Fixture) {
    let mut wav = hound::WavWriter::create(
        f.root.join("source.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..192000u32 {
        let age = frame % 24000;
        let v = if age < 128 {
            0.5 * (1.0 - age as f32 / 128.0) * if age % 2 == 0 { 1.0 } else { -1.0 }
        } else {
            0.0
        };
        wav.write_sample(v).unwrap();
    }
    wav.finalize().unwrap();
    import_source(c, 0, 7, "source.wav");
    let markers = (0..=8).map(|i| i * 24000).collect::<Vec<_>>();
    let result = edit(
        c,
        1,
        vec![
            marker_edit(31, Value::Null, markers.clone()),
            marker_edit(32, json!(70), markers),
            json!({"op":"set_marker_lock","marker_set_id":31,"frame":24000,"locked":true}),
            region_edit(21, 0, 192000),
            json!({"op":"assign_region","instrument_id":1,"region_id":21}),
            json!({"op":"upsert_pattern","id":1,"name":"Tempo is metadata only","rows":4}),
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5,"pan":0.0}}),
            kit_cell(1, 1, json!("Off"), 1),
        ],
    );
    assert_eq!(result["ok"], true, "{result}");
}
fn tempo_analyze(c: &mut Client, revision: u64, evidence: Value, config: Value) -> Value {
    c.ok(
        "leviathan_tempo_analyze",
        json!({"expected_revision":revision,"evidence":evidence,"config":config}),
    )
}
fn tempo_use(c: &mut Client, revision: u64, analysis: &Value, target: u64, index: usize) -> Value {
    c.ok("leviathan_tempo_use_bpm_preview",json!({"expected_revision":revision,"analysis_token":analysis["data"]["analysis_token"],"target_marker_set_id":target,"candidate_index":index}))
}
fn tempo_commit(c: &mut Client, revision: u64, review: &Value) -> Value {
    c.call(
        "leviathan_tempo_use_bpm_commit",
        json!({"expected_revision":revision,"preview_token":review["data"]["preview_token"]}),
    )
}
fn tempo_points(
    c: &mut Client,
    revision: u64,
    analysis: &Value,
    start: usize,
    limit: usize,
) -> Value {
    c.call("leviathan_tempo_inspect",json!({"expected_revision":revision,"analysis_token":analysis["data"]["analysis_token"],"kind":"points","start":start,"limit":limit}))
}

#[test]
fn tempo_marker_analysis_pulses_and_explicit_use_match_shared_editor_with_exact_roundtrip() {
    use aqua_pattern_core::{editor::Editor, song::*, tempo};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let bytes = fs::read(f.root.join("source.wav")).unwrap();
    let shared = tempo::analyze_markers(
        &song,
        tempo::MarkerRequest {
            marker_set: MarkerSetId(31),
            start_index: 0,
            end_index: 8,
            include_start: true,
        },
        tempo::TempoConfig {
            version: 1,
            min_bpm_tenths: 1200,
            max_bpm_tenths: 1200,
        },
    )
    .unwrap();
    let analysis = tempo_analyze(
        &mut c,
        2,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    assert_eq!(analysis["data"]["analysis"], json!(shared.analysis()));
    assert_eq!(analysis["data"]["analysis_id"], shared.id());
    assert_eq!(analysis["data"]["source_identity_verified_now"], false);
    assert_eq!(
        analysis["data"]["analysis"]["candidates"][0]["bpm_tenths"],
        1200
    );
    let mut points = Vec::new();
    let mut start = 0;
    loop {
        let page = tempo_points(&mut c, 2, &analysis, start, 3);
        assert_eq!(page["ok"], true);
        points.extend(page["data"]["records"].as_array().unwrap().clone());
        if let Some(next) = page["data"]["page"]["next_start"].as_u64() {
            start = next as usize
        } else {
            break;
        }
    }
    assert_eq!(json!(points), json!(shared.points()));
    let mut pulses = Vec::new();
    let mut start = 0;
    loop {
        let page=c.ok("leviathan_tempo_inspect",json!({"expected_revision":2,"analysis_token":analysis["data"]["analysis_token"],"kind":"pulses","candidate_index":0,"start":start,"limit":3}));
        let exact = shared.pulse_page(0, start, 3).unwrap();
        assert_eq!(page["data"]["page"]["frames"], json!(exact.frames));
        pulses.extend(exact.frames);
        if let Some(next) = page["data"]["page"]["next_start"].as_u64() {
            start = next as usize
        } else {
            break;
        }
    }
    assert_eq!(pulses, shared.pulse_page(0, 0, 256).unwrap().frames);
    assert_eq!(c.document(), before);
    let review = tempo_use(&mut c, 2, &analysis, 32, 0);
    let selected = shared.use_bpm(MarkerSetId(32), 0).unwrap();
    assert_eq!(review["data"]["summary"], json!(selected.summary()));
    assert_eq!(c.document(), before);
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor
        .commit_tempo_preview(editor.revision(), &selected)
        .unwrap();
    let applied = tempo_commit(&mut c, 2, &review);
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["data"]["changed"], true);
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let mut expected = song.clone();
    expected
        .marker_sets
        .get_mut(&MarkerSetId(32))
        .unwrap()
        .source_bpm = Some(120.0);
    assert_eq!(editor.song, expected);
    assert_eq!(
        aqua_pattern_core::song_render::evaluate(&song, None).unwrap(),
        aqua_pattern_core::song_render::evaluate(&editor.song, None).unwrap()
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], json!(song));
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"tempo.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":6,"path":"tempo.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(c.document()["data"]["song"], json!(expected));
    assert_eq!(fs::read(f.root.join("source.wav")).unwrap(), bytes);
}

#[test]
fn tempo_strict_inputs_insufficient_evidence_and_wrong_source_use_leave_previous_review_unchanged()
{
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let good = tempo_analyze(
        &mut c,
        2,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let before = c.document();
    let valid = json!({"expected_revision":2,"evidence":tempo_marker_evidence(0,8,true),"config":tempo_config(1200,1200)});
    for field in ["version", "min_bpm_tenths", "max_bpm_tenths"] {
        let mut args = valid.clone();
        args["config"].as_object_mut().unwrap().remove(field);
        assert_eq!(c.call("leviathan_tempo_analyze", args)["ok"], false);
    }
    for cfg in [
        json!({"version":2,"min_bpm_tenths":200,"max_bpm_tenths":4000}),
        tempo_config(199, 4000),
        tempo_config(4000, 3999),
        tempo_config(200, 4001),
        json!({"version":1,"min_bpm_tenths":200.5,"max_bpm_tenths":4000}),
        json!({"version":1,"min_bpm_tenths":200,"max_bpm_tenths":4000,"seed":1}),
    ] {
        let mut args = valid.clone();
        args["config"] = cfg;
        assert_eq!(c.call("leviathan_tempo_analyze", args)["ok"], false);
    }
    for evidence in [
        tempo_marker_evidence(8, 8, true),
        tempo_marker_evidence(0, 9, true),
        json!({"kind":"markers","marker_set_id":99,"start_index":0,"end_index":8,"include_start":true}),
        json!({"kind":"markers","marker_set_id":31,"start_index":0,"end_index":8}),
        json!({"kind":"markers","marker_set_id":31,"start_index":0,"end_index":8,"include_start":true,"points":[0,24000]}),
    ] {
        let mut args = valid.clone();
        args["evidence"] = evidence;
        assert_eq!(c.call("leviathan_tempo_analyze", args)["ok"], false);
    }
    assert_eq!(tempo_points(&mut c, 2, &good, 0, 64)["ok"], true);
    assert_eq!(c.document(), before);
    for args in [
        json!({"expected_revision":2,"analysis_token":good["data"]["analysis_token"],"kind":"points","candidate_index":null}),
        json!({"expected_revision":2,"analysis_token":good["data"]["analysis_token"],"kind":"pulses"}),
        json!({"expected_revision":2,"analysis_token":good["data"]["analysis_token"],"kind":"pulses","candidate_index":null}),
    ] {
        assert_eq!(c.call("leviathan_tempo_inspect", args)["ok"], false);
    }
    for index in [8, u64::MAX] {
        assert_eq!(c.call("leviathan_tempo_use_bpm_preview",json!({"expected_revision":2,"analysis_token":good["data"]["analysis_token"],"target_marker_set_id":32,"candidate_index":index}))["ok"],false);
    }
    let empty = tempo_analyze(
        &mut c,
        2,
        tempo_marker_evidence(0, 1, false),
        tempo_config(200, 4000),
    );
    assert_eq!(empty["data"]["analysis"]["candidates"], json!([]));
    assert!(!empty["data"]["analysis"]["insufficient_reason"].is_null());
    assert_eq!(c.call("leviathan_tempo_use_bpm_preview",json!({"expected_revision":2,"analysis_token":empty["data"]["analysis_token"],"target_marker_set_id":32,"candidate_index":0}))["ok"],false);
    assert_eq!(c.document(), before);
    import_source(&mut c, 2, 8, "source.wav");
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_marker_set","id":33,"name":"Other registered source","sample_id":8,"source_bpm":null,"markers":[0,192000]})
            ]
        )["ok"],
        true
    );
    let a = tempo_analyze(
        &mut c,
        4,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let good_use = tempo_use(&mut c, 4, &a, 32, 0);
    let before = c.document();
    for target in [33, 999] {
        assert_eq!(c.call("leviathan_tempo_use_bpm_preview",json!({"expected_revision":4,"analysis_token":a["data"]["analysis_token"],"target_marker_set_id":target,"candidate_index":0}))["ok"],false);
    }
    assert_eq!(c.document(), before);
    assert_eq!(tempo_commit(&mut c, 4, &good_use)["ok"], true);
}

#[test]
fn tempo_review_tokens_aba_noop_and_supersession_preserve_ordinary_history() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let a = tempo_analyze(
        &mut c,
        2,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let use_a = tempo_use(&mut c, 2, &a, 32, 0);
    assert_eq!(
        edit(&mut c, 2, vec![json!({"op":"set_song","title":"ABA"})])["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(tempo_commit(&mut c, 4, &use_a)["ok"], false);
    assert_eq!(tempo_points(&mut c, 4, &a, 0, 64)["ok"], false);
    let a = tempo_analyze(
        &mut c,
        4,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 4, &a, 32, 0);
    assert_eq!(tempo_commit(&mut c, 4, &review)["ok"], true);
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![json!({"op":"set_song","title":"Redo preserved"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    let before = c.document();
    assert_eq!(before["data"]["session"]["can_redo"], true);
    let a = tempo_analyze(
        &mut c,
        7,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let older = tempo_use(&mut c, 7, &a, 32, 0);
    let newer = tempo_use(&mut c, 7, &a, 32, 0);
    assert_eq!(
        tempo_commit(&mut c, 7, &older)["error"]["code"],
        "invalid_tempo_use_token"
    );
    let no_op = tempo_commit(&mut c, 7, &newer);
    assert_eq!(no_op["ok"], true);
    assert_eq!(no_op["data"]["changed"], false);
    assert_eq!(no_op["revision"], 7);
    assert_eq!(c.document(), before);
    assert_eq!(tempo_commit(&mut c, 7, &newer)["ok"], false);
    assert_eq!(tempo_points(&mut c, 7, &a, 0, 64)["ok"], true);
    let replacement = tempo_analyze(
        &mut c,
        7,
        tempo_marker_evidence(1, 8, true),
        tempo_config(1200, 1200),
    );
    assert_eq!(
        tempo_points(&mut c, 7, &a, 0, 64)["error"]["code"],
        "invalid_tempo_token"
    );
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":7,"path":"same.leviathan.json"}),
    );
    assert_eq!(tempo_points(&mut c, 8, &replacement, 0, 64)["ok"], false);
    let a = tempo_analyze(
        &mut c,
        8,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 8, &a, 32, 0);
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":8,"path":"same.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(tempo_commit(&mut c, 9, &review)["ok"], false);
}

#[cfg(unix)]
#[test]
fn tempo_marker_mode_uses_stored_identity_but_reauthorizes_paths_before_explicit_metadata_commit() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let source = f.root.join("source.wav");
    let bytes = fs::read(&source).unwrap();
    fs::write(&source, b"changed bytes are not marker timing evidence").unwrap();
    let a = tempo_analyze(
        &mut c,
        2,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    assert_eq!(a["data"]["source_identity_verified_now"], false);
    let review = tempo_use(&mut c, 2, &a, 32, 0);
    let before = c.document();
    fs::write(f.base.join("outside.wav"), &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(f.base.join("outside.wav"), &source).unwrap();
    assert_eq!(tempo_points(&mut c, 2, &a, 0, 64)["ok"], false);
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], false);
    assert_eq!(c.document(), before);
    fs::remove_file(&source).unwrap();
    fs::write(&source, &bytes).unwrap();
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], true);
    assert_eq!(fs::read(&source).unwrap(), bytes);
}

fn tempo_cached_evidence(onset: &Value, basis: &str, start: u64, end: u64) -> Value {
    json!({"kind":"cached_onsets","cache_token":onset["data"]["cache_token"],"start_frame":start,"end_frame":end,"basis":basis})
}
#[test]
fn tempo_cached_peak_and_refined_analysis_exactly_match_shared_trusted_cache_without_mutating_markers(
) {
    use aqua_pattern_core::{onset_service, onset_v2, song::*, tempo};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let onset = onset_prepare(
        &mut c,
        2,
        "Balanced",
        json!({"threshold":0.2,"mad_multiplier":2.0}),
    );
    let config: onset_v2::Config =
        serde_json::from_value(onset["data"]["effective_config"].clone()).unwrap();
    let asset = &song.samples[&SampleId(7)];
    let cached = onset_service::prepare_verified(asset, &asset.path, &config).unwrap();
    for (basis, typed) in [
        ("peak", tempo::PointBasis::Peak),
        ("refined_attack", tempo::PointBasis::RefinedAttack),
    ] {
        let shared = tempo::analyze_cached(
            &song,
            SampleId(7),
            &cached.cache,
            &config,
            tempo::CachedRequest {
                start_frame: 0,
                end_frame: 192000,
                basis: typed,
            },
            tempo::TempoConfig {
                version: 1,
                min_bpm_tenths: 1200,
                max_bpm_tenths: 1200,
            },
        )
        .unwrap();
        let result = tempo_analyze(
            &mut c,
            2,
            tempo_cached_evidence(&onset, basis, 0, 192000),
            tempo_config(1200, 1200),
        );
        assert_eq!(result["data"]["analysis"], json!(shared.analysis()));
        assert_eq!(result["data"]["analysis_id"], shared.id());
        assert_eq!(result["data"]["source_identity_verified_now"], true);
        assert_eq!(
            result["data"]["analysis"]["binding"]["kind"]["onset_config_sha256"],
            onset["data"]["source_analysis_summary"]["config_sha256"]
        );
        assert_eq!(
            tempo_points(&mut c, 2, &result, 0, 64)["data"]["records"],
            json!(shared.points())
        );
        assert!(
            !shared.analysis().candidates.is_empty(),
            "fixture should produce usable periodic evidence: {:?}",
            shared.analysis()
        );
        let selected = shared.use_bpm(MarkerSetId(32), 0).unwrap();
        let preview = tempo_use(&mut c, 2, &result, 32, 0);
        assert_eq!(preview["data"]["summary"], json!(selected.summary()));
    }
    assert_eq!(c.document(), before);
    assert_eq!(onset_full_result(&mut c, 2, &onset), json!(cached.result));
}

#[test]
fn tempo_cached_epoch_repick_and_retirement_guards_differ_from_independent_marker_evidence() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let before = c.document();
    let first = onset_prepare(
        &mut c,
        2,
        "Balanced",
        json!({"threshold":0.2,"mad_multiplier":2.0}),
    );
    let analysis = tempo_analyze(
        &mut c,
        2,
        tempo_cached_evidence(&first, "peak", 0, 192000),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 2, &analysis, 32, 0);
    let failed=c.call("leviathan_onset_v2_repick",json!({"expected_revision":2,"cache_token":first["data"]["cache_token"],"picker":{"threshold":1}}));
    assert_eq!(failed["ok"], false);
    assert_eq!(tempo_points(&mut c, 2, &analysis, 0, 64)["ok"], true);
    let repicked=c.ok("leviathan_onset_v2_repick",json!({"expected_revision":2,"cache_token":first["data"]["cache_token"],"picker":{"enable_phase":true}}));
    assert_eq!(
        tempo_points(&mut c, 2, &analysis, 0, 64)["error"]["code"],
        "invalid_onset_cache_token"
    );
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], false);
    assert_eq!(c.document(), before);
    let analysis = tempo_analyze(
        &mut c,
        2,
        tempo_cached_evidence(&repicked, "refined_attack", 0, 192000),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 2, &analysis, 32, 0);
    let failed=c.call("leviathan_onset_v2_prepare",json!({"expected_revision":2,"sample_id":7,"engine":"experimental_v2","preset":"Balanced","picker":{"threshold":1}}));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["previous_cache_retired"], true);
    assert_eq!(
        tempo_points(&mut c, 2, &analysis, 0, 64)["error"]["code"],
        "onset_cache_unavailable"
    );
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], false);
    assert_eq!(c.document(), before);
    let marker = tempo_analyze(
        &mut c,
        2,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let marker_use = tempo_use(&mut c, 2, &marker, 32, 0);
    let cache = onset_prepare(&mut c, 2, "Balanced", json!({}));
    c.ok("leviathan_onset_v2_repick",json!({"expected_revision":2,"cache_token":cache["data"]["cache_token"],"picker":{"enable_phase":true}}));
    assert_eq!(c.call("leviathan_onset_v2_prepare",json!({"expected_revision":2,"sample_id":7,"engine":"experimental_v2","preset":"Balanced","picker":{"threshold":1}}))["ok"],false);
    assert_eq!(tempo_points(&mut c, 2, &marker, 0, 64)["ok"], true);
    assert_eq!(tempo_commit(&mut c, 2, &marker_use)["ok"], true);
}

#[cfg(unix)]
#[test]
fn tempo_cached_analysis_and_review_reauthorize_current_source_bytes_and_recover_without_hidden_writes(
) {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let source = f.root.join("source.wav");
    let bytes = fs::read(&source).unwrap();
    let before = c.document();
    let onset = onset_prepare(
        &mut c,
        2,
        "Balanced",
        json!({"threshold":0.2,"mad_multiplier":2.0}),
    );
    let a = tempo_analyze(
        &mut c,
        2,
        tempo_cached_evidence(&onset, "peak", 0, 192000),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 2, &a, 32, 0);
    fs::write(&source, b"same path changed content").unwrap();
    assert_eq!(tempo_points(&mut c, 2, &a, 0, 64)["ok"], false);
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], false);
    assert_eq!(c.call("leviathan_tempo_analyze",json!({"expected_revision":2,"evidence":tempo_cached_evidence(&onset,"peak",0,192000),"config":tempo_config(1200,1200)}))["ok"],false);
    assert_eq!(c.document(), before);
    fs::write(f.root.join("same.wav"), &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(f.root.join("same.wav"), &source).unwrap();
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], false);
    fs::remove_file(&source).unwrap();
    fs::write(&source, &bytes).unwrap();
    assert_eq!(
        tempo_points(&mut c, 2, &a, 0, 64)["data"]["source_identity_verified_now"],
        true
    );
    assert_eq!(tempo_commit(&mut c, 2, &review)["ok"], true);
    assert_eq!(fs::read(&source).unwrap(), bytes);
}

#[test]
fn tempo_pages_have_exact_totals_end_bounds_and_visible_ambiguity_without_truncating_evidence() {
    use aqua_pattern_core::{song::*, tempo};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    // All 256 observations are explicit accepted markers, not injected analysis evidence.
    let markers = (0..=256).map(|i| i * 750).collect::<Vec<_>>();
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"set_marker_lock","marker_set_id":31,"frame":24000,"locked":false}),
                marker_edit(31, Value::Null, markers)
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let config = tempo::TempoConfig {
        version: 1,
        min_bpm_tenths: 4000,
        max_bpm_tenths: 4000,
    };
    let shared = tempo::analyze_markers(
        &song,
        tempo::MarkerRequest {
            marker_set: MarkerSetId(31),
            start_index: 0,
            end_index: 256,
            include_start: true,
        },
        config,
    )
    .unwrap();
    let a = tempo_analyze(
        &mut c,
        3,
        tempo_marker_evidence(0, 256, true),
        json!(config),
    );
    let mut all = Vec::new();
    for start in [0, 64, 128, 192, 256] {
        let page = tempo_points(&mut c, 3, &a, start, 64);
        assert_eq!(page["ok"], true);
        assert_eq!(page["data"]["page"]["total"], 256);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        all.extend(page["data"]["records"].as_array().unwrap().clone());
    }
    assert_eq!(json!(all), json!(shared.points()));
    for (start, limit) in [(257, 1), (usize::MAX, 1), (0, 0), (0, 65)] {
        assert_eq!(tempo_points(&mut c, 3, &a, start, limit)["ok"], false);
    }
    for candidate in 0..shared.analysis().candidates.len() {
        let exact = shared.pulse_page(candidate, 0, 256).unwrap();
        let end=c.ok("leviathan_tempo_inspect",json!({"expected_revision":3,"analysis_token":a["data"]["analysis_token"],"kind":"pulses","candidate_index":candidate,"start":exact.total}));
        assert_eq!(end["data"]["page"]["frames"], json!([]));
        assert!(end["data"]["page"]["next_start"].is_null());
        assert_eq!(c.call("leviathan_tempo_inspect",json!({"expected_revision":3,"analysis_token":a["data"]["analysis_token"],"kind":"pulses","candidate_index":candidate,"start":exact.total+1}))["ok"],false);
    }
    assert_eq!(c.document(), before);
    let broad = tempo_analyze(
        &mut c,
        3,
        tempo_marker_evidence(0, 256, true),
        tempo_config(200, 4000),
    );
    assert!(broad["data"]["analysis"]["shortlist_truncated"].is_boolean());
    assert!(
        broad["data"]["analysis"]["candidates"]
            .as_array()
            .unwrap()
            .len()
            <= 8
    );
    assert_eq!(broad["data"]["point_count"], 256);
    assert_eq!(c.document(), before);
}

#[test]
fn tempo_use_bpm_exact_pretty_save_boundary_rejects_one_byte_growth_and_preserves_review() {
    use aqua_pattern_core::{project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let markers = vec![0, 7200, 14400, 21600, 28800, 36000, 43200, 50400, 192000];
    let set = song.marker_sets.get_mut(&MarkerSetId(31)).unwrap();
    set.locked_frames.clear();
    set.markers = markers;
    set.source_bpm = None;
    song.marker_sets
        .get_mut(&MarkerSetId(32))
        .unwrap()
        .source_bpm = Some(400.0);
    let mut filler = song.patterns[&PatternId(1)].clone();
    filler.name.clear();
    filler.tracks.clear();
    filler.track_aliases.clear();
    filler.generation = None;
    song.patterns.insert(PatternId(99), filler);
    let original = project::check_serialized_size(&song).unwrap();
    let target = 8 * 1024 * 1024;
    song.patterns.get_mut(&PatternId(99)).unwrap().name = "x".repeat(target - original);
    assert_eq!(project::check_serialized_size(&song).unwrap(), target);
    project::save(
        &song,
        &f.root.join("tempo-limit.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":2,"path":"tempo-limit.leviathan.json","discard_unsaved":true}),
    );
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    let files = fs::read_dir(&f.root).unwrap().count();
    let a = tempo_analyze(
        &mut c,
        3,
        tempo_marker_evidence(0, 8, true),
        tempo_config(4000, 4000),
    );
    let noop = tempo_use(&mut c, 3, &a, 32, 0);
    let failed=c.call("leviathan_tempo_use_bpm_preview",json!({"expected_revision":3,"analysis_token":a["data"]["analysis_token"],"target_marker_set_id":31,"candidate_index":0}));
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("8 MiB"),
        "{failed}"
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
    let applied = tempo_commit(&mut c, 3, &noop);
    assert_eq!(applied["ok"], true);
    assert_eq!(applied["data"]["changed"], false);
    assert_eq!(applied["revision"], 3);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), files);
}

#[test]
fn tempo_native_rate_and_window_budgets_reject_without_fallback_or_destroying_a_valid_analysis() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    write_onset_source(&f.root.join("long.wav"), 8000, 8000 * 301, 1);
    write_onset_source(&f.root.join("low.wav"), 7999, 7999, 1);
    import_source(&mut c, 2, 8, "long.wav");
    import_source(&mut c, 3, 9, "low.wav");
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"upsert_marker_set","id":33,"name":"Exact300second window","sample_id":8,"source_bpm":null,"markers":[0,8000,16000,24000,2400000,2408000]}),
                json!({"op":"upsert_marker_set","id":34,"name":"Low rate stays valid elsewhere","sample_id":9,"source_bpm":null,"markers":[0,1000,2000,3000,7999]})
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let evidence = json!({"kind":"markers","marker_set_id":33,"start_index":0,"end_index":4,"include_start":true});
    let valid = tempo_analyze(&mut c, 5, evidence.clone(), tempo_config(600, 600));
    assert_eq!(
        valid["data"]["analysis"]["binding"]["window_end_frame"],
        2400000
    );
    assert_eq!(
        valid["data"]["analysis"]["candidates"][0]["bpm_tenths"],
        600
    );
    let mut too_long = evidence;
    too_long["end_index"] = json!(5);
    let failed = c.call(
        "leviathan_tempo_analyze",
        json!({"expected_revision":5,"evidence":too_long,"config":tempo_config(600,600)}),
    );
    assert_eq!(failed["ok"], false);
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("300 seconds"));
    let low=c.call("leviathan_tempo_analyze",json!({"expected_revision":5,"evidence":{"kind":"markers","marker_set_id":34,"start_index":0,"end_index":4,"include_start":true},"config":tempo_config(600,600)}));
    assert_eq!(low["ok"], false);
    assert!(low["error"]["message"].as_str().unwrap().contains("8000"));
    assert_eq!(tempo_points(&mut c, 5, &valid, 0, 64)["ok"], true);
    assert_eq!(c.document(), before);
}

#[test]
fn tempo_commit_bounds_large_title_history_responses_before_changed_and_noop_mutation() {
    use aqua_pattern_core::{project, song::Song};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    tempo_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    song.title.clear();
    let original = project::check_serialized_size(&song).unwrap();
    song.title = "x".repeat(8 * 1024 * 1024 - 128 - original);
    project::save(
        &song,
        &f.root.join("large-title.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":2,"path":"large-title.leviathan.json","discard_unsaved":true}),
    );
    let a = tempo_analyze(
        &mut c,
        3,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 3, &a, 32, 0);
    let result = tempo_commit(&mut c, 3, &review);
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["changed"], true);
    assert_eq!(result["data"]["session_scope"], "history_only");
    assert_eq!(
        result["data"]["session"],
        json!({"dirty":true,"can_undo":true,"can_redo":false})
    );
    assert!(result["data"]["session"].get("title").is_none());
    assert!(serde_json::to_vec(&result["data"]).unwrap().len() <= 256 * 1024);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"set_song","title":"Temporary small title"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    let a = tempo_analyze(
        &mut c,
        6,
        tempo_marker_evidence(0, 8, true),
        tempo_config(1200, 1200),
    );
    let review = tempo_use(&mut c, 6, &a, 32, 0);
    let noop = tempo_commit(&mut c, 6, &review);
    assert_eq!(noop["ok"], true);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["revision"], 6);
    assert_eq!(
        noop["data"]["session"],
        json!({"dirty":true,"can_undo":true,"can_redo":true})
    );
    assert!(serde_json::to_vec(&noop["data"]).unwrap().len() <= 256 * 1024);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"marker_sets"}))["data"]["marker_sets"]
            ["32"]["source_bpm"],
        120.0
    );
    c.ok("leviathan_history_redo", json!({"expected_revision":6}));
    let title = c.ok("leviathan_project_read", json!({"section":"summary"}));
    assert_eq!(title["data"]["title"], "Temporary small title");
}

fn feature_setup(c: &mut Client, f: &Fixture) {
    sampler_setup(c, f);
    assert_eq!(
        edit(
            c,
            2,
            vec![region_edit(21, 0, 4800), region_edit(22, 4800, 9600)]
        )["ok"],
        true
    );
}
fn feature_analyze(c: &mut Client, revision: u64, regions: Value, source: Value) -> Value {
    c.call("leviathan_slice_features_analyze",json!({"expected_revision":revision,"request":{"version":1,"sample":7,"regions":regions},"source":source}))
}
fn feature_fresh(c: &mut Client, revision: u64) -> Value {
    let result = feature_analyze(
        c,
        revision,
        json!([21, 22]),
        json!({"kind":"verified_source"}),
    );
    assert_eq!(result["ok"], true, "{result}");
    result
}
fn feature_page(c: &mut Client, revision: u64, a: &Value, start: usize, limit: usize) -> Value {
    c.call("leviathan_slice_features_inspect",json!({"kind":"pending","expected_revision":revision,"analysis_token":a["data"]["analysis_token"],"start":start,"limit":limit}))
}
fn feature_stored(c: &mut Client, revision: u64, start: usize, limit: usize) -> Value {
    c.call("leviathan_slice_features_inspect",json!({"kind":"stored","expected_revision":revision,"sample_id":7,"start":start,"limit":limit}))
}
fn feature_review(c: &mut Client, revision: u64, a: &Value, roles: Value) -> Value {
    c.call("leviathan_slice_catalog_preview",json!({"expected_revision":revision,"analysis_token":a["data"]["analysis_token"],"roles":roles}))
}
fn feature_commit(c: &mut Client, revision: u64, review: &Value) -> Value {
    c.call(
        "leviathan_slice_catalog_commit",
        json!({"expected_revision":revision,"preview_token":review["data"]["preview_token"]}),
    )
}
fn feature_label(role: &str, articulation: &str, note: &str) -> Value {
    json!({"role":role,"articulation":articulation,"note":note})
}
#[test]
fn catalog_verified_measurements_roles_editor_parity_one_undo_and_save_export_roundtrip() {
    use aqua_pattern_core::{editor::Editor, slice_features as sf, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let source_bytes = fs::read(f.root.join("kit.wav")).unwrap();
    let plan = sf::prepare(
        &song,
        sf::FeatureRequest {
            version: 1,
            sample: SampleId(7),
            regions: vec![RegionId(21), RegionId(22)],
        },
    )
    .unwrap();
    let exact = sf::analyze_verified(&plan, &f.root.join("kit.wav")).unwrap();
    let a = feature_fresh(&mut c, 3);
    assert_eq!(a["data"]["summary"], json!(exact.summary()));
    assert_eq!(a["data"]["analysis_id"], exact.id());
    assert_eq!(
        a["data"]["summary"]["layout"]["shape_support_floor_relative"],
        json!(1e-24)
    );
    let first = feature_page(&mut c, 3, &a, 0, 1);
    let last = feature_page(&mut c, 3, &a, 1, 1);
    assert_eq!(first["data"]["records"][0], json!(exact.records()[0]));
    assert_eq!(last["data"]["records"][0], json!(exact.records()[1]));
    assert_eq!(first["data"]["source_identity_verified_now"], false);
    assert!(first["data"]["records"][0]["snapshot"]["metrics"]["supported_shape_windows"].is_u64());
    assert_eq!(
        feature_stored(&mut c, 3, 0, 16)["data"]["records"][0]["entry"],
        Value::Null
    );
    assert_eq!(c.document(), before);
    let choices =
        json!([{"region":21,"label":feature_label("kick","ghost","Explicit contextual review")}]);
    let reviewed = exact
        .catalog_preview(serde_json::from_value(choices.clone()).unwrap())
        .unwrap();
    let r = feature_review(&mut c, 3, &a, choices);
    assert_eq!(r["data"]["summary"], json!(reviewed.summary()));
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor
        .commit_region_catalog_preview(editor.revision(), &reviewed)
        .unwrap();
    let applied = feature_commit(&mut c, 3, &r);
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["data"]["session_scope"], "history_only");
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let identical = edit(
        &mut c,
        4,
        vec![
            json!({"op":"set_region_role","region_id":21,"label":feature_label("kick","ghost","Explicit contextual review")}),
        ],
    );
    assert_eq!(identical["ok"], true);
    assert_eq!(identical["revision"], 4);
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    assert_eq!(
        song_render::evaluate(&song, None).unwrap(),
        song_render::evaluate(&editor.song, None).unwrap()
    );
    assert!(editor.song.region_catalog[&RegionId(22)]
        .reviewed_role
        .is_none());
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":4,"path":"catalog.wav"}),
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(song));
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":5,"path":"without-catalog.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("catalog.wav")).unwrap(),
        fs::read(f.root.join("without-catalog.wav")).unwrap()
    );
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"catalog.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"catalog.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let stored = feature_stored(&mut c, 8, 0, 16);
    assert_eq!(
        stored["data"]["records"][0],
        json!(sf::inspect_catalog(&editor.song, RegionId(21)).unwrap())
    );
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), source_bytes);
}
#[test]
fn catalog_cached_pcm_matches_fresh_measurements_and_detaches_from_picker_epochs() {
    use aqua_pattern_core::{onset_service, onset_v2, slice_features as sf, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let onset = onset_prepare(&mut c, 3, "Balanced", json!({}));
    let config: onset_v2::Config =
        serde_json::from_value(onset["data"]["effective_config"].clone()).unwrap();
    let asset = &song.samples[&SampleId(7)];
    let cached = onset_service::prepare_verified(asset, &asset.path, &config).unwrap();
    let plan = sf::prepare(
        &song,
        sf::FeatureRequest {
            version: 1,
            sample: SampleId(7),
            regions: vec![RegionId(21), RegionId(22)],
        },
    )
    .unwrap();
    let exact = sf::analyze_cached(&plan, &cached.cache).unwrap();
    assert_eq!(
        exact.records(),
        sf::analyze_verified(&plan, &asset.path).unwrap().records()
    );
    let a = feature_analyze(
        &mut c,
        3,
        json!([21, 22]),
        json!({"kind":"cached_onsets","cache_token":onset["data"]["cache_token"]}),
    );
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["data"]["analysis_id"], exact.id());
    assert_eq!(
        feature_page(&mut c, 3, &a, 0, 16)["data"]["records"],
        json!(exact.records())
    );
    let r = feature_review(&mut c, 3, &a, json!([]));
    assert_eq!(r["ok"], true);
    c.ok("leviathan_onset_v2_repick",json!({"expected_revision":3,"cache_token":onset["data"]["cache_token"],"picker":{"enable_phase":true}}));
    assert_eq!(feature_page(&mut c, 3, &a, 0, 16)["ok"], true);
    let retired=c.call("leviathan_onset_v2_prepare",json!({"expected_revision":3,"sample_id":7,"engine":"experimental_v2","preset":"Balanced","picker":{"threshold":1}}));
    assert_eq!(retired["ok"], false);
    assert_eq!(retired["error"]["details"]["previous_cache_retired"], true);
    assert_eq!(c.document(), before);
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], true);
}
#[test]
fn catalog_strict_selections_roles_and_injection_errors_preserve_previous_reviews() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let a = feature_fresh(&mut c, 3);
    let r = feature_review(&mut c, 3, &a, json!([]));
    let before = c.document();
    for regions in [
        json!([]),
        json!([21, 21]),
        json!([22, 21]),
        json!([999]),
        json!((1..=65).collect::<Vec<_>>()),
    ] {
        let e = feature_analyze(&mut c, 3, regions, json!({"kind":"verified_source"}));
        assert_eq!(e["ok"], false, "{e}");
    }
    for source in [
        json!({"kind":"verified_source","pcm":[0]}),
        json!({"kind":"cached_onsets","cache_token":"fake"}),
        json!({"kind":"other"}),
    ] {
        assert_eq!(feature_analyze(&mut c, 3, json!([21]), source)["ok"], false);
    }
    for roles in [
        json!([{"region":999,"label":feature_label("kick","normal","")}]),
        json!([{"region":21,"label":feature_label("kick","normal","")},{"region":21,"label":feature_label("snare","ghost","")}]),
        json!([{"region":21,"label":feature_label("tom","normal","")}]),
        json!([{"region":21,"label":feature_label("kick","automatic","")}]),
        json!([{"region":21,"label":feature_label("kick","ghost",&"é".repeat(129))}]),
    ] {
        assert_eq!(feature_review(&mut c, 3, &a, roles)["ok"], false);
    }
    for args in [
        json!({"expected_revision":3,"analysis_token":a["data"]["analysis_token"]}),
        json!({"expected_revision":3,"analysis_token":a["data"]["analysis_token"],"roles":[],"metrics":{"peak":1}}),
    ] {
        assert_eq!(c.call("leviathan_slice_catalog_preview", args)["ok"], false);
    }
    assert_eq!(c.document(), before);
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], true);
}
#[test]
fn catalog_manual_labels_required_nullability_and_clear_operations_share_editor_semantics() {
    use aqua_pattern_core::{editor::Editor, slice_features as sf, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let a = feature_fresh(&mut c, 3);
    let r = feature_review(&mut c, 3, &a, json!([]));
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], true);
    let before = c.document();
    let mut editor = Editor::default();
    editor
        .replace_document(serde_json::from_value(before["data"]["song"].clone()).unwrap())
        .unwrap();
    editor.edit_mode = true;
    let label = feature_label("unknown", "ghost", &"é".repeat(128));
    let typed: sf::RoleLabel = serde_json::from_value(label.clone()).unwrap();
    editor.set_region_role(RegionId(21), Some(typed)).unwrap();
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"set_region_role","region_id":21,"label":label})]
        )["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    let now = c.document();
    for op in [
        json!({"op":"set_region_role","region_id":21}),
        json!({"op":"set_region_role","region_id":21,"label":feature_label("kick","ghost",&"é".repeat(129))}),
        json!({"op":"clear_region_features","region_id":999}),
    ] {
        assert_eq!(edit(&mut c, 5, vec![op])["ok"], false);
        assert_eq!(c.document(), now);
    }
    editor.clear_region_features(RegionId(21)).unwrap();
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![json!({"op":"clear_region_features","region_id":21})]
        )["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    assert!(editor.song.region_catalog[&RegionId(21)].features.is_none());
    assert!(editor.song.region_catalog[&RegionId(21)]
        .reviewed_role
        .as_ref()
        .unwrap()
        .based_on_feature_sha256
        .is_none());
    editor.set_region_role(RegionId(21), None).unwrap();
    assert_eq!(
        edit(
            &mut c,
            6,
            vec![json!({"op":"set_region_role","region_id":21,"label":null})]
        )["ok"],
        true
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    assert!(!editor.song.region_catalog.contains_key(&RegionId(21)));
}
#[test]
fn catalog_binding_staleness_preserves_roles_and_native_playback_metadata_is_independent() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let a = feature_fresh(&mut c, 3);
    let roles = json!([{"region":21,"label":feature_label("kick","normal","")}]);
    let r = feature_review(&mut c, 3, &a, roles);
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], true);
    let original = c.document()["data"]["song"]["region_catalog"].clone();
    let mut rename = region_edit(21, 0, 4800);
    rename["name"] = json!("renamed");
    rename["root_note"] = json!(64);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                rename,
                json!({"op":"set_region_playback","region_id":21,"amp_envelope":{"attack_ms":2,"decay_ms":3,"sustain_percent":50,"release_ms":7},"forward_loop":{"start_frame":20,"end_frame":100}})
            ]
        )["ok"],
        true
    );
    let current = feature_stored(&mut c, 5, 0, 16);
    assert_eq!(
        current["data"]["records"][0]["features_current_match"],
        true
    );
    assert_eq!(
        current["data"]["records"][0]["reviewed_role_current_match"],
        true
    );
    assert_eq!(edit(&mut c, 5, vec![region_edit(21, 0, 4000)])["ok"], true);
    assert_eq!(c.document()["data"]["song"]["region_catalog"], original);
    let stale = feature_stored(&mut c, 6, 0, 16);
    assert_eq!(stale["data"]["records"][0]["features_current_match"], false);
    assert_eq!(
        stale["data"]["records"][0]["reviewed_role_current_match"],
        false
    );
    let a = feature_fresh(&mut c, 6);
    let r = feature_review(&mut c, 6, &a, json!([]));
    assert_eq!(r["data"]["summary"]["preserved_review_count"], 1);
    assert_eq!(feature_commit(&mut c, 6, &r)["ok"], true);
    let stored = feature_stored(&mut c, 7, 0, 16);
    assert_eq!(stored["data"]["records"][0]["features_current_match"], true);
    assert_eq!(
        stored["data"]["records"][0]["reviewed_role_current_match"],
        false
    );
    let before = c.document();
    assert_eq!(
        edit(&mut c, 7, vec![json!({"op":"remove_region","id":21})])["ok"],
        false
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(&mut c, 7, vec![json!({"op":"remove_region","id":22})])["ok"],
        true
    );
    assert!(c.document()["data"]["song"]["region_catalog"]
        .get("22")
        .is_none());
}
#[cfg(unix)]
#[test]
fn catalog_historical_inspection_survives_tamper_missing_and_symlink_but_store_reverifies() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let a = feature_fresh(&mut c, 3);
    let r = feature_review(&mut c, 3, &a, json!([]));
    let before = c.document();
    let path = f.root.join("kit.wav");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, b"changed source").unwrap();
    assert_eq!(feature_page(&mut c, 3, &a, 0, 16)["ok"], true);
    assert_eq!(feature_stored(&mut c, 3, 0, 16)["ok"], true);
    assert_eq!(feature_review(&mut c, 3, &a, json!([]))["ok"], false);
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], false);
    fs::remove_file(&path).unwrap();
    assert_eq!(feature_page(&mut c, 3, &a, 0, 16)["ok"], true);
    assert_eq!(feature_stored(&mut c, 3, 0, 16)["ok"], true);
    let outside = f.base.join("outside.wav");
    fs::write(&outside, &bytes).unwrap();
    symlink(&outside, &path).unwrap();
    assert_eq!(
        feature_commit(&mut c, 3, &r)["error"]["code"],
        "path_denied"
    );
    assert_eq!(
        feature_analyze(&mut c, 3, json!([21]), json!({"kind":"verified_source"}))["ok"],
        false
    );
    assert_eq!(c.document(), before);
    fs::remove_file(&path).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], true);
    fs::remove_file(&path).unwrap();
    let stored = feature_stored(&mut c, 4, 0, 16);
    assert_eq!(stored["ok"], true);
    assert_eq!(stored["data"]["source_identity_verified_now"], false);
    assert_eq!(stored["data"]["records"][0]["features_current_match"], true);
}
#[test]
fn catalog_tokens_aba_supersession_failed_preview_and_noop_redo_are_atomic() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let a = feature_fresh(&mut c, 3);
    let r = feature_review(&mut c, 3, &a, json!([]));
    let second = feature_fresh(&mut c, 3);
    assert_eq!(feature_page(&mut c, 3, &a, 0, 16)["ok"], false);
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], false);
    let r = feature_review(&mut c, 3, &second, json!([]));
    assert_eq!(feature_commit(&mut c, 3, &r)["ok"], true);
    let original = c.document()["data"]["song"].clone();
    let a = feature_fresh(&mut c, 4);
    let r = feature_review(&mut c, 4, &a, json!([]));
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"set_song","title":"temporary"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], original);
    assert_eq!(feature_commit(&mut c, 6, &r)["ok"], false);
    let a = feature_fresh(&mut c, 6);
    let r = feature_review(&mut c, 6, &a, json!([]));
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    assert_eq!(
        feature_review(
            &mut c,
            6,
            &a,
            json!([{"region":999,"label":feature_label("unknown","unspecified","")}])
        )["ok"],
        false
    );
    let noop = feature_commit(&mut c, 6, &r);
    assert_eq!(noop["ok"], true);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["revision"], 6);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
    assert_eq!(feature_commit(&mut c, 6, &r)["ok"], false);
    c.ok("leviathan_history_redo", json!({"expected_revision":6}));
    assert_eq!(c.document()["data"]["song"]["title"], "temporary");
}
#[test]
fn catalog_pages_bound_exact_records_and_reject_invalid_ranges_without_mutation() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let ops = (23..=84).map(|id| region_edit(id, 0, 1)).collect();
    assert_eq!(edit(&mut c, 3, ops)["ok"], true);
    let regions = (21..=84).collect::<Vec<_>>();
    let a = feature_analyze(&mut c, 4, json!(regions), json!({"kind":"verified_source"}));
    assert_eq!(a["ok"], true, "{a}");
    let before = c.document();
    let mut records = vec![];
    for start in (0..64).step_by(7) {
        let page = feature_page(&mut c, 4, &a, start, 7);
        assert_eq!(page["ok"], true);
        assert_eq!(page["data"]["page"]["total"], 64);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        records.extend(page["data"]["records"].as_array().unwrap().clone());
    }
    assert_eq!(records.len(), 64);
    for (i, record) in records.iter().enumerate() {
        assert_eq!(record["region"], 21 + i as u64);
    }
    assert_eq!(
        feature_page(&mut c, 4, &a, 64, 16)["data"]["records"],
        json!([])
    );
    for (start, limit) in [(65, 1), (usize::MAX, 1), (0, 0), (0, 17)] {
        assert_eq!(feature_page(&mut c, 4, &a, start, limit)["ok"], false);
        assert_eq!(feature_stored(&mut c, 4, start, limit)["ok"], false);
    }
    for args in [
        json!({"kind":"stored","expected_revision":4,"sample_id":7,"analysis_token":"x"}),
        json!({"kind":"pending","expected_revision":4,"analysis_token":a["data"]["analysis_token"],"sample_id":7}),
    ] {
        assert_eq!(
            c.call("leviathan_slice_features_inspect", args)["ok"],
            false
        );
    }
    assert_eq!(c.document(), before);
}

#[test]
fn catalog_rate_duration_and_full_scan_work_limits_fail_before_replacing_review() {
    use aqua_pattern_core::{project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    write_onset_source(&f.root.join("long.wav"), 8000, 248001, 1);
    import_source(&mut c, 0, 7, "long.wav");
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![region_edit(21, 0, 240000), region_edit(22, 0, 240001)]
        )["ok"],
        true
    );
    let a = feature_analyze(&mut c, 2, json!([21]), json!({"kind":"verified_source"}));
    assert_eq!(a["ok"], true, "{a}");
    let before = c.document();
    let e = feature_analyze(&mut c, 2, json!([22]), json!({"kind":"verified_source"}));
    assert_eq!(e["ok"], false);
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("30 seconds"));
    assert_eq!(c.document(), before);
    assert_eq!(feature_page(&mut c, 2, &a, 0, 16)["ok"], true);
    let mut song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    song.regions.remove(&RegionId(22));
    for id in 22..=84 {
        let region = song.regions[&RegionId(21)].clone();
        song.regions.insert(RegionId(id), region);
    }
    project::save(
        &song,
        &f.root.join("work.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":2,"path":"work.leviathan.json","discard_unsaved":true}),
    );
    let e = feature_analyze(
        &mut c,
        3,
        json!((21..=84).collect::<Vec<_>>()),
        json!({"kind":"verified_source"}),
    );
    assert_eq!(e["ok"], false);
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("full-scan cap"),
        "{e}"
    );
    write_onset_source(&f.root.join("low.wav"), 7999, 7999, 1);
    import_source(&mut c, 3, 8, "low.wav");
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"upsert_region","id":99,"name":"low","sample_id":8,"start_frame":0,"end_frame":7999,"root_note":60})
            ]
        )["ok"],
        true
    );
    let e=c.call("leviathan_slice_features_analyze",json!({"expected_revision":5,"request":{"version":1,"sample":8,"regions":[99]},"source":{"kind":"verified_source"}}));
    assert_eq!(e["ok"], false);
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("8000..384000"));
    assert_eq!(
        feature_analyze(
            &mut c,
            5,
            json!([21, 99]),
            json!({"kind":"verified_source"})
        )["ok"],
        false
    );
}
#[test]
fn catalog_persisted_metadata_is_not_attestation_and_full_pages_stay_bounded() {
    use aqua_pattern_core::{project, slice_features as sf, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let plan = sf::prepare(
        &song,
        sf::FeatureRequest {
            version: 1,
            sample: SampleId(7),
            regions: vec![RegionId(21)],
        },
    )
    .unwrap();
    let computed = sf::analyze_verified(&plan, &f.root.join("kit.wav")).unwrap();
    let mut snapshot = computed.records()[0].snapshot.clone();
    // Validly shaped, deliberately user-authored historical annotations are not proof of computation.
    snapshot.metrics.peak *= 2.0;
    snapshot.metrics.rms *= 2.0;
    for dc in &mut snapshot.metrics.dc_per_channel {
        *dc *= 2.0;
    }
    snapshot.metrics.analysed_power = snapshot.metrics.analysed_power.map(|p| p * 4.0);
    for id in 21..=36 {
        song.regions
            .insert(RegionId(id), song.regions[&RegionId(21)].clone());
        song.region_catalog.insert(
            RegionId(id),
            sf::RegionCatalogEntry {
                features: Some(snapshot.clone()),
                reviewed_role: None,
            },
        );
    }
    song.validate().unwrap();
    project::save(
        &song,
        &f.root.join("historical.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":3,"path":"historical.leviathan.json","discard_unsaved":true}),
    );
    fs::remove_file(f.root.join("kit.wav")).unwrap();
    let before = c.document();
    let full = feature_stored(&mut c, 4, 0, 16);
    assert_eq!(full["ok"], true);
    assert_eq!(full["data"]["records"].as_array().unwrap().len(), 16);
    assert!(serde_json::to_vec(&full["data"]).unwrap().len() <= 256 * 1024);
    let small = feature_stored(&mut c, 4, 0, 1);
    assert_eq!(small["ok"], true);
    assert_eq!(small["data"]["source_identity_verified_now"], false);
    assert_eq!(small["data"]["metrics_recomputed_now"], false);
    assert_eq!(small["data"]["records"][0]["features_current_match"], true);
    assert_eq!(
        small["data"]["records"][0]["entry"]["features"],
        json!(snapshot)
    );
    assert_eq!(c.document(), before);
}
#[test]
fn catalog_large_title_commit_is_bounded_and_exact_pretty_limit_preserves_noop_review() {
    use aqua_pattern_core::{project, song::Song};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    song.title = "x".repeat(6 * 1024 * 1024);
    project::save(
        &song,
        &f.root.join("large.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":3,"path":"large.leviathan.json","discard_unsaved":true}),
    );
    let a = feature_fresh(&mut c, 4);
    let r = feature_review(&mut c, 4, &a, json!([]));
    let result = feature_commit(&mut c, 4, &r);
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["changed"], true);
    assert_eq!(result["data"]["session_scope"], "history_only");
    assert!(result["data"]["session"].get("title").is_none());
    assert!(serde_json::to_vec(&result["data"]).unwrap().len() < 256 * 1024);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    song.title.clear();
    let size = project::check_serialized_size(&song).unwrap();
    song.title = "x".repeat(8 * 1024 * 1024 - size);
    assert_eq!(
        project::check_serialized_size(&song).unwrap(),
        8 * 1024 * 1024
    );
    project::save(
        &song,
        &f.root.join("limit.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":5,"path":"limit.leviathan.json","discard_unsaved":true}),
    );
    let a = feature_fresh(&mut c, 6);
    let r = feature_review(&mut c, 6, &a, json!([]));
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    let failed = feature_review(
        &mut c,
        6,
        &a,
        json!([{"region":21,"label":feature_label("kick","normal","")}]),
    );
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("8 MiB"),
        "{failed}"
    );
    let noop = feature_commit(&mut c, 6, &r);
    assert_eq!(noop["ok"], true);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["revision"], 6);
    assert!(serde_json::to_vec(&noop["data"]).unwrap().len() < 256 * 1024);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
}
#[test]
fn catalog_256_entry_limit_is_independent_of_analysis_pages_and_all_removals_are_explicit() {
    use aqua_pattern_core::{project, slice_features as sf, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    feature_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    for id in 21..=277 {
        let region = song.regions[&RegionId(21)].clone();
        song.regions.insert(RegionId(id), region);
    }
    for id in 21..=276 {
        song.set_region_role(
            RegionId(id),
            Some(sf::RoleLabel {
                role: sf::PercussionRole::Unknown,
                articulation: sf::Articulation::Unspecified,
                note: String::new(),
            }),
        )
        .unwrap();
    }
    assert_eq!(song.region_catalog.len(), 256);
    project::save(
        &song,
        &f.root.join("full.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":3,"path":"full.leviathan.json","discard_unsaved":true}),
    );
    let before = c.document();
    let e = feature_analyze(&mut c, 4, json!([277]), json!({"kind":"verified_source"}));
    assert_eq!(e["ok"], false);
    assert!(e["error"]["message"].as_str().unwrap().contains("256"));
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"set_region_role","region_id":277,"label":feature_label("unknown","normal","")})
            ]
        )["ok"],
        false
    );
    assert_eq!(c.document(), before);
    let last = feature_stored(&mut c, 4, 256, 16);
    assert_eq!(last["data"]["page"]["total"], 257);
    assert_eq!(last["data"]["records"][0]["region"], 277);
    assert_eq!(last["data"]["records"][0]["entry"], Value::Null);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"set_region_role","region_id":276,"label":null})]
        )["ok"],
        true
    );
    assert_eq!(
        feature_analyze(&mut c, 5, json!([277]), json!({"kind":"verified_source"}))["ok"],
        true
    );
}

fn thin_setup(c: &mut Client, f: &Fixture) {
    sampler_setup(c, f);
    let mut operations = vec![
        region_edit(22, 0, 48000),
        json!({"op":"assign_region","instrument_id":2,"region_id":22}),
        json!({"op":"set_region_role","region_id":21,"label":feature_label("kick","normal","")}),
        json!({"op":"set_region_role","region_id":22,"label":feature_label("closed_hat","ghost","Explicit human review")}),
        json!({"op":"clear_pattern_track","pattern_id":1,"track_id":1}),
        json!({"op":"upsert_pattern","id":1,"name":"Thin ordinary cells","rows":12}),
    ];
    for row in 0..8 {
        operations.push(json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":row,"column":0,"cell":{"note":{"Note":60},"instrument":if row%2==0 {1} else {2},"velocity":0.5,"pan":0.0}}));
    }
    operations.push(kit_cell(1, 8, json!("Off"), 1));
    assert_eq!(edit(c, 2, operations)["ok"], true);
}
fn thin_config(percent: u8, seed: u64) -> Value {
    json!({"version":1,"scope":{"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[0]},"seed":seed,"removal_percent":percent,"max_removed":4096,"protected_roles":["kick","snare"]})
}
fn thin_preview(c: &mut Client, revision: u64, config: Value) -> Value {
    c.call(
        "leviathan_thin_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn thin_page(
    c: &mut Client,
    revision: u64,
    p: &Value,
    kind: &str,
    start: usize,
    limit: usize,
) -> Value {
    c.call("leviathan_thin_inspect",json!({"expected_revision":revision,"preview_token":p["data"]["preview_token"],"kind":kind,"start":start,"limit":limit}))
}
fn thin_commit(c: &mut Client, revision: u64, p: &Value) -> Value {
    c.call(
        "leviathan_thin_commit",
        json!({"expected_revision":revision,"preview_token":p["data"]["preview_token"]}),
    )
}
#[test]
fn thin_exact_shared_preview_editor_pages_undo_and_ordinary_save_export_roundtrip() {
    use aqua_pattern_core::{editor::Editor, mutations, song::*, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let config = thin_config(100, 42);
    let exact = mutations::preview(&song, serde_json::from_value(config.clone()).unwrap()).unwrap();
    let p = thin_preview(&mut c, 3, config);
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["data"]["preview_id"], exact.id());
    assert_eq!(p["data"]["summary"], json!(exact.summary()));
    assert_eq!(p["data"]["record"], json!(exact.record()));
    assert_eq!(p["data"]["source_identity_verified"], false);
    assert_eq!(p["data"]["summary"]["eligible_cells"], 4);
    assert_eq!(p["data"]["summary"]["removed_cells"], 4);
    for (kind, expected) in [
        ("decisions", json!(exact.decisions())),
        ("uses", json!(exact.uses())),
        ("contexts", json!(exact.contexts())),
        ("changes", json!(exact.changes())),
    ] {
        let mut records = vec![];
        let mut start = 0;
        loop {
            let page = thin_page(&mut c, 3, &p, kind, start, 3);
            assert_eq!(page["ok"], true);
            records.extend(page["data"]["records"].as_array().unwrap().clone());
            if let Some(next) = page["data"]["page"]["next_start"].as_u64() {
                start = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(json!(records), expected);
    }
    assert_eq!(c.document(), before);
    let mut editor = Editor::default();
    editor.replace_document(song.clone()).unwrap();
    editor.edit_mode = true;
    editor
        .commit_thin_preview(editor.revision(), &exact)
        .unwrap();
    let applied = thin_commit(&mut c, 3, &p);
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["data"]["session_scope"], "history_only");
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    for row in 0..8 {
        assert_eq!(
            editor.song.patterns[&PatternId(1)].tracks[&TrackId(1)][&row].contains_key(&0),
            row % 2 == 0
        );
    }
    assert_eq!(
        song_render::evaluate(&editor.song, None).unwrap(),
        song_render::evaluate(exact.candidate(), None).unwrap()
    );
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":4,"path":"thin.wav"}),
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], json!(song));
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"thin.leviathan.json"}),
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"thin.leviathan.json","discard_unsaved":true}),
    );
    assert_eq!(c.document()["data"]["song"], json!(editor.song));
    c.ok(
        "leviathan_song_export",
        json!({"expected_revision":8,"path":"reopened-thin.wav"}),
    );
    assert_eq!(
        fs::read(f.root.join("thin.wav")).unwrap(),
        fs::read(f.root.join("reopened-thin.wav")).unwrap()
    );
    assert_eq!(
        fs::read(f.root.join("thin.events.json")).unwrap(),
        fs::read(f.root.join("reopened-thin.events.json")).unwrap()
    );
}
#[test]
fn thin_real_pinned_pattrns_gate_matches_explicit_seeds_percent_endpoints_and_canonical_cells() {
    use pattrns::{gates::ProbabilityGate, Gate, RhythmEvent};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let before = c.document();
    for seed in [0, 1, 42, u64::MAX] {
        for percent in [0, 25, 50, 75, 100] {
            let config = thin_config(percent, seed);
            let p = thin_preview(&mut c, 3, config.clone());
            assert_eq!(p["ok"], true, "{p}");
            let mut gate = ProbabilityGate::new(Some(seed));
            let pulse = RhythmEvent {
                value: (f64::from(percent) / 100.0) as f32,
                step_time: 1.0,
            };
            let expected: Vec<_> = (1..8)
                .step_by(2)
                .filter(|_| percent != 0 && gate.run(&pulse))
                .collect();
            let changes = thin_page(&mut c, 3, &p, "changes", 0, 64);
            let rows: Vec<_> = changes["data"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["row"].as_u64().unwrap() as usize)
                .collect();
            assert_eq!(rows, expected);
            assert_eq!(
                p["data"]["summary"]["gate_calls"],
                if percent == 0 { 0 } else { 4 }
            );
            assert_eq!(
                p["data"]["summary"]["random_draws"],
                if percent == 0 || percent == 100 { 0 } else { 4 }
            );
            let again = thin_preview(&mut c, 3, config);
            assert_eq!(again["data"]["record"], p["data"]["record"]);
            assert_eq!(again["data"]["preview_id"], p["data"]["preview_id"]);
        }
    }
    assert_eq!(c.document(), before);
}
#[test]
fn thin_strict_integer_parameters_roles_unknown_fields_and_removal_cap_preserve_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let good = thin_preview(&mut c, 3, thin_config(0, 42));
    assert_eq!(good["ok"], true);
    let before = c.document();
    for (key, value) in [
        ("version", json!(2)),
        ("removal_percent", json!(-1)),
        ("removal_percent", json!(101)),
        ("removal_percent", json!(0.5)),
        ("removal_percent", json!(100.0000000001)),
        ("max_removed", json!(4097)),
        ("protected_roles", json!(["unknown"])),
        ("protected_roles", json!(["mixed"])),
        ("protected_roles", json!(["kick", "kick"])),
        ("seed", json!(-1)),
    ] {
        let mut config = thin_config(50, 42);
        config[key] = value;
        assert_eq!(thin_preview(&mut c, 3, config)["ok"], false);
    }
    for key in [
        "version",
        "scope",
        "seed",
        "removal_percent",
        "max_removed",
        "protected_roles",
    ] {
        let mut config = thin_config(0, 42);
        config.as_object_mut().unwrap().remove(key);
        assert_eq!(thin_preview(&mut c, 3, config)["ok"], false);
    }
    for key in ["candidate", "random_values", "probability", "role_scores"] {
        let mut config = thin_config(0, 42);
        config[key] = json!([]);
        assert_eq!(thin_preview(&mut c, 3, config)["ok"], false);
    }
    let mut limited = thin_config(100, 42);
    limited["max_removed"] = json!(3);
    let e = thin_preview(&mut c, 3, limited);
    assert_eq!(e["ok"], false);
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("max_removed"));
    assert_eq!(c.document(), before);
    assert_eq!(thin_commit(&mut c, 3, &good)["data"]["changed"], false);
}
#[test]
fn thin_unreviewed_unknown_mixed_and_stale_are_retained_even_without_optional_role_protection() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let mut revision = 3;
    for label in [
        Value::Null,
        feature_label("unknown", "normal", ""),
        feature_label("mixed", "ghost", ""),
    ] {
        assert_eq!(
            edit(
                &mut c,
                revision,
                vec![json!({"op":"set_region_role","region_id":22,"label":label})]
            )["ok"],
            true
        );
        revision += 1;
        let p = thin_preview(&mut c, revision, thin_config(100, 42));
        assert_eq!(p["ok"], true, "{p}");
        assert_eq!(p["data"]["summary"]["removed_cells"], 0);
    }
    assert_eq!(
        edit(
            &mut c,
            revision,
            vec![
                json!({"op":"set_region_role","region_id":22,"label":feature_label("closed_hat","ghost","")}),
                region_edit(22, 1, 48000)
            ]
        )["ok"],
        true
    );
    revision += 1;
    let mut config = thin_config(100, 42);
    config["protected_roles"] = json!([]);
    let p = thin_preview(&mut c, revision, config);
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["data"]["summary"]["removed_cells"], 4);
    let decisions = thin_page(&mut c, revision, &p, "decisions", 0, 64);
    for d in decisions["data"]["records"].as_array().unwrap() {
        if d["row"].as_u64().unwrap() % 2 == 1 {
            assert_eq!(d["reason"], "stale_review");
            assert_eq!(d["gate_index"], Value::Null);
        }
    }
}
#[test]
fn thin_repeated_alias_uses_are_shared_while_alias_target_writes_and_scope_errors_reject() {
    use aqua_pattern_core::{mutations, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_pattern","id":2,"name":"Alias target","rows":12}),
                json!({"op":"set_track_alias","pattern_id":2,"track_id":2,"source":{"pattern":1,"track":1}}),
                json!({"op":"set_order","pattern_ids":[1,2,1]}),
                json!({"op":"set_occurrence_track_mute","occurrence_index":2,"track_id":1,"muted":true})
            ]
        )["ok"],
        true
    );
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let exact =
        mutations::preview(&song, serde_json::from_value(thin_config(100, 42)).unwrap()).unwrap();
    let p = thin_preview(&mut c, 4, thin_config(100, 42));
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["data"]["summary"], json!(exact.summary()));
    assert_eq!(p["data"]["summary"]["gate_calls"], 4);
    assert_eq!(p["data"]["summary"]["removed_starts"], 8);
    assert_eq!(
        thin_page(&mut c, 4, &p, "contexts", 0, 64)["data"]["records"],
        json!(exact.contexts())
    );
    let before = c.document();
    let mut alias = thin_config(100, 42);
    alias["scope"]["pattern"] = json!(2);
    alias["scope"]["track"] = json!(2);
    assert_eq!(thin_preview(&mut c, 4, alias)["ok"], false);
    for scope in [
        json!({"pattern":1,"track":1,"row_start":8,"row_end":8,"columns":[0]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":13,"columns":[0]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[0,0]}),
        json!({"pattern":1,"track":1,"row_start":0,"row_end":8,"columns":[999]}),
    ] {
        let mut config = thin_config(0, 42);
        config["scope"] = scope;
        assert_eq!(thin_preview(&mut c, 4, config)["ok"], false);
    }
    assert_eq!(c.document(), before);
    assert_eq!(thin_commit(&mut c, 4, &p)["ok"], true);
}
#[test]
fn thin_page_kinds_bounds_supersession_aba_and_true_noop_preserve_redo() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let first = thin_preview(&mut c, 3, thin_config(100, 42));
    let second = thin_preview(&mut c, 3, thin_config(0, 42));
    assert_eq!(thin_page(&mut c, 3, &first, "changes", 0, 64)["ok"], false);
    for (kind, total) in [
        ("decisions", 8),
        ("uses", 8),
        ("contexts", 1),
        ("changes", 0),
    ] {
        let end = thin_page(&mut c, 3, &second, kind, total, 64);
        assert_eq!(end["ok"], true);
        assert_eq!(end["data"]["records"], json!([]));
        for (start, limit) in [(total + 1, 1), (usize::MAX, 1), (0, 0), (0, 65)] {
            assert_eq!(
                thin_page(&mut c, 3, &second, kind, start, limit)["ok"],
                false
            );
        }
    }
    assert_eq!(thin_page(&mut c, 3, &second, "audio", 0, 1)["ok"], false);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![json!({"op":"set_song","title":"temporary thin"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(thin_commit(&mut c, 5, &second)["ok"], false);
    let p = thin_preview(&mut c, 5, thin_config(0, 42));
    let before = c.ok("leviathan_project_read", json!({"section":"history"}));
    let noop = thin_commit(&mut c, 5, &p);
    assert_eq!(noop["ok"], true);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["revision"], 5);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        before
    );
    assert_eq!(thin_commit(&mut c, 5, &p)["ok"], false);
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"]["title"], "temporary thin");
}
#[cfg(unix)]
#[test]
fn thin_authorizes_paths_without_claiming_hashes_and_retries_after_symlink_restoration() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let p = thin_preview(&mut c, 3, thin_config(100, 42));
    assert_eq!(p["ok"], true);
    let path = f.root.join("kit.wav");
    let bytes = fs::read(&path).unwrap();
    let outside = f.base.join("same-bytes.wav");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&outside, &path).unwrap();
    let before = c.document();
    assert_eq!(
        thin_page(&mut c, 3, &p, "changes", 0, 64)["error"]["code"],
        "path_denied"
    );
    assert_eq!(thin_commit(&mut c, 3, &p)["error"]["code"], "path_denied");
    assert_eq!(c.document(), before);
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"Changed but regular authorized bytes").unwrap();
    let page = thin_page(&mut c, 3, &p, "changes", 0, 64);
    assert_eq!(page["ok"], true);
    assert_eq!(page["data"]["source_identity_verified"], false);
    assert_eq!(thin_commit(&mut c, 3, &p)["ok"], true);
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":4,"path":"invalid-source.wav"})
        )["ok"],
        false
    );
    assert!(!f.root.join("invalid-source.wav").exists());
    fs::write(path, bytes).unwrap();
}
#[test]
fn thin_zero_percent_still_requires_model_evaluable_assignment_and_failures_keep_previous_plan() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let empty = thin_preview(&mut c, 0, thin_config(0, 42));
    assert_eq!(empty["ok"], false);
    thin_setup(&mut c, &f);
    let good = thin_preview(&mut c, 3, thin_config(0, 42));
    assert_eq!(good["ok"], true);
    let mut bad = thin_config(0, 42);
    bad["scope"]["pattern"] = json!(999);
    assert_eq!(thin_preview(&mut c, 3, bad)["ok"], false);
    assert_eq!(thin_commit(&mut c, 3, &good)["ok"], true);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![json!({"op":"assign_region","instrument_id":2,"region_id":null})]
        )["ok"],
        true
    );
    let before = c.document();
    let e = thin_preview(&mut c, 4, thin_config(0, 42));
    assert_eq!(e["ok"], false);
    assert!(
        e["error"]["message"].as_str().unwrap().contains("base"),
        "{e}"
    );
    assert_eq!(c.document(), before);
}

#[test]
fn thin_rejects_retained_selection_and_owned_control_retargeting_without_losing_noop_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let target = json!({"note":{"Note":60},"velocity":0.5,"pan":0.0,"effects":{"release":0.5}});
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":2,"column":0,"cell":target})
            ]
        )["ok"],
        true
    );
    let good = thin_preview(&mut c, 4, thin_config(0, 42));
    assert_eq!(good["ok"], true, "{good}");
    let before = c.document();
    let rejected = thin_preview(&mut c, 4, thin_config(100, 42));
    assert_eq!(rejected["ok"], false);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("retained-ON"),
        "{rejected}"
    );
    assert_eq!(c.document(), before);
    assert_eq!(thin_commit(&mut c, 4, &good)["data"]["changed"], false);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":2,"column":0,"cell":{"note":"Empty","velocity":0.7}})
            ]
        )["ok"],
        true
    );
    let good = thin_preview(&mut c, 5, thin_config(0, 42));
    assert_eq!(good["ok"], true, "{good}");
    let before = c.document();
    let rejected = thin_preview(&mut c, 5, thin_config(100, 42));
    assert_eq!(rejected["ok"], false);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("control"),
        "{rejected}"
    );
    assert_eq!(c.document(), before);
    assert_eq!(thin_commit(&mut c, 5, &good)["ok"], true);
}
#[test]
fn thin_role_protection_aggregates_distinct_round_robin_sources_across_shared_occurrences() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let zones = json!([{"min_velocity":0,"max_velocity":127,"alternatives":[{"id":11,"region":21},{"id":12,"region":22}]}]);
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                pad_sources(Value::Null, zones),
                json!({"op":"set_region_role","region_id":21,"label":feature_label("closed_hat","normal","")}),
                json!({"op":"set_region_role","region_id":22,"label":feature_label("kick","normal","")}),
                zone_cell(1, 0, 1, 0.5),
                json!({"op":"set_order","pattern_ids":[1,1]})
            ]
        )["ok"],
        true
    );
    let p = thin_preview(&mut c, 3, thin_config(100, 42));
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["data"]["summary"]["gate_calls"], 0);
    assert_eq!(p["data"]["summary"]["removed_cells"], 0);
    let uses = thin_page(&mut c, 3, &p, "uses", 0, 64);
    let records = uses["data"]["records"].as_array().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["source"]["region"], 21);
    assert_eq!(records[1]["source"]["region"], 22);
    assert_eq!(records[1]["retention"], "protected_role");
    assert_eq!(
        thin_page(&mut c, 3, &p, "decisions", 0, 64)["data"]["records"][0]["reason"],
        "protected_role"
    );
    assert_eq!(thin_commit(&mut c, 3, &p)["data"]["changed"], false);
}
#[test]
fn thin_record_and_context_budgets_reject_without_truncation_or_replacing_valid_review() {
    use aqua_pattern_core::{project, song::*};
    use std::collections::BTreeSet;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let base: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let mut song = base.clone();
    song.tracks.get_mut(&TrackId(1)).unwrap().note_columns = 16;
    let pattern = song.patterns.get_mut(&PatternId(1)).unwrap();
    pattern.rows = 300;
    let track = pattern.tracks.get_mut(&TrackId(1)).unwrap();
    track.clear();
    for row in 0..256 {
        for column in 0..16 {
            track.entry(row).or_default().insert(
                column,
                Cell {
                    velocity: Some(0.2),
                    ..Cell::default()
                },
            );
        }
    }
    track.entry(256).or_default().insert(
        0,
        Cell {
            velocity: Some(0.2),
            ..Cell::default()
        },
    );
    track.entry(299).or_default().insert(
        0,
        Cell {
            note: Note::Note(60),
            instrument: Some(InstrumentId(1)),
            velocity: Some(0.5),
            ..Cell::default()
        },
    );
    project::save(
        &song,
        &f.root.join("record-limit.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":3,"path":"record-limit.leviathan.json","discard_unsaved":true}),
    );
    let mut config = thin_config(0, 42);
    config["scope"]["row_end"] = json!(256);
    config["scope"]["columns"] = json!((0..16).collect::<Vec<_>>());
    let p = thin_preview(&mut c, 4, config.clone());
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["data"]["counts"]["decisions"], 4096);
    let mut total = 0;
    for start in (0..4096).step_by(64) {
        let page = thin_page(&mut c, 4, &p, "decisions", start, 64);
        assert_eq!(page["ok"], true);
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        total += page["data"]["records"].as_array().unwrap().len();
    }
    assert_eq!(total, 4096);
    config["scope"]["row_end"] = json!(257);
    let e = thin_preview(&mut c, 4, config);
    assert_eq!(e["ok"], false);
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("4096 nondefault"));
    assert_eq!(thin_commit(&mut c, 4, &p)["data"]["changed"], false);
    let mut song = base;
    let source = song.patterns.get_mut(&PatternId(1)).unwrap();
    source.rows = 1;
    for rows in source.tracks.values_mut() {
        rows.retain(|row, _| *row == 0);
    }
    // This context budget is specifically a five-track scenario. Keep the startup
    // fixture's extra default tracks out of the hand-built order.
    song.tracks.retain(|id, _| id.0 <= 4);
    song.track_order.retain(|id| id.0 <= 4);
    for pattern in song.patterns.values_mut() {
        pattern.tracks.retain(|id, _| id.0 <= 4);
        pattern.track_aliases.clear();
    }
    for occurrence in &mut song.order {
        occurrence.muted_tracks.retain(|id| id.0 <= 4);
    }
    song.tracks.insert(
        TrackId(5),
        Track {
            name: "Fifth alias".into(),
            note_columns: song.tracks[&TrackId(1)].note_columns,
        },
    );
    song.track_order.push(TrackId(5));
    for id in 2..=5 {
        song.set_track_alias(
            PatternId(1),
            TrackId(id),
            Some(TrackAlias {
                pattern: PatternId(1),
                track: TrackId(1),
            }),
        )
        .unwrap();
    }
    let mut unused = song.patterns[&PatternId(1)].clone();
    unused.track_aliases.clear();
    song.patterns.insert(PatternId(2), unused);
    song.order = vec![
        Occurrence {
            pattern: PatternId(1),
            muted_tracks: (1..=5).map(TrackId).collect::<BTreeSet<_>>(),
            section: None,
        };
        1024
    ];
    song.validate().unwrap();
    project::save(
        &song,
        &f.root.join("context-limit.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":4,"path":"context-limit.leviathan.json","discard_unsaved":true}),
    );
    let mut unused_config = thin_config(0, 42);
    unused_config["scope"]["pattern"] = json!(2);
    unused_config["scope"]["row_end"] = json!(1);
    let p = thin_preview(&mut c, 5, unused_config);
    assert_eq!(p["ok"], true, "{p}");
    let mut shared_config = thin_config(0, 42);
    shared_config["scope"]["row_end"] = json!(1);
    let e = thin_preview(&mut c, 5, shared_config);
    assert_eq!(e["ok"], false);
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("4096 ordered"),
        "{e}"
    );
    assert_eq!(thin_commit(&mut c, 5, &p)["data"]["changed"], false);
}
#[test]
fn thin_large_title_changed_and_noop_commits_preflight_bounded_history_only_responses() {
    use aqua_pattern_core::{project, song::Song};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    thin_setup(&mut c, &f);
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    song.title = "x".repeat(6 * 1024 * 1024);
    project::save(
        &song,
        &f.root.join("large-thin.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":3,"path":"large-thin.leviathan.json","discard_unsaved":true}),
    );
    let p = thin_preview(&mut c, 4, thin_config(100, 42));
    assert_eq!(p["ok"], true, "{p}");
    let result = thin_commit(&mut c, 4, &p);
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["changed"], true);
    assert!(result["data"]["session"].get("title").is_none());
    assert!(serde_json::to_vec(&result["data"]).unwrap().len() <= 256 * 1024);
    assert_eq!(
        edit(
            &mut c,
            5,
            vec![json!({"op":"set_song","title":"temporary"})]
        )["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    let before = c.ok("leviathan_project_read", json!({"section":"history"}));
    let p = thin_preview(&mut c, 7, thin_config(0, 42));
    assert_eq!(p["ok"], true, "{p}");
    let result = thin_commit(&mut c, 7, &p);
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["changed"], false);
    assert!(serde_json::to_vec(&result["data"]).unwrap().len() <= 256 * 1024);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        before
    );
}

fn audition_region_target() -> Value {
    json!({"kind":"region_draft","sample_id":7,"start_frame":10,"end_frame":48000,
        "root_note":72,"amp_envelope":{"attack_ms":2,"decay_ms":3,"sustain_percent":50,"release_ms":7},
        "forward_loop":{"start_frame":20,"end_frame":100}})
}
fn audition_prepare(c: &mut Client, revision: u64, target: Value) -> Value {
    c.call(
        "leviathan_audition_prepare",
        json!({"expected_revision":revision,"target":target}),
    )
}

#[test]
fn audition_prepare_region_is_exact_shared_metadata_not_audio_or_a_document_transaction() {
    use aqua_pattern_core::{audition, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let before = c.document();
    let history = c.ok("leviathan_project_read", json!({"section":"history"}));
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let draft = SampleRegion {
        name: "Audition region draft".into(),
        sample: SampleId(7),
        start_frame: 10,
        end_frame: 48000,
        root_note: 72,
        amp_envelope: Some(AmpEnvelope {
            attack_ms: 2,
            decay_ms: 3,
            sustain_percent: 50,
            release_ms: 7,
        }),
        forward_loop: Some(ForwardLoop {
            start_frame: 20,
            end_frame: 100,
        }),
    };
    let shared = audition::region_preview(&song.samples[&SampleId(7)], &draft).unwrap();
    let prepared = audition_prepare(&mut c, 2, audition_region_target());
    assert_eq!(prepared["ok"], true, "{prepared}");
    let data = &prepared["data"];
    assert_eq!(data["summary"], json!(shared.summary()));
    assert_eq!(data["preview_id"], shared.id());
    assert_eq!(data["summary"]["sample_rate"], 48000);
    assert_eq!(data["summary"]["musical_frames"], 96000);
    assert_eq!(data["summary"]["tail_frames"], 24000);
    assert_eq!(data["summary"]["total_frames"], 120000);
    assert_eq!(data["summary"]["sounding_note"], 72);
    assert_eq!(data["summary"]["region"]["start_frame"], 10);
    assert_eq!(data["summary"]["region"]["end_frame"], 48000);
    assert_eq!(data["scope"], "headless_metadata_only");
    assert_eq!(data["source_paths_authorized_now"], true);
    for field in [
        "source_identity_verified_now",
        "audio_rendered",
        "audio_device_opened",
        "document_changed",
    ] {
        assert_eq!(data[field], false, "{field}");
    }
    assert_eq!(data["summary"]["source_identity_verified"], false);
    assert_eq!(data["review_ref"].as_str().unwrap().len(), 64);
    assert!(serde_json::to_vec(data).unwrap().len() < 256 * 1024);
    assert_eq!(
        audition_prepare(&mut c, 2, audition_region_target()),
        prepared
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"})),
        history
    );
    assert_eq!(
        fs::read_dir(&f.root).unwrap().count(),
        1,
        "no WAV, sidecar or preview file created"
    );

    // No bytes are verified: current regular source bytes may be invalid, while
    // preparation honestly reports only the stored declared identity.
    let original = fs::read(f.root.join("kit.wav")).unwrap();
    fs::write(f.root.join("kit.wav"), b"not audio anymore").unwrap();
    assert_eq!(
        audition_prepare(&mut c, 2, audition_region_target()),
        prepared
    );
    assert_eq!(
        fs::read(f.root.join("kit.wav")).unwrap(),
        b"not audio anymore"
    );
    fs::write(f.root.join("kit.wav"), original).unwrap();
    assert_eq!(c.document(), before);
}

#[test]
fn audition_prepare_pad_resolves_committed_gain_tuning_velocity_zone_and_fresh_first_rr() {
    use aqua_pattern_core::{audition, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    zone_setup(&mut c, &f);
    let mut metadata = kit_pad(301, 0, 36);
    metadata["region_id"] = Value::Null;
    metadata["gain"] = json!(0.25);
    metadata["pan"] = json!(-0.5);
    metadata["root_note"] = json!(48);
    metadata["tune_semitones"] = json!(12);
    metadata["mode"] = json!("Gated");
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![metadata, pad_sources(Value::Null, velocity_zones())]
        )["ok"],
        true
    );
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let shared = audition::pad_preview(&song, InstrumentId(1), PadId(301)).unwrap();
    let target = json!({"kind":"pad","instrument_id":1,"pad_id":301});
    let prepared = audition_prepare(&mut c, 3, target.clone());
    assert_eq!(prepared["ok"], true, "{prepared}");
    let summary = &prepared["data"]["summary"];
    assert_eq!(*summary, json!(shared.summary()));
    assert_eq!(summary["input_velocity"], 1.0);
    assert_eq!(summary["velocity_bin"], 127);
    assert_eq!(summary["on_gain"], 0.25);
    assert_eq!(summary["pan"], -0.5);
    assert_eq!(summary["sounding_note"], 60);
    assert_eq!(summary["pad"]["note"], 36);
    assert_eq!(summary["pad"]["tune_semitones"], 12);
    assert_eq!(summary["pad"]["mode"], "Gated");
    assert_eq!(summary["pad"]["gain"], 0.25);
    assert_eq!(summary["region"]["region"], 23);
    assert_eq!(summary["region"]["root_note"], 48);
    assert_eq!(summary["layer_selection"]["layer"], 21);
    assert_eq!(summary["layer_selection"]["min_velocity"], 64);
    assert_eq!(summary["layer_selection"]["max_velocity"], 127);
    assert_eq!(summary["layer_selection"]["round_robin_index"], 0);
    assert_eq!(summary["layer_selection"]["round_robin_ordinal"], 0);
    for _ in 0..3 {
        assert_eq!(
            audition_prepare(&mut c, 3, target.clone()),
            prepared,
            "every prepare is fresh, not a persistent RR counter"
        );
    }
    assert_eq!(c.document(), before);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

#[test]
fn audition_prepare_schema_invalid_drafts_and_revision_ref_bindings_are_explicit() {
    let catalog = mcp::tool_catalog();
    let tool = catalog
        .iter()
        .find(|tool| tool["name"] == "leviathan_audition_prepare")
        .unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["idempotentHint"], true);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
    assert!(tool["title"].as_str().unwrap().contains("no audio"));
    let variants = tool["inputSchema"]["properties"]["target"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0]["additionalProperties"], false);
    assert_eq!(variants[1]["additionalProperties"], false);
    assert!(variants[0]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("amp_envelope")));
    assert!(variants[0]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("forward_loop")));
    assert!(!catalog
        .iter()
        .any(|tool| tool["name"] == "leviathan_audition_render"));
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    let before = c.document();
    let prepared = audition_prepare(&mut c, 2, audition_region_target());
    assert_eq!(prepared["ok"], true);
    for field in [
        "kind",
        "sample_id",
        "start_frame",
        "end_frame",
        "root_note",
        "amp_envelope",
        "forward_loop",
    ] {
        let mut target = audition_region_target();
        target.as_object_mut().unwrap().remove(field);
        assert_eq!(
            audition_prepare(&mut c, 2, target)["error"]["code"],
            "invalid_arguments",
            "{field}"
        );
    }
    for (field, value) in [
        ("kind", json!("live_pad")),
        ("sample_id", json!(0)),
        ("sample_id", json!(999)),
        ("start_frame", json!(48000)),
        ("end_frame", json!(48001)),
        ("end_frame", json!(10)),
        ("root_note", json!(128)),
        ("root_note", json!(60.5)),
        (
            "amp_envelope",
            json!({"attack_ms":10001,"decay_ms":0,"sustain_percent":100,"release_ms":50}),
        ),
        (
            "amp_envelope",
            json!({"attack_ms":0,"decay_ms":0,"sustain_percent":100,"release_ms":50,"curve":"custom"}),
        ),
        (
            "forward_loop",
            json!({"start_frame":20,"end_frame":100,"crossfade":10}),
        ),
        ("forward_loop", json!({"start_frame":10,"end_frame":11})),
        ("forward_loop", json!({"start_frame":0,"end_frame":100})),
        ("velocity", json!(0.5)),
        ("path", json!("../outside.wav")),
        ("candidate", json!({})),
    ] {
        let mut target = audition_region_target();
        target[field] = value;
        assert_eq!(
            audition_prepare(&mut c, 2, target.clone())["ok"],
            false,
            "{target}"
        );
        assert_eq!(c.document(), before);
    }
    for target in [
        json!({"kind":"pad","instrument_id":1}),
        json!({"kind":"pad","instrument_id":0,"pad_id":301}),
        json!({"kind":"pad","instrument_id":1,"pad_id":301}),
        json!({"kind":"pad","instrument_id":999,"pad_id":301}),
        json!({"kind":"pad","instrument_id":1,"pad_id":301,"velocity":0.5}),
    ] {
        assert_eq!(audition_prepare(&mut c, 2, target)["ok"], false);
    }
    assert_eq!(
        audition_prepare(&mut c, 1, audition_region_target())["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(&mut c, 2, vec![title("Unrelated base change")])["ok"],
        true
    );
    let changed = audition_prepare(&mut c, 3, audition_region_target());
    assert_eq!(
        changed["data"]["preview_id"],
        prepared["data"]["preview_id"]
    );
    assert_ne!(
        changed["data"]["review_ref"],
        prepared["data"]["review_ref"]
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    let aba = audition_prepare(&mut c, 4, audition_region_target());
    assert_eq!(aba["data"]["preview_id"], prepared["data"]["preview_id"]);
    assert_ne!(aba["data"]["review_ref"], prepared["data"]["review_ref"]);
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"}))["data"]["can_redo"],
        true
    );
    let mut target = audition_region_target();
    target["root_note"] = json!(73);
    assert_ne!(
        audition_prepare(&mut c, 4, target)["data"]["review_ref"],
        aba["data"]["review_ref"]
    );
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn audition_prepare_reauthorizes_selected_snapshot_paths_but_not_unrelated_sources() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    sampler_setup(&mut c, &f);
    fs::copy(f.root.join("kit.wav"), f.root.join("unrelated.wav")).unwrap();
    import_source(&mut c, 2, 8, "unrelated.wav");
    fs::remove_file(f.root.join("unrelated.wav")).unwrap();
    let before = c.document();
    let prepared = audition_prepare(&mut c, 3, audition_region_target());
    assert_eq!(
        prepared["ok"], true,
        "unselected document assets are not copied into the isolated snapshot: {prepared}"
    );
    let bytes = fs::read(f.root.join("kit.wav")).unwrap();
    let outside = f.base.join("outside.wav");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(f.root.join("kit.wav")).unwrap();
    symlink(&outside, f.root.join("kit.wav")).unwrap();
    assert_eq!(
        audition_prepare(&mut c, 3, audition_region_target())["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(&outside).unwrap(), bytes);
    fs::remove_file(f.root.join("kit.wav")).unwrap();
    fs::create_dir(f.root.join("kit.wav")).unwrap();
    assert_eq!(
        audition_prepare(&mut c, 3, audition_region_target())["error"]["code"],
        "path_denied"
    );
    fs::remove_dir(f.root.join("kit.wav")).unwrap();
    assert_eq!(
        audition_prepare(&mut c, 3, audition_region_target())["error"]["code"],
        "io_error"
    );
    fs::write(f.root.join("kit.wav"), bytes).unwrap();
    assert_eq!(
        audition_prepare(&mut c, 3, audition_region_target()),
        prepared
    );
    assert_eq!(c.document(), before);
}

#[cfg(unix)]
#[test]
fn audition_prepare_preflights_unused_retained_pad_alternatives_before_reporting_metadata() {
    use aqua_pattern_core::{audition, song::*};
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    fs::copy(
        f.root.join("kit.wav"),
        f.root.join("unused-alternative.wav"),
    )
    .unwrap();
    import_source(&mut c, 2, 8, "unused-alternative.wav");
    let mut alternative = region_edit(25, 0, 48000);
    alternative["sample_id"] = json!(8);
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                alternative,
                pad_sources(
                    Value::Null,
                    json!([
                        {"min_velocity":0,"max_velocity":127,"alternatives":[{"id":1,"region":21},{"id":2,"region":25}]}
                    ])
                )
            ]
        )["ok"],
        true
    );
    let before = c.document();
    let target = json!({"kind":"pad","instrument_id":1,"pad_id":301});
    let prepared = audition_prepare(&mut c, 4, target.clone());
    assert_eq!(prepared["ok"], true, "{prepared}");
    assert_eq!(prepared["data"]["summary"]["region"]["region"], 21);
    assert_eq!(prepared["data"]["summary"]["retained_sources"], 2);
    let bytes = fs::read(f.root.join("unused-alternative.wav")).unwrap();
    fs::remove_file(f.root.join("unused-alternative.wav")).unwrap();
    // The shared builder is pure, and its chosen source is still present. MCP
    // intentionally applies its stricter path policy to all retained mappings.
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    assert!(audition::pad_preview(&song, InstrumentId(1), PadId(301)).is_ok());
    assert_eq!(
        audition_prepare(&mut c, 4, target.clone())["error"]["code"],
        "io_error"
    );
    let outside = f.base.join("outside-alternative.wav");
    fs::write(&outside, &bytes).unwrap();
    symlink(&outside, f.root.join("unused-alternative.wav")).unwrap();
    assert_eq!(
        audition_prepare(&mut c, 4, target.clone())["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(&outside).unwrap(), bytes);
    fs::remove_file(f.root.join("unused-alternative.wav")).unwrap();
    fs::write(f.root.join("unused-alternative.wav"), bytes).unwrap();
    assert_eq!(audition_prepare(&mut c, 4, target), prepared);
    assert_eq!(c.document(), before);
}

#[test]
fn audition_prepare_large_stored_labels_reject_without_unbounded_response_or_mutation() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    kit_setup(&mut c, &f);
    let mut region = region_edit(21, 0, 48000);
    // The general Song format permits longer historical region names; the
    // bounded audition snapshot deliberately does not copy/echo them.
    region["name"] = json!("x".repeat(300_000));
    assert_eq!(edit(&mut c, 2, vec![region])["ok"], true);
    let before = c.document();
    let result = audition_prepare(
        &mut c,
        3,
        json!({"kind":"pad","instrument_id":1,"pad_id":301}),
    );
    assert_eq!(result["error"]["code"], "validation_error");
    assert!(serde_json::to_vec(&result).unwrap().len() < 2048);
    assert_eq!(c.document(), before);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

fn marker_bank_setup(c: &mut Client, f: &Fixture) {
    kit_setup(c, f);
    assert_eq!(
        edit(
            c,
            2,
            vec![
                marker_edit(31, Value::Null, vec![0, 12000, 24000, 36000, 48000]),
                json!({"op":"set_song","bpm":153.25,"lines_per_beat":7}),
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":36},"instrument":1,"velocity":0.37,"pan":-0.0}}),
            ]
        )["ok"],
        true
    );
}
fn marker_bank_config(indices: Vec<u16>) -> Value {
    json!({"version":1,"marker_set":31,"interval_indices":indices,"name":"Reviewed marker bank"})
}
fn marker_bank_preview(c: &mut Client, revision: u64, config: Value) -> Value {
    c.call(
        "leviathan_marker_bank_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn marker_bank_commit(c: &mut Client, revision: u64, preview: &Value) -> Value {
    c.call(
        "leviathan_marker_bank_commit",
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}

#[test]
fn marker_bank_shared_preview_commit_exact_bounds_defaults_one_undo_and_project_roundtrip() {
    use aqua_pattern_core::{editor::Editor, marker_bank, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    marker_bank_setup(&mut c, &f);
    let before = c.document();
    let source_bytes = fs::read(f.root.join("kit.wav")).unwrap();
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let config = marker_bank_config(vec![0, 2]);
    let typed = serde_json::from_value(config.clone()).unwrap();
    let shared = marker_bank::preview(&base, &typed).unwrap();
    let preview = marker_bank_preview(&mut c, 3, config.clone());
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["data"]["summary"], json!(shared.summary()));
    assert_eq!(preview["data"]["preview_id"], shared.id());
    assert_eq!(preview["data"]["effective_config"], config);
    assert_eq!(preview["data"]["source_identity_verified_now"], false);
    assert_eq!(preview["data"]["source_path_authorized_now"], false);
    assert_eq!(preview["data"]["audio_decoded"], false);
    assert_eq!(preview["data"]["document_changed"], false);
    assert_eq!(preview["data"]["summary"]["kit"], 42);
    let next_instrument_id = base.instruments.keys().map(|id| id.0).max().unwrap() + 1;
    assert_eq!(preview["data"]["summary"]["instrument"], next_instrument_id);
    let pads = preview["data"]["summary"]["pads"].as_array().unwrap();
    assert_eq!(pads.len(), 2);
    for (index, (start, end)) in [(0, 12000), (24000, 36000)].into_iter().enumerate() {
        assert_eq!(pads[index]["region"], 22 + index);
        assert_eq!(pads[index]["pad"], 1 + index);
        assert_eq!(pads[index]["slot"], index);
        assert_eq!(pads[index]["selector"], 36 + index);
        assert_eq!(pads[index]["start_frame"], start);
        assert_eq!(pads[index]["end_frame"], end);
        assert_eq!(pads[index]["root_note"], 60);
        assert_eq!(pads[index]["tune_semitones"], 0);
        assert_eq!(pads[index]["gain"], 1.0);
        assert_eq!(pads[index]["pan"], 0.0);
        assert_eq!(pads[index]["mode"], "OneShot");
        assert_eq!(pads[index]["new_note"], "Overlap");
        assert!(pads[index]["choke_group"].is_null());
    }
    assert_eq!(c.document(), before);
    let mut editor = Editor {
        song: base.clone(),
        ..Editor::default()
    };
    editor.edit_mode = true;
    editor
        .commit_marker_bank_preview(editor.revision(), &shared)
        .unwrap();
    let committed = marker_bank_commit(&mut c, 3, &preview);
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["revision"], 4);
    assert_eq!(committed["data"]["source_identity_verified_now"], true);
    assert_eq!(
        committed["data"]["all_source_identities_verified_now"],
        false
    );
    assert_eq!(committed["data"]["audio_decoded"], false);
    assert_eq!(committed["data"]["source_files_changed"], false);
    assert_eq!(committed["data"]["session_scope"], "history_only");
    assert_eq!(committed["data"]["summary"], json!(shared.summary()));
    let after = c.document()["data"]["song"].clone();
    assert_eq!(after, json!(editor.song));
    assert_eq!(after, json!(shared.candidate()));
    for field in [
        "bpm",
        "lines_per_beat",
        "patterns",
        "order",
        "tracks",
        "track_order",
        "samples",
        "marker_sets",
        "phrases",
        "grooves",
        "region_catalog",
    ] {
        assert_eq!(after[field], before["data"]["song"][field], "{field}");
    }
    assert!(after["regions"]["22"].get("amp_envelope").is_none());
    assert!(after["regions"]["22"].get("forward_loop").is_none());
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), source_bytes);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":5}));
    assert_eq!(c.document()["data"]["song"], after);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":6,"path":"marker-bank.leviathan.json"}),
    );
    c.ok("leviathan_project_new", json!({"expected_revision":7}));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":8,"path":"marker-bank.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], after);
    assert_eq!(fs::read(f.root.join("kit.wav")).unwrap(), source_bytes);
}

#[test]
fn marker_bank_strict_schema_failure_preserves_review_and_explicit_sixteen_interval_limit() {
    let catalog = mcp::tool_catalog();
    let tool = catalog
        .iter()
        .find(|tool| tool["name"] == "leviathan_marker_bank_preview")
        .unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
    let schema = &tool["inputSchema"]["properties"]["config"];
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["version"]["const"], 1);
    assert_eq!(schema["properties"]["interval_indices"]["minItems"], 1);
    assert_eq!(schema["properties"]["interval_indices"]["maxItems"], 16);
    assert_eq!(
        schema["properties"]["interval_indices"]["uniqueItems"],
        true
    );
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    marker_bank_setup(&mut c, &f);
    let before = c.document();
    let good = marker_bank_preview(&mut c, 3, marker_bank_config(vec![0, 2]));
    assert_eq!(good["ok"], true);
    for field in ["version", "marker_set", "interval_indices", "name"] {
        let mut config = marker_bank_config(vec![0]);
        config.as_object_mut().unwrap().remove(field);
        assert_eq!(
            marker_bank_preview(&mut c, 3, config)["error"]["code"],
            "invalid_arguments"
        );
    }
    for (field, value) in [
        ("version", json!(2)),
        ("marker_set", json!(0)),
        ("marker_set", json!(999)),
        ("interval_indices", json!([])),
        ("interval_indices", json!([2, 0])),
        ("interval_indices", json!([0, 0])),
        ("interval_indices", json!([4])),
        ("interval_indices", json!([0.5])),
        ("interval_indices", json!([-1])),
        ("interval_indices", json!((0..17).collect::<Vec<_>>())),
        ("name", json!("  ")),
        ("name", json!("é".repeat(129))),
        ("target_bpm", json!(174)),
        ("source_bpm", json!(70)),
        ("root_note", json!(48)),
        ("path", json!("../outside.wav")),
        ("candidate", json!({})),
    ] {
        let mut config = marker_bank_config(vec![0, 2]);
        config[field] = value;
        assert_eq!(
            marker_bank_preview(&mut c, 3, config.clone())["ok"],
            false,
            "{config}"
        );
        assert_eq!(c.document(), before);
    }
    let injected = c.call(
        "leviathan_marker_bank_commit",
        json!({"expected_revision":3,"preview_token":good["data"]["preview_token"],"candidate":{}}),
    );
    assert_eq!(injected["error"]["code"], "invalid_arguments");
    assert_eq!(
        marker_bank_commit(&mut c, 3, &good)["ok"],
        true,
        "failed previews kept the original private candidate"
    );
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![marker_edit(
                31,
                Value::Null,
                (0..=16).map(|i| i * 3000).collect()
            )]
        )["ok"],
        true
    );
    let mut config = marker_bank_config((0..16).collect());
    config["name"] = json!("é".repeat(128));
    let full = marker_bank_preview(&mut c, 5, config);
    assert_eq!(full["ok"], true, "{full}");
    assert_eq!(
        full["data"]["summary"]["pads"].as_array().unwrap().len(),
        16
    );
    assert_eq!(full["data"]["summary"]["pads"][15]["slot"], 15);
    assert_eq!(full["data"]["summary"]["pads"][15]["selector"], 51);
    assert_eq!(full["data"]["summary"]["pads"][15]["end_frame"], 48000);
    assert_eq!(marker_bank_commit(&mut c, 5, &full)["ok"], true);
}

#[test]
fn marker_bank_preview_is_pure_commit_rehashes_and_failure_preserves_recoverable_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    marker_bank_setup(&mut c, &f);
    let before = c.document();
    let source = f.root.join("kit.wav");
    let bytes = fs::read(&source).unwrap();
    fs::remove_file(&source).unwrap();
    let missing = marker_bank_preview(&mut c, 3, marker_bank_config(vec![0, 2]));
    assert_eq!(
        missing["ok"], true,
        "preview never touches source files: {missing}"
    );
    assert_eq!(
        marker_bank_commit(&mut c, 3, &missing)["error"]["code"],
        "io_error"
    );
    fs::write(&source, b"changed bytes, not audio").unwrap();
    let changed = marker_bank_commit(&mut c, 3, &missing);
    assert_eq!(changed["error"]["code"], "validation_error");
    assert!(changed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("SHA-256"));
    let large = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&source)
        .unwrap();
    large
        .set_len(aqua_pattern_core::sample_asset::MAX_ENCODED_BYTES as u64 + 1)
        .unwrap();
    drop(large);
    let budget = marker_bank_commit(&mut c, 3, &missing);
    assert_eq!(budget["error"]["code"], "validation_error");
    assert!(budget["error"]["message"]
        .as_str()
        .unwrap()
        .contains("64 MiB"));
    assert_eq!(c.document(), before);
    fs::write(&source, &bytes).unwrap();
    assert_eq!(
        marker_bank_commit(&mut c, 3, &missing)["ok"],
        true,
        "same token recovers after external source restoration"
    );
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn marker_bank_commit_rejects_lexical_symlinks_but_does_not_verify_unselected_sources() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    marker_bank_setup(&mut c, &f);
    fs::copy(f.root.join("kit.wav"), f.root.join("unrelated.wav")).unwrap();
    import_source(&mut c, 3, 8, "unrelated.wav");
    fs::remove_file(f.root.join("unrelated.wav")).unwrap();
    let before = c.document();
    let source = f.root.join("kit.wav");
    let outside = f.base.join("outside.wav");
    let bytes = fs::read(&source).unwrap();
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(&source).unwrap();
    symlink(&outside, &source).unwrap();
    let preview = marker_bank_preview(&mut c, 4, marker_bank_config(vec![1, 3]));
    assert_eq!(preview["ok"], true, "pure preview does not authorize paths");
    assert_eq!(
        marker_bank_commit(&mut c, 4, &preview)["error"]["code"],
        "path_denied"
    );
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(&outside).unwrap(), bytes);
    fs::remove_file(&source).unwrap();
    fs::create_dir(&source).unwrap();
    assert_eq!(
        marker_bank_commit(&mut c, 4, &preview)["error"]["code"],
        "path_denied"
    );
    fs::remove_dir(&source).unwrap();
    fs::write(&source, &bytes).unwrap();
    let committed = marker_bank_commit(&mut c, 4, &preview);
    assert_eq!(
        committed["ok"], true,
        "unselected missing source is not a selected-source hash failure: {committed}"
    );
    assert_eq!(
        committed["data"]["all_source_identities_verified_now"],
        false
    );
    assert_eq!(
        c.document()["data"]["song"]["samples"],
        before["data"]["song"]["samples"]
    );
    // Normal whole-project save policy is not bypassed or redefined by this tool.
    assert_eq!(
        c.call(
            "leviathan_project_save",
            json!({"expected_revision":5,"path":"not-save-ready.leviathan.json"})
        )["ok"],
        false
    );
    assert!(!f.root.join("not-save-ready.leviathan.json").exists());
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

#[test]
fn marker_bank_supersession_noop_redo_aba_and_repeated_commit_are_guarded() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    marker_bank_setup(&mut c, &f);
    let base = c.document()["data"]["song"].clone();
    let old = marker_bank_preview(&mut c, 3, marker_bank_config(vec![0]));
    assert_eq!(edit(&mut c, 3, vec![title("Temporary")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], base);
    assert_eq!(
        marker_bank_commit(&mut c, 3, &old)["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(
        marker_bank_commit(&mut c, 5, &old)["error"]["code"],
        "marker_bank_preview_unavailable"
    );
    let a = marker_bank_preview(&mut c, 5, marker_bank_config(vec![0, 2]));
    let b = marker_bank_preview(&mut c, 5, marker_bank_config(vec![0, 2]));
    assert_eq!(a["data"]["preview_id"], b["data"]["preview_id"]);
    assert_ne!(a["data"]["preview_token"], b["data"]["preview_token"]);
    let noop = edit(&mut c, 5, vec![title(base["title"].as_str().unwrap())]);
    assert_eq!(noop["revision"], 5);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["data"]["session"]["can_redo"], true);
    assert_eq!(
        marker_bank_commit(&mut c, 5, &a)["error"]["code"],
        "invalid_marker_bank_token"
    );
    assert_eq!(
        marker_bank_preview(&mut c, 5, marker_bank_config(vec![]))["ok"],
        false
    );
    let committed = marker_bank_commit(&mut c, 5, &b);
    assert_eq!(committed["ok"], true);
    assert_eq!(committed["revision"], 6);
    assert_eq!(committed["data"]["session"]["can_redo"], false);
    let after = c.document();
    assert_eq!(
        marker_bank_commit(&mut c, 6, &b)["error"]["code"],
        "marker_bank_preview_unavailable"
    );
    assert_eq!(c.document(), after);
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    assert_eq!(c.document()["data"]["song"], base);
}

#[test]
fn marker_bank_id_exhaustion_is_atomic_and_last_region_id_is_not_rounded_or_reused() {
    for namespace in ["region", "kit", "instrument"] {
        let f = Fixture::new();
        let mut c = Client::ready(&f);
        marker_bank_setup(&mut c, &f);
        let operation = match namespace {
            "region" => region_edit(u64::MAX, 0, 12000),
            "kit" => kit_metadata(u64::MAX, "Last kit"),
            "instrument" => {
                json!({"op":"upsert_instrument","id":u64::MAX,"name":"Last instrument","sample":null})
            }
            _ => unreachable!(),
        };
        assert_eq!(edit(&mut c, 3, vec![operation])["ok"], true);
        let before = c.document();
        let failed = marker_bank_preview(&mut c, 4, marker_bank_config(vec![0]));
        assert_eq!(
            failed["error"]["code"], "validation_error",
            "{namespace}: {failed}"
        );
        assert!(failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ID space exhausted"));
        assert_eq!(c.document(), before);
        c.ok("leviathan_history_undo", json!({"expected_revision":4}));
        assert_eq!(
            marker_bank_preview(&mut c, 5, marker_bank_config(vec![0]))["ok"],
            true
        );
    }
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    marker_bank_setup(&mut c, &f);
    assert_eq!(
        edit(&mut c, 3, vec![region_edit(u64::MAX - 1, 0, 12000)])["ok"],
        true
    );
    let last = marker_bank_preview(&mut c, 4, marker_bank_config(vec![0]));
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["data"]["summary"]["pads"][0]["region"], u64::MAX);
    assert_eq!(
        marker_bank_preview(&mut c, 4, marker_bank_config(vec![0, 2]))["ok"],
        false
    );
    assert_eq!(marker_bank_commit(&mut c, 4, &last)["ok"], true);
    assert!(c.document()["data"]["song"]["regions"]
        .get(u64::MAX.to_string())
        .is_some());
}

fn cell_scope(
    pattern: u64,
    row_start: u32,
    row_end: u32,
    column_start: u16,
    column_end: u16,
) -> Value {
    json!({"pattern":pattern,"track":1,"row_start":row_start,"row_end":row_end,"column_start":column_start,"column_end":column_end})
}
fn cell_copy(c: &mut Client, revision: u64, scope: Value) -> Value {
    c.call(
        "leviathan_cell_block_copy",
        json!({"expected_revision":revision,"scope":scope}),
    )
}
fn cell_paste(
    c: &mut Client,
    revision: u64,
    copied: &Value,
    pattern: u64,
    row: u32,
    column: u16,
) -> Value {
    c.call("leviathan_cell_block_preview", json!({"expected_revision":revision,"operation":{"kind":"paste_replace","clipboard_token":copied["data"]["clipboard_token"],"destination":{"pattern":pattern,"track":1,"row_start":row,"column_start":column}}}))
}
fn cell_clear(c: &mut Client, revision: u64, scope: Value) -> Value {
    c.call(
        "leviathan_cell_block_preview",
        json!({"expected_revision":revision,"operation":{"kind":"clear","scope":scope}}),
    )
}
fn cell_commit(c: &mut Client, revision: u64, preview: &Value) -> Value {
    c.call(
        "leviathan_cell_block_commit",
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}
fn cell_inspect(
    c: &mut Client,
    revision: u64,
    value: &Value,
    kind: &str,
    start: usize,
    limit: usize,
) -> Value {
    let mut request = json!({"expected_revision":revision,"kind":kind,"start":start,"limit":limit});
    let key = if kind == "clipboard" {
        "clipboard_token"
    } else {
        "preview_token"
    };
    request[key] = value["data"][key].clone();
    c.call("leviathan_cell_block_inspect", request)
}
fn cell_block_setup(c: &mut Client, f: &Fixture) {
    use aqua_pattern_core::{project, song::*};
    let mut operations = vec![
        json!({"op":"upsert_track","id":1,"name":"Rectangle track","note_columns":3}),
        json!({"op":"upsert_track","id":2,"name":"Other track","note_columns":3}),
        phrase_metadata(7),
        json!({"op":"set_phrase_cell","phrase_id":7,"row":0,"column":0,"cell":{"note":{"Note":67},"velocity":0.5}}),
    ];
    for id in 1..=4 {
        operations
            .push(json!({"op":"upsert_pattern","id":id,"name":format!("Rectangle {id}"),"rows":8}));
    }
    operations.extend([
        json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5,"pan":-0.0,"delay":0.125,"effects":{"retrigger":{"repeats":1,"spacing":0.25},"release":0.75,"glide":{"target":67,"semitones_per_second":12.0},"offset":10,"direction":"Reverse"}}}),
        json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":0,"cell":{"note":"Off","instrument":1,"delay":0.125}}),
        json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":1,"cell":{"note":"Empty","velocity":0.0,"pan":0.0,"delay":0.5}}),
        json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":1,"column":2,"cell":{"note":{"Note":60},"instrument":1,"phrase":{"phrase":7,"cycles":1}}}),
    ]);
    for row in 4..6 {
        for column in 0..3 {
            operations.push(json!({"op":"set_cell","pattern_id":2,"track_id":1,"row":row,"column":column,"cell":{"note":{"Note":62},"instrument":2,"velocity":1.0}}));
        }
    }
    operations.extend([
        json!({"op":"set_track_alias","pattern_id":3,"track_id":1,"source":{"pattern":1,"track":1}}),
        json!({"op":"set_track_alias","pattern_id":4,"track_id":1,"source":{"pattern":2,"track":1}}),
        json!({"op":"set_order","pattern_ids":[1,2,2,3,4]}),
        json!({"op":"set_occurrence_track_mute","occurrence_index":2,"track_id":1,"muted":true}),
    ]);
    let result = edit(c, 0, operations);
    assert_eq!(result["ok"], true, "{result}");
    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    // Existing set_cell deliberately removes default cells; a stored explicit
    // default is nevertheless legal schema1 and must survive rectangle copying.
    song.patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .get_mut(&TrackId(1))
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .insert(2, Cell::default());
    project::save(
        &song,
        &f.root.join("cell-fixture.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":1,"path":"cell-fixture.leviathan.json","discard_unsaved":true}),
    );
}

#[test]
fn cell_block_shared_alias_copy_whole_cells_holes_defaults_paged_uses_paste_clear_and_one_undo() {
    use aqua_pattern_core::{cell_blocks as blocks, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    cell_block_setup(&mut c, &f);
    let before = c.document();
    let base: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let scope = cell_scope(3, 0, 2, 0, 3);
    let frozen = blocks::copy(&base, &serde_json::from_value(scope.clone()).unwrap()).unwrap();
    let copied = cell_copy(&mut c, 2, scope);
    assert_eq!(copied["ok"], true, "{copied}");
    assert_eq!(copied["data"]["clipboard_id"], frozen.id());
    assert_eq!(copied["data"]["summary"], json!(frozen.summary()));
    assert_eq!(copied["data"]["summary"]["positions"], 6);
    assert_eq!(copied["data"]["summary"]["holes"], 1);
    assert_eq!(copied["data"]["summary"]["explicit_default_cells"], 1);
    assert_eq!(
        copied["data"]["summary"]["source_chain"],
        json!([{"pattern":3,"track":1},{"pattern":1,"track":1}])
    );
    let mut records = Vec::new();
    for start in [0, 2, 4, 6] {
        let page = cell_inspect(&mut c, 2, &copied, "clipboard", start, 2);
        assert_eq!(page["ok"], true);
        assert_eq!(page["data"]["page"]["total"], 6);
        records.extend(page["data"]["records"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(json!(records), json!(frozen.page(0, 64).unwrap().cells));
    assert!(records[0]["cell"]["pan"]
        .as_f64()
        .unwrap()
        .is_sign_negative());
    assert!(records[1]["cell"].is_null());
    assert_eq!(records[2]["cell"], json!(Cell::default()));
    let destination = blocks::PasteDestination {
        pattern: PatternId(2),
        track: TrackId(1),
        row_start: 4,
        column_start: 0,
    };
    let shared = blocks::paste_preview(&base, &frozen, &destination).unwrap();
    let preview = cell_paste(&mut c, 2, &copied, 2, 4, 0);
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["data"]["summary"], json!(shared.summary()));
    assert_eq!(preview["data"]["preview_id"], shared.id());
    for (kind, expected) in [
        ("changes", json!(shared.changes())),
        ("contexts", json!(shared.contexts())),
        ("aliases", json!(shared.aliases())),
    ] {
        let page = cell_inspect(&mut c, 2, &preview, kind, 0, 64);
        assert_eq!(page["data"]["records"], expected, "{kind}");
    }
    assert_eq!(
        preview["data"]["record_counts"],
        json!({"changes":6,"contexts":3,"aliases":1})
    );
    assert!(shared
        .contexts()
        .iter()
        .any(|context| context.occurrence == 2 && context.muted));
    assert_eq!(c.document(), before);
    let committed = cell_commit(&mut c, 2, &preview);
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["revision"], 3);
    assert_eq!(committed["data"]["changed"], true);
    assert_eq!(committed["data"]["files_changed"], false);
    let after = c.document()["data"]["song"].clone();
    assert_eq!(after, json!(shared.candidate()));
    let row = &after["patterns"]["2"]["tracks"]["1"]["4"];
    assert!(row.get("1").is_none(), "copied hole removes destination");
    assert_eq!(
        row["2"],
        json!(Cell::default()),
        "explicit default remains stored"
    );
    assert!(row["0"]["pan"].as_f64().unwrap().is_sign_negative());
    assert_eq!(
        cell_inspect(&mut c, 3, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(c.document()["data"]["song"], after);
    let cleared = cell_clear(&mut c, 5, cell_scope(2, 4, 6, 0, 3));
    assert_eq!(cleared["data"]["summary"]["removed_cells"], 5);
    assert_eq!(cell_commit(&mut c, 5, &cleared)["ok"], true);
    assert_eq!(
        c.document()["data"]["song"]["patterns"]["2"]["tracks"]["1"]["4"],
        json!({})
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    assert_eq!(c.document()["data"]["song"], after);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
}

#[test]
fn cell_block_clipboard_is_frozen_across_edits_save_undo_redo_but_new_and_open_revoke_owner() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    cell_block_setup(&mut c, &f);
    let copied = cell_copy(&mut c, 2, cell_scope(1, 0, 1, 0, 1));
    assert_eq!(
        c.call(
            "leviathan_project_open",
            json!({"expected_revision":2,"path":"missing.leviathan.json"})
        )["ok"],
        false
    );
    assert_eq!(
        cell_inspect(&mut c, 2, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    assert_eq!(
        edit(
            &mut c,
            2,
            vec![
                json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":61},"instrument":1}})
            ]
        )["ok"],
        true
    );
    assert_eq!(
        c.call("leviathan_project_new", json!({"expected_revision":3}))["error"]["code"],
        "unsaved_changes"
    );
    let page = cell_inspect(&mut c, 3, &copied, "clipboard", 0, 64);
    assert_eq!(
        page["data"]["records"][0]["cell"]["note"]["Note"], 60,
        "frozen, not a live source read"
    );
    let preview = cell_paste(&mut c, 3, &copied, 2, 6, 0);
    assert_eq!(cell_commit(&mut c, 3, &preview)["ok"], true);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":4,"path":"cells-saved.leviathan.json"}),
    );
    assert_eq!(
        cell_inspect(&mut c, 5, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(
        cell_inspect(&mut c, 6, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    c.ok("leviathan_history_redo", json!({"expected_revision":6}));
    assert_eq!(
        cell_inspect(&mut c, 7, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    let pending = cell_clear(&mut c, 7, cell_scope(2, 6, 7, 0, 1));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"cells-saved.leviathan.json"}),
    );
    assert_eq!(
        cell_inspect(&mut c, 8, &copied, "clipboard", 0, 64)["error"]["code"],
        "cell_block_clipboard_unavailable",
        "even exact-content Open creates a new owner"
    );
    assert_eq!(
        cell_commit(&mut c, 8, &pending)["error"]["code"],
        "cell_block_preview_unavailable"
    );
    let fresh = cell_copy(&mut c, 8, cell_scope(1, 0, 1, 0, 1));
    c.ok("leviathan_project_new", json!({"expected_revision":8}));
    assert_eq!(
        cell_inspect(&mut c, 9, &fresh, "clipboard", 0, 64)["error"]["code"],
        "cell_block_clipboard_unavailable"
    );
}

#[test]
fn cell_block_noop_redo_supersession_failed_copy_and_stale_preview_have_separate_lifetimes() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    cell_block_setup(&mut c, &f);
    let base = c.document()["data"]["song"].clone();
    assert_eq!(edit(&mut c, 2, vec![title("Temporary")])["ok"], true);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    let copied = cell_copy(&mut c, 4, cell_scope(3, 0, 2, 0, 3));
    let noop = cell_paste(&mut c, 4, &copied, 1, 0, 0);
    assert_eq!(noop["data"]["summary"]["changed_cells"], 0);
    let receipt = cell_commit(&mut c, 4, &noop);
    assert_eq!(receipt["ok"], true);
    assert_eq!(receipt["revision"], 4);
    assert_eq!(receipt["data"]["changed"], false);
    assert_eq!(receipt["data"]["session"]["can_redo"], true);
    assert_eq!(receipt["data"]["session"]["dirty"], false);
    assert_eq!(c.document()["data"]["song"], base);
    assert_eq!(
        cell_commit(&mut c, 4, &noop)["error"]["code"],
        "cell_block_preview_unavailable"
    );
    let clear = cell_clear(&mut c, 4, cell_scope(2, 4, 6, 0, 3));
    assert_eq!(cell_copy(&mut c, 4, cell_scope(1, 2, 2, 0, 1))["ok"], false);
    assert_eq!(
        cell_inspect(&mut c, 4, &clear, "changes", 0, 64)["ok"],
        true
    );
    assert_eq!(
        cell_inspect(&mut c, 4, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    let replacement = cell_copy(&mut c, 4, cell_scope(1, 0, 1, 0, 1));
    assert_ne!(
        replacement["data"]["clipboard_token"],
        copied["data"]["clipboard_token"]
    );
    assert_eq!(
        cell_commit(&mut c, 4, &clear)["error"]["code"],
        "cell_block_preview_unavailable"
    );
    assert_eq!(
        cell_inspect(&mut c, 4, &copied, "clipboard", 0, 64)["error"]["code"],
        "invalid_cell_block_clipboard_token"
    );
    let stale = cell_clear(&mut c, 4, cell_scope(2, 4, 6, 0, 3));
    assert_eq!(
        edit(&mut c, 4, vec![title(base["title"].as_str().unwrap())])["revision"],
        4,
        "true no-op preserves review"
    );
    assert_eq!(
        cell_inspect(&mut c, 4, &stale, "changes", 0, 64)["ok"],
        true
    );
    assert_eq!(edit(&mut c, 4, vec![title("Changed")])["revision"], 5);
    c.ok("leviathan_history_undo", json!({"expected_revision":5}));
    assert_eq!(
        cell_commit(&mut c, 6, &stale)["error"]["code"],
        "cell_block_preview_unavailable"
    );
    assert_eq!(
        cell_inspect(&mut c, 6, &replacement, "clipboard", 0, 64)["ok"],
        true
    );
}

#[test]
fn cell_block_strict_inputs_alias_destinations_cross_track_bounds_and_page_tokens_reject_atomically(
) {
    let catalog = mcp::tool_catalog();
    let inspect = catalog
        .iter()
        .find(|tool| tool["name"] == "leviathan_cell_block_inspect")
        .unwrap();
    assert_eq!(inspect["inputSchema"]["oneOf"].as_array().unwrap().len(), 4);
    assert_eq!(inspect["annotations"]["readOnlyHint"], true);
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    cell_block_setup(&mut c, &f);
    let before = c.document();
    let copied = cell_copy(&mut c, 2, cell_scope(1, 0, 2, 0, 3));
    let preview = cell_paste(&mut c, 2, &copied, 2, 4, 0);
    for scope in [
        cell_scope(1, 0, 0, 0, 1),
        cell_scope(1, 2, 1, 0, 1),
        cell_scope(1, 0, 9, 0, 1),
        cell_scope(1, 0, 1, 0, 4),
        cell_scope(999, 0, 1, 0, 1),
        cell_scope(1, 0, 1, 1, 1),
    ] {
        assert_eq!(cell_copy(&mut c, 2, scope)["ok"], false);
    }
    for field in [
        "pattern",
        "track",
        "row_start",
        "row_end",
        "column_start",
        "column_end",
    ] {
        let mut scope = cell_scope(1, 0, 2, 0, 3);
        scope.as_object_mut().unwrap().remove(field);
        assert_eq!(
            cell_copy(&mut c, 2, scope)["error"]["code"],
            "invalid_arguments"
        );
    }
    for (pattern, row, column) in [(3, 0, 0), (2, 7, 0), (2, 0, 1), (999, 0, 0)] {
        assert_eq!(
            cell_paste(&mut c, 2, &copied, pattern, row, column)["ok"],
            false
        );
    }
    assert_eq!(
        cell_clear(&mut c, 2, cell_scope(3, 0, 1, 0, 1))["ok"],
        false
    );
    let cross = c.call("leviathan_cell_block_preview", json!({"expected_revision":2,"operation":{"kind":"paste_replace","clipboard_token":copied["data"]["clipboard_token"],"destination":{"pattern":2,"track":2,"row_start":0,"column_start":0}}}));
    assert_eq!(cross["ok"], false);
    for request in [
        json!({"expected_revision":2,"scope":cell_scope(1,0,1,0,1),"cells":[]}),
        json!({"expected_revision":2,"scope":cell_scope(1,0,1,0,1),"document_owner":"forged"}),
    ] {
        assert_eq!(
            c.call("leviathan_cell_block_copy", request)["error"]["code"],
            "invalid_arguments"
        );
    }
    assert_eq!(c.call("leviathan_cell_block_commit", json!({"expected_revision":2,"preview_token":preview["data"]["preview_token"],"candidate":{}}))["error"]["code"], "invalid_arguments");
    for kind in ["clipboard", "changes", "contexts", "aliases"] {
        let value = if kind == "clipboard" {
            &copied
        } else {
            &preview
        };
        for (start, limit) in [(0, 0), (0, 65), (99999, 64)] {
            assert_eq!(
                cell_inspect(&mut c, 2, value, kind, start, limit)["error"]["code"],
                "invalid_arguments"
            );
        }
    }
    assert_eq!(c.call("leviathan_cell_block_inspect", json!({"expected_revision":2,"kind":"clipboard","preview_token":preview["data"]["preview_token"]}))["error"]["code"], "invalid_arguments");
    assert_eq!(c.call("leviathan_cell_block_inspect", json!({"expected_revision":2,"kind":"changes","preview_token":copied["data"]["clipboard_token"]}))["error"]["code"], "invalid_cell_block_preview_token");
    assert_eq!(c.document(), before);
    assert_eq!(
        cell_commit(&mut c, 2, &preview)["ok"],
        true,
        "all failures preserved the prior review"
    );
}

#[test]
fn cell_block_no_source_io_and_overlapping_paste_use_frozen_raw_cells_not_live_reads() {
    use aqua_pattern_core::{cell_blocks, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    cell_block_setup(&mut c, &f);
    kit_source(&f);
    import_source(&mut c, 2, 7, "kit.wav");
    fs::remove_file(f.root.join("kit.wav")).unwrap();
    let before: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let scope = cell_scope(1, 0, 2, 0, 3);
    let copied = cell_copy(&mut c, 3, scope.clone());
    assert_eq!(
        copied["ok"], true,
        "Copy is model-only with unavailable registered media"
    );
    let snapshot = cell_blocks::copy(&before, &serde_json::from_value(scope).unwrap()).unwrap();
    let shared = cell_blocks::paste_preview(
        &before,
        &snapshot,
        &cell_blocks::PasteDestination {
            pattern: PatternId(1),
            track: TrackId(1),
            row_start: 1,
            column_start: 0,
        },
    )
    .unwrap();
    let preview = cell_paste(&mut c, 3, &copied, 1, 1, 0);
    assert_eq!(preview["ok"], true, "{preview}");
    let committed = cell_commit(&mut c, 3, &preview);
    assert_eq!(committed["ok"], true);
    assert_eq!(committed["data"]["source_identity_verified_now"], false);
    assert_eq!(c.document()["data"]["song"], json!(shared.candidate()));
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 1);
    assert_eq!(
        c.call(
            "leviathan_project_save",
            json!({"expected_revision":4,"path":"still-missing.leviathan.json"})
        )["ok"],
        false,
        "ordinary Save validation is unchanged"
    );
}

#[test]
fn cell_block_signed_zero_editor_delta_advances_existing_batch_revision_and_invalidates_review() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    cell_block_setup(&mut c, &f);
    let before = c.document()["data"]["song"].clone();
    let copied = cell_copy(&mut c, 2, cell_scope(1, 0, 1, 0, 1));
    let reviewed = cell_clear(&mut c, 2, cell_scope(2, 4, 6, 0, 3));
    let mut changed = before["patterns"]["1"]["tracks"]["1"]["0"]["0"].clone();
    assert!(changed["pan"].as_f64().unwrap().is_sign_negative());
    changed["pan"] = json!(0.0);
    let receipt = edit(
        &mut c,
        2,
        vec![
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":changed}),
        ],
    );
    assert_eq!(receipt["ok"], true);
    assert_eq!(
        receipt["data"]["changed"], true,
        "Editor signed-zero transaction must advance MCP too"
    );
    assert_eq!(receipt["revision"], 3);
    assert_eq!(receipt["data"]["session"]["dirty"], true);
    assert_eq!(
        cell_commit(&mut c, 3, &reviewed)["error"]["code"],
        "cell_block_preview_unavailable"
    );
    assert_eq!(
        cell_inspect(&mut c, 3, &copied, "clipboard", 0, 64)["ok"],
        true
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert!(
        c.document()["data"]["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["pan"]
            .as_f64()
            .unwrap()
            .is_sign_negative()
    );
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"}))["data"]["dirty"],
        false
    );
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert!(
        !c.document()["data"]["song"]["patterns"]["1"]["tracks"]["1"]["0"]["0"]["pan"]
            .as_f64()
            .unwrap()
            .is_sign_negative()
    );
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"positive-zero.leviathan.json"}),
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":6}));
    assert_eq!(
        c.ok("leviathan_project_read", json!({"section":"history"}))["data"]["dirty"],
        true,
        "negative zero differs from saved positive zero"
    );
    let noop = edit(
        &mut c,
        7,
        vec![
            json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":before["patterns"]["1"]["tracks"]["1"]["0"]["0"]}),
        ],
    );
    assert_eq!(noop["revision"], 7);
    assert_eq!(noop["data"]["changed"], false);
    assert_eq!(noop["data"]["session"]["can_redo"], true);
}

#[test]
fn cell_block_position_and_clipboard_byte_limits_preserve_previous_handles_without_truncation() {
    use aqua_pattern_core::{cell_blocks, project, song::*};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(
        edit(
            &mut c,
            0,
            vec![
                json!({"op":"upsert_track","id":1,"name":"Wide","note_columns":256}),
                json!({"op":"upsert_pattern","id":1,"name":"Bounded","rows":17}),
            ]
        )["ok"],
        true
    );
    let copied = cell_copy(&mut c, 1, cell_scope(1, 0, 16, 0, 256));
    assert_eq!(copied["ok"], true, "{copied}");
    assert_eq!(copied["data"]["summary"]["positions"], 4096);
    assert_eq!(copied["data"]["summary"]["holes"], 4096);
    let last = cell_inspect(&mut c, 1, &copied, "clipboard", 4032, 64);
    assert_eq!(last["data"]["records"].as_array().unwrap().len(), 64);
    assert!(last["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|record| record["cell"].is_null()));
    assert!(last["data"]["page"]["next_start"].is_null());
    assert_eq!(
        cell_inspect(&mut c, 1, &copied, "clipboard", 4096, 64)["data"]["records"],
        json!([])
    );
    let pending = cell_clear(&mut c, 1, cell_scope(1, 0, 1, 0, 1));
    let rejected = cell_copy(&mut c, 1, cell_scope(1, 0, 17, 0, 256));
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("4096"));
    assert_eq!(
        cell_inspect(&mut c, 1, &copied, "clipboard", 0, 1)["ok"],
        true
    );
    assert_eq!(cell_commit(&mut c, 1, &pending)["data"]["changed"], false);

    let mut song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let rich = Cell {
        note: Note::Note(60),
        instrument: Some(InstrumentId(1)),
        velocity: Some(0.12345679),
        pan: Some(-0.2345679),
        delay: Some(0.12345678912345678),
        effects: StepEffects {
            retrigger: Some(Retrigger {
                repeats: 1,
                spacing: 0.2345678912345678,
            }),
            release: Some(0.9876543212345678),
            glide: Some(Glide {
                target: 74,
                semitones_per_second: 11.234567,
            }),
            offset: Some(u64::from(u32::MAX)),
            direction: Some(Direction::Reverse),
            ..StepEffects::default()
        },
        ..Cell::default()
    };
    assert!(serde_json::to_vec(&rich).unwrap().len() * 4096 > cell_blocks::MAX_CLIPBOARD_BYTES);
    let rows = song
        .patterns
        .get_mut(&PatternId(1))
        .unwrap()
        .tracks
        .entry(TrackId(1))
        .or_default();
    for row in 0..16 {
        for column in 0..256 {
            rows.entry(row).or_default().insert(column, rich.clone());
        }
    }
    project::save(
        &song,
        &f.root.join("cell-byte-budget.leviathan.json"),
        project::SaveMode::CreateNew,
    )
    .unwrap();
    c.ok("leviathan_project_open", json!({"expected_revision":1,"path":"cell-byte-budget.leviathan.json","discard_unsaved":true}));
    let small = cell_copy(&mut c, 2, cell_scope(1, 16, 17, 0, 1));
    let pending = cell_clear(&mut c, 2, cell_scope(1, 0, 1, 0, 1));
    let before = c.document();
    let too_large = cell_copy(&mut c, 2, cell_scope(1, 0, 16, 0, 256));
    assert_eq!(too_large["error"]["code"], "validation_error");
    assert!(too_large["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Cell clipboard"));
    assert_eq!(
        cell_inspect(&mut c, 2, &small, "clipboard", 0, 64)["ok"],
        true
    );
    assert_eq!(
        cell_inspect(&mut c, 2, &pending, "changes", 0, 64)["ok"],
        true
    );
    assert_eq!(c.document(), before);
    assert_eq!(cell_commit(&mut c, 2, &pending)["ok"], true);
}

fn mix_setting(track: u64, gain: f64, balance: f64, mute: bool, solo: bool) -> Value {
    json!({"track":track,"gain":gain,"balance":balance,"mute":mute,"solo":solo})
}
fn mix_config(settings: Vec<Value>) -> Value {
    json!({"version":1,"settings":settings})
}
fn mix_preview(c: &mut Client, revision: u64, config: Value) -> Value {
    c.call(
        "leviathan_track_mix_preview",
        json!({"expected_revision":revision,"config":config}),
    )
}
fn mix_commit(c: &mut Client, revision: u64, preview: &Value) -> Value {
    c.call(
        "leviathan_track_mix_commit",
        json!({"expected_revision":revision,"preview_token":preview["data"]["preview_token"]}),
    )
}
fn mix_setup(c: &mut Client, controlled: bool, second_track: bool) -> u64 {
    c.ok(
        "leviathan_sample_generate",
        json!({"expected_revision":0,"path":"mix-source.wav"}),
    );
    let revision = if controlled {
        c.ok(
            "leviathan_sample_import",
            json!({"expected_revision":0,"sample_id":7,"path":"mix-source.wav"}),
        );
        1
    } else {
        0
    };
    let mut ops = vec![
        json!({"op":"upsert_pattern","id":1,"name":"Track mix","rows":4}),
        json!({"op":"set_order","pattern_ids":[1]}),
        json!({"op":"set_cell","pattern_id":1,"track_id":1,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.5}}),
    ];
    // Set the source before inserting notes; each ordered batch step is valid.
    let source_ops = if controlled {
        vec![
            json!({"op":"upsert_region","id":9,"name":"Full","sample_id":7,"start_frame":0,"end_frame":48000,"root_note":60}),
            json!({"op":"assign_region","instrument_id":1,"region_id":9}),
        ]
    } else {
        vec![
            json!({"op":"upsert_instrument","id":1,"name":"Shared legacy","sample":"mix-source.wav"}),
        ]
    };
    ops.splice(0..0, source_ops);
    if second_track {
        ops.push(json!({"op":"set_cell","pattern_id":1,"track_id":2,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.25}}));
    }
    let result = edit(c, revision, ops);
    assert_eq!(result["ok"], true, "{result}");
    revision + 1
}

#[test]
fn track_mix_shared_controlled_plan_read_commit_one_undo_and_project_roundtrip() {
    use aqua_pattern_core::{song::Song, track_mix};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(mix_setup(&mut c, true, true), 2);
    let base: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let config = mix_config(vec![
        mix_setting(1, 0.50000000001, -0.5, false, false),
        mix_setting(2, 0.25, 1.0, false, false),
    ]);
    let shared =
        track_mix::preview(&base, serde_json::from_value(config.clone()).unwrap()).unwrap();
    let prepared = mix_preview(&mut c, 2, config);
    assert_eq!(prepared["ok"], true, "{prepared}");
    assert_eq!(prepared["data"]["preview_id"], shared.id());
    assert_eq!(
        prepared["data"]["stored_settings"],
        serde_json::to_value(&shared.candidate().track_mix).unwrap()
    );
    assert_eq!(prepared["data"]["stored_settings"]["1"]["gain"], 0.5);
    assert_eq!(
        prepared["data"]["requested_config"]["settings"][0]["gain"],
        0.50000000001
    );
    assert_eq!(
        prepared["data"]["summary"],
        serde_json::to_value(shared.summary()).unwrap()
    );
    assert_eq!(
        prepared["data"]["requested_config"],
        serde_json::to_value(shared.config()).unwrap()
    );
    assert_eq!(
        prepared["data"]["summary"]["layout_status"]["kind"],
        "available"
    );
    assert_eq!(
        prepared["data"]["summary"]["tracks"][0]["coefficients"],
        json!({"left":0.5,"right":0.25})
    );
    assert_eq!(
        prepared["data"]["summary"]["tracks"][1]["coefficients"],
        json!({"left":0.0,"right":0.25})
    );
    for flag in [
        "document_changed",
        "source_identity_verified_now",
        "audio_rendered",
        "live_update_applied",
    ] {
        assert_eq!(prepared["data"][flag], false);
    }
    assert_eq!(
        c.document()["data"]["song"],
        serde_json::to_value(&base).unwrap()
    );
    let applied = mix_commit(&mut c, 2, &prepared);
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["revision"], 3);
    assert_eq!(applied["data"]["changed"], true);
    assert_eq!(applied["data"]["live_update_applied"], false);
    assert_eq!(applied["data"]["session_scope"], "history_only");
    assert_eq!(
        c.document()["data"]["song"],
        serde_json::to_value(shared.candidate()).unwrap()
    );
    let stored = c.ok("leviathan_project_read", json!({"section":"track_mix"}));
    assert_eq!(
        stored["data"]["settings"],
        serde_json::to_value(&shared.candidate().track_mix).unwrap()
    );
    assert_eq!(stored["data"]["compiled_layout_checked"], false);
    assert_eq!(stored["data"]["live_update_applied"], false);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(
        c.document()["data"]["song"],
        serde_json::to_value(&base).unwrap()
    );
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"mix.leviathan.json"}),
    );
    c.ok("leviathan_project_new", json!({"expected_revision":6}));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"mix.leviathan.json"}),
    );
    assert_eq!(
        c.document()["data"]["song"],
        serde_json::to_value(shared.candidate()).unwrap()
    );
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 2);
}

#[test]
fn track_mix_unrenderable_drafts_can_save_without_claiming_compiled_or_live_readiness() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(
        edit(&mut c, 0, vec![json!({"op":"set_order","pattern_ids":[]})])["ok"],
        true
    );
    let config = mix_config(vec![mix_setting(1, 0.75, -0.0, false, false)]);
    let prepared = mix_preview(&mut c, 1, config.clone());
    assert_eq!(prepared["ok"], true, "{prepared}");
    assert_eq!(
        prepared["data"]["summary"]["layout_status"]["kind"],
        "unavailable"
    );
    assert!(prepared["data"]["summary"]["layout_status"]["reason"].is_string());
    assert_eq!(prepared["data"]["summary"]["tracks"], json!([]));
    assert_eq!(prepared["data"]["live_update_applied"], false);
    assert_eq!(mix_commit(&mut c, 1, &prepared)["revision"], 2);
    let no_change = mix_preview(&mut c, 2, config);
    assert_eq!(mix_commit(&mut c, 2, &no_change)["data"]["changed"], false);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":2,"path":"draft-mix.leviathan.json"}),
    );
    let stored = c.document()["data"]["song"].clone();
    assert!(stored["track_mix"]["1"]["balance"]
        .as_f64()
        .unwrap()
        .is_sign_negative());
    assert_eq!(
        c.call("leviathan_song_evaluate", json!({"expected_revision":3}))["ok"],
        false
    );
    c.ok("leviathan_project_new", json!({"expected_revision":3}));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":4,"path":"draft-mix.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], stored);
}

#[test]
fn track_mix_strict_full_set_inputs_f64_bounds_and_failed_requests_preserve_review_atomically() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let valid = mix_config(vec![mix_setting(1, 0.5, 0.0, false, false)]);
    let prepared = mix_preview(&mut c, 0, valid.clone());
    assert_eq!(prepared["ok"], true, "{prepared}");
    let before = c.document();
    let mut configs = vec![
        json!({"settings":[]}),
        json!({"version":2,"settings":[]}),
        json!({"version":1,"settings":{}}),
        mix_config(vec![
            mix_setting(1, 0.5, 0.0, false, false),
            mix_setting(1, 0.25, 0.0, false, false),
        ]),
        mix_config(vec![mix_setting(0, 0.5, 0.0, false, false)]),
        mix_config(vec![mix_setting(999, 0.5, 0.0, false, false)]),
        mix_config(vec![mix_setting(1, 1.00000000001, 0.0, false, false)]),
        mix_config(vec![mix_setting(1, -1e-50, 0.0, false, false)]),
        mix_config(vec![mix_setting(1, 0.5, 1.00000000001, false, false)]),
        mix_config(vec![mix_setting(1, 0.5, -1.00000000001, false, false)]),
        mix_config(vec![mix_setting(1, 0.5, 0.0, false, false); 65]),
        json!({"version":1,"settings":[],"layout_id":"forged"}),
    ];
    for field in ["track", "gain", "balance", "mute", "solo"] {
        let mut entry = mix_setting(1, 0.5, 0.0, false, false);
        entry.as_object_mut().unwrap().remove(field);
        configs.push(mix_config(vec![entry]));
    }
    let mut extra = valid.clone();
    extra["settings"][0]["path"] = json!("/tmp/injected.wav");
    configs.push(extra);
    let mut wrong_bool = valid.clone();
    wrong_bool["settings"][0]["mute"] = Value::Null;
    configs.push(wrong_bool);
    for config in configs {
        let failed = mix_preview(&mut c, 0, config.clone());
        assert_eq!(failed["ok"], false, "accepted {config}: {failed}");
        assert_eq!(c.document(), before);
    }
    for field in ["candidate", "document_owner", "live", "device"] {
        let mut request = json!({"expected_revision":0,"config":valid});
        request[field] = json!({});
        assert_eq!(
            c.call("leviathan_track_mix_preview", request)["error"]["code"],
            "invalid_arguments"
        );
    }
    assert_eq!(c.call("leviathan_track_mix_commit", json!({"expected_revision":0,"preview_token":prepared["data"]["preview_token"],"candidate":before["data"]["song"]}))["error"]["code"], "invalid_arguments");
    assert_eq!(
        c.call(
            "leviathan_track_mix_commit",
            json!({"expected_revision":0,"preview_token":"wrong"})
        )["error"]["code"],
        "invalid_track_mix_preview_token"
    );
    assert_eq!(mix_commit(&mut c, 0, &prepared)["revision"], 1);
    let before = c.document();
    let failed = edit(
        &mut c,
        1,
        vec![
            title("Must roll back"),
            json!({"op":"set_track_mix","config":{"version":1,"settings":[{"track":999,"gain":0.5,"balance":0,"mute":false,"solo":false}]}}),
        ],
    );
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["details"]["operation_index"], 1);
    assert_eq!(c.document(), before);
    let batch = edit(
        &mut c,
        1,
        vec![
            title("One transaction"),
            json!({"op":"set_track_mix","config":{"version":1,"settings":[]}}),
        ],
    );
    assert_eq!(batch["revision"], 2);
    assert!(c.document()["data"]["song"].get("track_mix").is_none());
    c.ok("leviathan_history_undo", json!({"expected_revision":2}));
    assert_eq!(c.document()["data"]["song"], before["data"]["song"]);
}

#[test]
fn track_mix_legacy_owners_require_equal_effective_channels_and_silent_solo_mute_are_explicit() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(mix_setup(&mut c, false, true), 1);
    let equal = mix_config(vec![
        mix_setting(1, 0.5, 0.0, false, false),
        mix_setting(2, 0.5, 0.0, false, false),
    ]);
    let prepared = mix_preview(&mut c, 1, equal);
    assert_eq!(prepared["ok"], true, "{prepared}");
    assert_eq!(
        prepared["data"]["summary"]["tracks"][0]["shared_legacy_tracks"],
        json!([2])
    );
    assert_eq!(
        prepared["data"]["summary"]["tracks"][1]["shared_legacy_instruments"],
        json!([1])
    );
    let failed = mix_preview(
        &mut c,
        1,
        mix_config(vec![mix_setting(1, 0.5, 0.0, false, false)]),
    );
    assert_eq!(failed["error"]["code"], "validation_error");
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("effective L/R"));
    let bad_batch = edit(
        &mut c,
        1,
        vec![
            json!({"op":"set_track_mix","config":mix_config(vec![mix_setting(2,0.1,0.0,false,false)])}),
        ],
    );
    assert_eq!(bad_batch["ok"], false);
    assert_eq!(mix_commit(&mut c, 1, &prepared)["revision"], 2);
    let silent_solo = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(3, 1.0, 0.0, false, true)]),
    );
    assert_eq!(silent_solo["ok"], true, "{silent_solo}");
    assert_eq!(silent_solo["data"]["summary"]["any_solo"], true);
    for row in silent_solo["data"]["summary"]["tracks"].as_array().unwrap() {
        assert_eq!(row["coefficients"], json!({"left":0.0,"right":0.0}));
    }
    assert_eq!(mix_commit(&mut c, 2, &silent_solo)["revision"], 3);
    let mute_wins = mix_preview(
        &mut c,
        3,
        mix_config(vec![mix_setting(1, 0.5, 0.25, true, true)]),
    );
    assert_eq!(mute_wins["ok"], true, "{mute_wins}");
    for row in mute_wins["data"]["summary"]["tracks"].as_array().unwrap() {
        assert_eq!(row["coefficients"], json!({"left":0.0,"right":0.0}));
    }
    assert_eq!(mix_commit(&mut c, 3, &mute_wins)["revision"], 4);
    assert!(kit_export(&mut c, &f, 4, "silent-mix.wav")
        .iter()
        .all(|&value| value == 0.0));
}

#[test]
fn track_mix_review_supersession_noop_redo_aba_save_new_open_and_signed_zero_are_exact() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    edit(&mut c, 0, vec![title("Temporary")]);
    c.ok("leviathan_history_undo", json!({"expected_revision":1}));
    let noop = mix_preview(&mut c, 2, mix_config(vec![]));
    let consumed = mix_commit(&mut c, 2, &noop);
    assert_eq!(consumed["data"]["changed"], false);
    assert_eq!(consumed["revision"], 2);
    assert_eq!(consumed["data"]["session"]["can_redo"], true);
    assert_eq!(consumed["data"]["session"]["dirty"], false);
    assert_eq!(
        mix_commit(&mut c, 2, &noop)["error"]["code"],
        "track_mix_preview_unavailable"
    );
    let first = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(1, 0.5, 0.0, false, false)]),
    );
    let second = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(1, 0.75, -0.0, false, false)]),
    );
    assert_ne!(
        first["data"]["preview_token"],
        second["data"]["preview_token"]
    );
    assert_eq!(
        mix_commit(&mut c, 2, &first)["error"]["code"],
        "invalid_track_mix_preview_token"
    );
    assert_eq!(mix_commit(&mut c, 2, &second)["revision"], 3);
    let review = mix_preview(&mut c, 3, mix_config(vec![]));
    let source = c.document()["data"]["song"].clone();
    let same = edit(
        &mut c,
        3,
        vec![
            json!({"op":"set_track_mix","config":mix_config(vec![mix_setting(1,0.75,-0.0,false,false)])}),
        ],
    );
    assert_eq!(same["data"]["changed"], false);
    assert_eq!(same["revision"], 3);
    let positive = edit(
        &mut c,
        3,
        vec![
            json!({"op":"set_track_mix","config":mix_config(vec![mix_setting(1,0.75,0.0,false,false)])}),
        ],
    );
    assert_eq!(positive["data"]["changed"], true);
    assert_eq!(positive["revision"], 4);
    c.ok("leviathan_history_undo", json!({"expected_revision":4}));
    assert!(c.document()["data"]["song"]["track_mix"]["1"]["balance"]
        .as_f64()
        .unwrap()
        .is_sign_negative());
    assert_eq!(c.document()["data"]["song"], source);
    assert_eq!(
        mix_commit(&mut c, 5, &review)["error"]["code"],
        "track_mix_preview_unavailable"
    );
    let saved_review = mix_preview(&mut c, 5, mix_config(vec![]));
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"mix-history.leviathan.json"}),
    );
    assert_eq!(
        mix_commit(&mut c, 6, &saved_review)["error"]["code"],
        "track_mix_preview_unavailable"
    );
    let opened_review = mix_preview(&mut c, 6, mix_config(vec![]));
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":6,"path":"mix-history.leviathan.json"}),
    );
    assert_eq!(c.document()["data"]["song"], source);
    assert_eq!(
        mix_commit(&mut c, 7, &opened_review)["error"]["code"],
        "track_mix_preview_unavailable"
    );
    let new_review = mix_preview(&mut c, 7, mix_config(vec![]));
    c.ok("leviathan_project_new", json!({"expected_revision":7}));
    assert_eq!(
        mix_commit(&mut c, 8, &new_review)["error"]["code"],
        "track_mix_preview_unavailable"
    );
}

#[test]
fn track_mix_metadata_commit_never_reads_sources_but_normal_batch_save_and_export_keep_policy() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let path = f.root.join("mix-source.wav");
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    let before_files = fs::read_dir(&f.root).unwrap().count();
    let missing = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(1, 0.5, 0.0, false, false)]),
    );
    assert_eq!(missing["ok"], true, "{missing}");
    assert_eq!(
        missing["data"]["summary"]["layout_status"]["kind"],
        "available"
    );
    let failed_batch = edit(
        &mut c,
        2,
        vec![
            json!({"op":"set_track_mix","config":mix_config(vec![mix_setting(1,0.25,0.0,false,false)])}),
        ],
    );
    assert_eq!(failed_batch["ok"], false, "normal batch path checks remain");
    assert_eq!(mix_commit(&mut c, 2, &missing)["revision"], 3);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), before_files);
    assert_eq!(
        c.call(
            "leviathan_project_save",
            json!({"expected_revision":3,"path":"missing.leviathan.json"})
        )["ok"],
        false
    );
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":3,"path":"missing.wav"})
        )["ok"],
        false
    );
    assert!(!f.root.join("missing.wav").exists());
    assert!(!f.root.join("missing.events.json").exists());
    fs::write(&path, b"changed bytes, not audio").unwrap();
    let changed = mix_preview(
        &mut c,
        3,
        mix_config(vec![mix_setting(1, 0.25, 0.0, false, false)]),
    );
    assert_eq!(changed["ok"], true);
    assert_eq!(mix_commit(&mut c, 3, &changed)["revision"], 4);
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":4,"path":"changed.wav"})
        )["ok"],
        false
    );
    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        fs::write(f.base.join("outside-mix.wav"), &original).unwrap();
        std::os::unix::fs::symlink(f.base.join("outside-mix.wav"), &path).unwrap();
        let linked = mix_preview(
            &mut c,
            4,
            mix_config(vec![mix_setting(1, 0.125, 0.0, false, false)]),
        );
        assert_eq!(
            linked["ok"], true,
            "metadata review must not follow symlinks or inspect sources"
        );
        assert_eq!(mix_commit(&mut c, 4, &linked)["revision"], 5);
        assert_eq!(
            c.call(
                "leviathan_project_save",
                json!({"expected_revision":5,"path":"linked.leviathan.json"})
            )["ok"],
            false
        );
        fs::remove_file(&path).unwrap();
    }
    fs::write(&path, original).unwrap();
}

#[test]
fn track_mix_existing_export_uses_persisted_coefficients_from_frame_zero_and_neutral_pcm_is_exact()
{
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let base_manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":2}))["data"]["manifest"].clone();
    let base = kit_export(&mut c, &f, 2, "base-mix.wav");
    assert!(base.iter().any(|sample| sample.abs() > 0.0001));
    let neutral = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(1, 1.0, 0.0, false, false)]),
    );
    assert_eq!(mix_commit(&mut c, 2, &neutral)["revision"], 3);
    let neutral_pcm = kit_export(&mut c, &f, 3, "neutral-mix.wav");
    assert_eq!(
        base.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        neutral_pcm.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(
        c.ok("leviathan_song_evaluate", json!({"expected_revision":3}))["data"]["manifest"],
        base_manifest
    );
    let attenuated = mix_preview(
        &mut c,
        3,
        mix_config(vec![mix_setting(1, 0.5, 1.0, false, false)]),
    );
    assert_eq!(mix_commit(&mut c, 3, &attenuated)["revision"], 4);
    let pcm = kit_export(&mut c, &f, 4, "attenuated-mix.wav");
    assert_eq!(base.len(), pcm.len());
    for (before, after) in base.as_chunks::<2>().0.iter().zip(pcm.as_chunks::<2>().0) {
        assert_eq!(after[0], 0.0);
        assert_eq!(
            after[1].to_bits(),
            (before[1] * 0.5).to_bits(),
            "offline initialization has no live ramp"
        );
    }
    let manifest =
        c.ok("leviathan_song_evaluate", json!({"expected_revision":4}))["data"]["manifest"].clone();
    assert_eq!(manifest["schema"], 11);
    assert_eq!(manifest["events"], base_manifest["events"]);
    let on_disk: Value =
        serde_json::from_slice(&fs::read(f.root.join("attenuated-mix.events.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk, manifest);
    assert_eq!(
        c.call(
            "leviathan_song_export",
            json!({"expected_revision":4,"path":"attenuated-mix.wav"})
        )["ok"],
        false
    );
}

#[test]
fn track_mix_64_settings_and_65_contributing_tracks_report_explicit_unsupported_without_truncation()
{
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, false, true);
    // The original shared legacy sampler keeps its startup gain history. Use a
    // deliberately quiet fixture so 65 simultaneous neutral voices do not trip
    // the unrelated export clipping guard during that preserved startup ramp.
    let source = f.root.join("mix-source.wav");
    let mut reader = hound::WavReader::open(&source).unwrap();
    let spec = reader.spec();
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .map(|s| s.unwrap() * 0.001)
        .collect();
    drop(reader);
    let mut writer = hound::WavWriter::create(&source, spec).unwrap();
    for sample in samples {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    let mut ops = Vec::new();
    for track in 5..=65 {
        ops.push(json!({"op":"upsert_track","id":track,"name":format!("Track {track}"),"note_columns":1}));
    }
    for track in 3..=65 {
        ops.push(json!({"op":"set_cell","pattern_id":1,"track_id":track,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.01}}));
    }
    assert_eq!(edit(&mut c, 1, ops)["revision"], 2);
    let neutral = mix_preview(
        &mut c,
        2,
        mix_config(
            (1..=64)
                .map(|t| mix_setting(t, 1.0, 0.0, false, false))
                .collect(),
        ),
    );
    assert_eq!(neutral["ok"], true, "{neutral}");
    assert_eq!(neutral["data"]["summary"]["configured_tracks"], 64);
    assert_eq!(
        neutral["data"]["summary"]["layout_status"]["kind"],
        "unsupported"
    );
    assert_eq!(
        neutral["data"]["summary"]["layout_status"]["contributing_tracks"],
        65
    );
    assert_eq!(
        neutral["data"]["summary"]["tracks"],
        json!([]),
        "never returns first64 as if complete"
    );
    assert!(serde_json::to_vec(&neutral["data"]).unwrap().len() <= 256 * 1024);
    let rejected = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(1, 0.5, 0.0, false, false)]),
    );
    assert_eq!(rejected["ok"], false);
    let excess = mix_preview(
        &mut c,
        2,
        mix_config(
            (1..=65)
                .map(|t| mix_setting(t, 1.0, 0.0, false, false))
                .collect(),
        ),
    );
    assert_eq!(excess["ok"], false);
    assert_eq!(mix_commit(&mut c, 2, &neutral)["revision"], 3);
    let stored = c.ok("leviathan_project_read", json!({"section":"track_mix"}));
    assert_eq!(stored["data"]["settings"].as_object().unwrap().len(), 64);
    assert!(kit_export(&mut c, &f, 3, "neutral-65-tracks.wav")
        .iter()
        .any(|x| x.abs() > 0.0001));
}

#[test]
fn track_mix_track_removal_requires_explicit_full_set_clear_and_same_id_upsert_preserves_settings()
{
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let prepared = mix_preview(
        &mut c,
        0,
        mix_config(vec![mix_setting(3, 0.5, 0.25, false, false)]),
    );
    assert_eq!(mix_commit(&mut c, 0, &prepared)["revision"], 1);
    let before = c.document();
    for clear_cells in [false, true] {
        let denied = edit(
            &mut c,
            1,
            vec![json!({"op":"remove_track","id":3,"clear_cells":clear_cells})],
        );
        assert_eq!(denied["error"]["code"], "track_mix_reference");
        assert!(denied["error"]["message"]
            .as_str()
            .unwrap()
            .contains("set_track_mix"));
        assert_eq!(c.document(), before);
    }
    let updated = edit(
        &mut c,
        1,
        vec![json!({"op":"upsert_track","id":3,"name":"Still configured","note_columns":2})],
    );
    assert_eq!(updated["revision"], 2);
    assert_eq!(
        c.document()["data"]["song"]["track_mix"],
        before["data"]["song"]["track_mix"]
    );
    let before_remove = c.document()["data"]["song"].clone();
    let removed = edit(
        &mut c,
        2,
        vec![
            json!({"op":"set_track_mix","config":{"version":1,"settings":[]}}),
            json!({"op":"remove_track","id":3,"clear_cells":true}),
        ],
    );
    assert_eq!(removed["revision"], 3);
    assert!(c.document()["data"]["song"]["tracks"].get("3").is_none());
    assert!(c.document()["data"]["song"].get("track_mix").is_none());
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    assert_eq!(c.document()["data"]["song"], before_remove);
}

fn peaks_analyze(c: &mut Client, revision: u64, start: u64, end: u64) -> Value {
    c.call("leviathan_sample_peaks_analyze",json!({"expected_revision":revision,"config":{"version":1,"start_frame":start,"end_frame":end}}))
}
fn peaks_page(
    c: &mut Client,
    revision: u64,
    analysis: &Value,
    start: usize,
    limit: usize,
) -> Value {
    c.call("leviathan_sample_peaks_inspect",json!({"expected_revision":revision,"analysis_token":analysis["data"]["analysis_token"],"start":start,"limit":limit}))
}
fn all_peak_windows(c: &mut Client, revision: u64, analysis: &Value) -> Vec<Value> {
    let total = analysis["data"]["page"]["total"].as_u64().unwrap() as usize;
    let mut windows = Vec::new();
    for start in (0..total).step_by(4) {
        let page = peaks_page(c, revision, analysis, start, 4);
        assert_eq!(page["ok"], true, "{page}");
        assert!(serde_json::to_vec(&page["data"]).unwrap().len() <= 256 * 1024);
        assert_eq!(page["data"]["source_identity_verified_now"], false);
        assert_eq!(page["data"]["pcm_rendered"], false);
        windows.extend(
            page["data"]["page"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
    }
    assert_eq!(windows.len(), total);
    let end = peaks_page(c, revision, analysis, total, 4);
    assert_eq!(end["data"]["page"]["windows"], json!([]));
    assert_eq!(end["data"]["page"]["next_start"], Value::Null);
    windows
}
#[test]
fn sample_peaks_exact_shared_analysis_pages_and_independent_master_pcm_without_files_or_history() {
    use aqua_pattern_core::{meters, song::Song, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    assert_eq!(mix_setup(&mut c, true, true), 2);
    let p = mix_preview(
        &mut c,
        2,
        mix_config(vec![mix_setting(1, 0.5, 0.25, false, false)]),
    );
    assert_eq!(mix_commit(&mut c, 2, &p)["revision"], 3);
    let before = c.document();
    let song: Song = serde_json::from_value(before["data"]["song"].clone()).unwrap();
    let source = fs::read(f.root.join("mix-source.wav")).unwrap();
    let names: Vec<_> = fs::read_dir(&f.root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let config = meters::MeterAnalysisConfig {
        version: 1,
        start_frame: 2400,
        end_frame: 24000,
    };
    let plan = meters::prepare(&song, config).unwrap();
    let shared = meters::analyze(
        &plan,
        |path| {
            assert!(path.starts_with(&f.root));
            Ok(())
        },
        || false,
    )
    .unwrap();
    let analysis = peaks_analyze(&mut c, 3, 2400, 24000);
    assert_eq!(analysis["ok"], true, "{analysis}");
    assert_eq!(analysis["revision"], 3);
    assert_eq!(analysis["data"]["analysis_id"], shared.id);
    assert_eq!(analysis["data"]["plan_id"], plan.id());
    assert_eq!(
        analysis["data"]["summary"],
        serde_json::to_value(&shared.summary).unwrap()
    );
    assert_eq!(analysis["data"]["verified_sources_at_analysis"], 1);
    assert_eq!(analysis["data"]["source_identity_verified_now"], true);
    for field in [
        "document_changed",
        "pcm_returned",
        "files_written",
        "audio_device_started",
        "live_gui_observed",
    ] {
        assert_eq!(analysis["data"][field], false);
    }
    assert!(serde_json::to_vec(&analysis["data"]).unwrap().len() <= 256 * 1024);
    let windows = all_peak_windows(&mut c, 3, &analysis);
    assert_eq!(
        serde_json::to_value(&shared.reports).unwrap(),
        json!(windows)
    );
    assert_eq!(windows.len(), 9);
    assert_eq!(windows[0]["first_song_frame"], 2400);
    assert_eq!(windows[0]["report_index"], 2);
    // Independent master oracle: ordinary Engine with observation DISABLED.
    let mut engine = song_render::Engine::new(&song, None).unwrap();
    let mut pcm = Vec::new();
    let mut block = [0.0f32; 1024];
    loop {
        let n = engine.block(&mut block, 24000).unwrap();
        if n == 0 {
            break;
        }
        pcm.extend_from_slice(&block[..n * 2]);
    }
    for (window, samples) in windows.iter().zip(pcm[4800..].as_chunks::<4800>().0) {
        let mut peaks = [0.0f32; 2];
        for pair in samples.as_chunks::<2>().0 {
            for ch in 0..2 {
                peaks[ch] = peaks[ch].max(pair[ch].abs());
            }
        }
        for ch in 0..2 {
            assert_eq!(
                (window["master_peak"][ch].as_f64().unwrap() as f32).to_bits(),
                peaks[ch].to_bits()
            );
        }
        assert_eq!(window["frames"], 2400);
        assert_eq!(window["partial"], false);
        assert_eq!(window["wrap_count"], 0);
        assert_eq!(window["first_mix_revision"], 0);
        assert_eq!(window["last_mix_revision"], 0);
    }
    assert_eq!(c.document(), before);
    assert_eq!(fs::read(f.root.join("mix-source.wav")).unwrap(), source);
    assert_eq!(
        fs::read_dir(&f.root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>(),
        names
    );
}
#[test]
fn sample_peaks_strict_aligned_ranges_injection_pages_and_successful_supersession() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let old = peaks_analyze(&mut c, 2, 0, 24000);
    assert_eq!(old["ok"], true, "{old}");
    let before = c.document();
    for config in [
        json!({}),
        json!({"version":2,"start_frame":0,"end_frame":2400}),
        json!({"version":1,"start_frame":1,"end_frame":2400}),
        json!({"version":1,"start_frame":0,"end_frame":2401}),
        json!({"version":1,"start_frame":2400,"end_frame":2400}),
        json!({"version":1,"start_frame":4800,"end_frame":2400}),
        json!({"version":1,"start_frame":0,"end_frame":120001}),
        json!({"version":1,"start_frame":0,"end_frame":u64::MAX}),
        json!({"version":1,"start_frame":0,"end_frame":2400,"pcm":[]}),
    ] {
        let denied = c.call(
            "leviathan_sample_peaks_analyze",
            json!({"expected_revision":2,"config":config}),
        );
        assert_eq!(denied["ok"], false, "{denied}");
        assert_eq!(peaks_page(&mut c, 2, &old, 0, 1)["ok"], true);
    }
    for extra in ["song", "layout", "source_path", "device", "candidate"] {
        let mut request =
            json!({"expected_revision":2,"config":{"version":1,"start_frame":0,"end_frame":2400}});
        request[extra] = json!({});
        assert_eq!(
            c.call("leviathan_sample_peaks_analyze", request)["error"]["code"],
            "invalid_arguments"
        );
    }
    for (start, limit) in [(0, 0), (0, 5), (11, 1), (usize::MAX, 1)] {
        assert_eq!(
            peaks_page(&mut c, 2, &old, start, limit)["error"]["code"],
            "invalid_arguments"
        );
    }
    assert_eq!(
        c.call(
            "leviathan_sample_peaks_inspect",
            json!({"expected_revision":2,"analysis_token":"fake"})
        )["error"]["code"],
        "invalid_sample_peaks_token"
    );
    assert_eq!(
        peaks_analyze(&mut c, 1, 0, 2400)["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(c.document(), before);
    let next = peaks_analyze(&mut c, 2, 2400, 4800);
    assert_eq!(next["ok"], true);
    assert_ne!(
        next["data"]["analysis_token"],
        old["data"]["analysis_token"]
    );
    assert_eq!(
        peaks_page(&mut c, 2, &old, 0, 1)["error"]["code"],
        "invalid_sample_peaks_token"
    );
    assert_eq!(
        peaks_page(&mut c, 2, &next, 0, 1)["data"]["page"]["windows"][0]["first_song_frame"],
        2400
    );
}
#[test]
fn sample_peaks_missing_tampered_oversized_and_symlink_sources_preserve_complete_report() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let old = peaks_analyze(&mut c, 2, 0, 2400);
    assert_eq!(old["ok"], true);
    let before = c.document();
    let path = f.root.join("mix-source.wav");
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(peaks_analyze(&mut c, 2, 0, 2400)["ok"], false);
    let read = peaks_page(&mut c, 2, &old, 0, 1);
    assert_eq!(read["ok"], true);
    assert_eq!(read["data"]["source_identity_verified_now"], false);
    let mut changed = bytes.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fs::write(&path, changed).unwrap();
    assert_eq!(peaks_analyze(&mut c, 2, 0, 2400)["ok"], false);
    assert_eq!(peaks_page(&mut c, 2, &old, 0, 1)["ok"], true);
    fs::write(&path, &bytes).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let oversized = peaks_analyze(&mut c, 2, 0, 2400);
    assert_eq!(oversized["ok"], false);
    assert!(oversized["error"]["message"]
        .as_str()
        .unwrap()
        .contains("64 MiB"));
    assert_eq!(peaks_page(&mut c, 2, &old, 0, 1)["ok"], true);
    fs::write(&path, &bytes).unwrap();
    #[cfg(unix)]
    {
        let backup = f.root.join("same-source.wav");
        fs::write(&backup, &bytes).unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&backup, &path).unwrap();
        assert_eq!(
            peaks_analyze(&mut c, 2, 0, 2400)["error"]["code"],
            "path_denied"
        );
        assert_eq!(peaks_page(&mut c, 2, &old, 0, 1)["ok"], true);
        fs::remove_file(&path).unwrap();
        fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(c.document(), before);
    assert_eq!(
        peaks_analyze(&mut c, 2, 0, 2400)["data"]["analysis_id"],
        old["data"]["analysis_id"]
    );
}
#[test]
fn sample_peaks_used_route_io_omits_unused_files_but_includes_later_notes() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let extra = f.root.join("later.wav");
    fs::copy(f.root.join("mix-source.wav"), &extra).unwrap();
    let bytes = fs::read(&extra).unwrap();
    c.ok(
        "leviathan_sample_import",
        json!({"expected_revision":2,"sample_id":8,"path":"later.wav"}),
    );
    fs::remove_file(&extra).unwrap();
    let analysis = peaks_analyze(&mut c, 3, 0, 2400);
    assert_eq!(analysis["ok"], true, "{analysis}");
    assert_eq!(analysis["data"]["verified_sources_at_analysis"], 1);
    assert_eq!(
        c.call(
            "leviathan_project_save",
            json!({"expected_revision":3,"path":"not-ready.leviathan.json"})
        )["ok"],
        false
    );
    assert_eq!(peaks_page(&mut c, 3, &analysis, 0, 1)["ok"], true);
    assert!(!f.root.join("not-ready.leviathan.json").exists());
    fs::write(&extra, &bytes).unwrap();
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_region","id":10,"name":"Later source","sample_id":8,"start_frame":0,"end_frame":48000,"root_note":60}),
                json!({"op":"assign_region","instrument_id":2,"region_id":10}),
                json!({"op":"set_cell","pattern_id":1,"track_id":2,"row":3,"column":0,"cell":{"note":{"Note":60},"instrument":2,"velocity":0.1}})
            ]
        )["revision"],
        4
    );
    let complete = peaks_analyze(&mut c, 4, 0, 2400);
    assert_eq!(complete["ok"], true, "{complete}");
    assert_eq!(complete["data"]["verified_sources_at_analysis"], 2);
    fs::remove_file(&extra).unwrap();
    let failed = peaks_analyze(&mut c, 4, 0, 2400);
    assert_eq!(
        failed["ok"], false,
        "future full-compiled source must be checked: {failed}"
    );
    assert_eq!(peaks_page(&mut c, 4, &complete, 0, 1)["ok"], true);
}
#[test]
fn sample_peaks_path_only_legacy_refuses_even_a_registered_matching_path_before_file_io() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, false, true);
    c.ok(
        "leviathan_sample_import",
        json!({"expected_revision":1,"sample_id":7,"path":"mix-source.wav"}),
    );
    let before = c.document();
    fs::remove_file(f.root.join("mix-source.wav")).unwrap();
    let denied = peaks_analyze(&mut c, 2, 0, 2400);
    assert_eq!(denied["ok"], false);
    assert!(denied["error"]["message"]
        .as_str()
        .unwrap()
        .contains("path-only legacy"));
    assert!(denied["error"]["message"]
        .as_str()
        .unwrap()
        .contains("assign a region"));
    assert_eq!(c.document(), before);
}
#[test]
fn sample_peaks_natural_end_partial_is_exact_and_no_other_range_is_rounded() {
    use aqua_pattern_core::{song::Song, song_render};
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    assert_eq!(
        edit(&mut c, 2, vec![json!({"op":"set_song","bpm":123.0})])["revision"],
        3
    );
    let song: Song = serde_json::from_value(c.document()["data"]["song"].clone()).unwrap();
    let manifest = song_render::evaluate(&song, None).unwrap();
    let terminal = manifest.musical_frames + manifest.tail_frames;
    assert_ne!(terminal % 2400, 0);
    assert!(terminal < 120000);
    let start = (terminal / 2400) * 2400;
    let a = peaks_analyze(&mut c, 3, start, terminal);
    assert_eq!(a["ok"], true, "{a}");
    let windows = all_peak_windows(&mut c, 3, &a);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0]["partial"], true);
    assert_eq!(windows[0]["first_song_frame"], start);
    assert_eq!(windows[0]["last_song_frame_end_exclusive"], terminal);
    assert_eq!(windows[0]["frames"], terminal - start);
    assert_eq!(a["data"]["summary"]["processed_frames"], terminal);
    assert_eq!(peaks_analyze(&mut c, 3, start, terminal - 1)["ok"], false);
    assert_eq!(peaks_analyze(&mut c, 3, start + 1, terminal)["ok"], false);
    assert_eq!(peaks_analyze(&mut c, 3, start, terminal + 1)["ok"], false);
    assert_eq!(peaks_page(&mut c, 3, &a, 0, 1)["ok"], true);
}
#[test]
fn sample_peaks_history_noop_save_new_open_and_signed_zero_mix_invalidate_exactly() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let original_title = c.document()["data"]["song"]["title"].clone();
    assert_eq!(edit(&mut c, 2, vec![title("Temporary")])["revision"], 3);
    c.ok("leviathan_history_undo", json!({"expected_revision":3}));
    let before = c.document();
    let a = peaks_analyze(&mut c, 4, 0, 2400);
    assert_eq!(a["ok"], true);
    assert_eq!(c.document(), before);
    assert_eq!(
        edit(
            &mut c,
            4,
            vec![json!({"op":"set_song","title":original_title})]
        )["revision"],
        4
    );
    assert_eq!(peaks_page(&mut c, 4, &a, 0, 1)["ok"], true);
    c.ok("leviathan_history_redo", json!({"expected_revision":4}));
    assert_eq!(
        peaks_page(&mut c, 5, &a, 0, 1)["error"]["code"],
        "sample_peaks_unavailable"
    );
    let b = peaks_analyze(&mut c, 5, 0, 2400);
    c.ok(
        "leviathan_project_save",
        json!({"expected_revision":5,"path":"peaks.leviathan.json"}),
    );
    assert_eq!(
        peaks_page(&mut c, 6, &b, 0, 1)["error"]["code"],
        "sample_peaks_unavailable"
    );
    let d = peaks_analyze(&mut c, 6, 0, 2400);
    c.ok("leviathan_project_new", json!({"expected_revision":6}));
    assert_eq!(
        peaks_page(&mut c, 7, &d, 0, 1)["error"]["code"],
        "sample_peaks_unavailable"
    );
    c.ok(
        "leviathan_project_open",
        json!({"expected_revision":7,"path":"peaks.leviathan.json"}),
    );
    assert_eq!(
        edit(
            &mut c,
            8,
            vec![
                json!({"op":"set_track_mix","config":mix_config(vec![mix_setting(1,0.5,-0.0,false,false)])})
            ]
        )["revision"],
        9
    );
    let zero = peaks_analyze(&mut c, 9, 0, 2400);
    assert_eq!(zero["ok"], true);
    assert_eq!(
        edit(
            &mut c,
            9,
            vec![
                json!({"op":"set_track_mix","config":mix_config(vec![mix_setting(1,0.5,0.0,false,false)])})
            ]
        )["revision"],
        10
    );
    assert_eq!(
        peaks_page(&mut c, 10, &zero, 0, 1)["error"]["code"],
        "sample_peaks_unavailable"
    );
    c.ok("leviathan_history_undo", json!({"expected_revision":10}));
    assert!(c.document()["data"]["song"]["track_mix"]["1"]["balance"]
        .as_f64()
        .unwrap()
        .is_sign_negative());
    assert_eq!(
        peaks_page(&mut c, 11, &zero, 0, 1)["error"]["code"],
        "sample_peaks_unavailable"
    );
}
#[test]
fn sample_peaks_64_tracks_and_neutral_65_track_master_only_have_no_truncated_totals() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    // Quiet authored PCM, not only velocity: retained gain smoothing must not
    // make this attribution-capacity fixture test the unrelated clipping guard.
    let mut writer = hound::WavWriter::create(
        f.root.join("mix-source.wav"),
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..48000 * 2 {
        writer.write_sample(0.001f32).unwrap();
    }
    writer.finalize().unwrap();
    c.ok(
        "leviathan_sample_import",
        json!({"expected_revision":0,"sample_id":7,"path":"mix-source.wav"}),
    );
    assert_eq!(
        edit(
            &mut c,
            1,
            vec![
                json!({"op":"upsert_region","id":9,"name":"Quiet dense source","sample_id":7,"start_frame":0,"end_frame":48000,"root_note":60}),
                json!({"op":"assign_region","instrument_id":1,"region_id":9}),
                json!({"op":"upsert_pattern","id":1,"name":"Dense controlled tracks","rows":4}),
                json!({"op":"set_order","pattern_ids":[1]})
            ]
        )["revision"],
        2
    );
    let mut ops = Vec::new();
    for track in 5..=64 {
        ops.push(
            json!({"op":"upsert_track","id":track,"name":format!("T{track}"),"note_columns":1}),
        );
    }
    for track in 1..=64 {
        ops.push(json!({"op":"set_cell","pattern_id":1,"track_id":track,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.001}}));
    }
    assert_eq!(edit(&mut c, 2, ops)["revision"], 3);
    let bounded = peaks_analyze(&mut c, 3, 0, 2400);
    assert_eq!(bounded["ok"], true, "{bounded}");
    assert_eq!(
        bounded["data"]["summary"]["layout"]["tracks"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        bounded["data"]["page"]["windows"][0]["track_peaks"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        edit(
            &mut c,
            3,
            vec![
                json!({"op":"upsert_track","id":u64::MAX,"name":"Last exact ID","note_columns":1}),
                json!({"op":"set_cell","pattern_id":1,"track_id":u64::MAX,"row":0,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":0.001}})
            ]
        )["revision"],
        4
    );
    let master = peaks_analyze(&mut c, 4, 0, 2400);
    assert_eq!(master["ok"], true, "{master}");
    assert_eq!(master["data"]["summary"]["layout"]["tracks"], json!([]));
    assert!(master["data"]["summary"]["layout"]["tracks_unavailable"].is_string());
    assert_eq!(
        master["data"]["page"]["windows"][0]["track_peaks"],
        json!([])
    );
    assert!(
        master["data"]["page"]["windows"][0]["master_peak"][0]
            .as_f64()
            .unwrap()
            > 0.0
    );
}
#[test]
fn sample_peaks_catalog_declares_offline_units_strict_bounds_and_no_live_tools() {
    let catalog = mcp::tool_catalog();
    assert_eq!(catalog.len(), 64);
    let analyze = catalog
        .iter()
        .find(|t| t["name"] == "leviathan_sample_peaks_analyze")
        .unwrap();
    let inspect = catalog
        .iter()
        .find(|t| t["name"] == "leviathan_sample_peaks_inspect")
        .unwrap();
    assert_eq!(analyze["annotations"]["readOnlyHint"], true);
    assert_eq!(inspect["annotations"]["readOnlyHint"], true);
    assert_eq!(
        analyze["inputSchema"]["properties"]["config"]["additionalProperties"],
        false
    );
    assert_eq!(
        analyze["inputSchema"]["properties"]["config"]["properties"]["start_frame"]["multipleOf"],
        2400
    );
    assert_eq!(inspect["inputSchema"]["properties"]["limit"]["maximum"], 4);
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let caps = c.ok("leviathan_capabilities", json!({}));
    assert_eq!(
        caps["data"]["sample_peaks"]["limits"]["processed_song_frames"],
        120000
    );
    assert_eq!(
        caps["data"]["sample_peaks"]["limits"]["completed_result_bytes"],
        1024 * 1024
    );
    assert!(caps["data"]["unavailable"]["live_transport"].is_string());
}

#[test]
fn sample_peaks_render_clipping_returns_no_partial_success_and_preserves_prior_complete_prefix() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    mix_setup(&mut c, true, false);
    let mut ops = Vec::new();
    for track in 5..=16 {
        ops.push(
            json!({"op":"upsert_track","id":track,"name":format!("Loud {track}"),"note_columns":1}),
        );
    }
    for track in 1..=16 {
        ops.push(json!({"op":"set_cell","pattern_id":1,"track_id":track,"row":1,"column":0,"cell":{"note":{"Note":60},"instrument":1,"velocity":1.0}}));
    }
    assert_eq!(edit(&mut c, 2, ops)["revision"], 3);
    let before = c.document();
    let early = peaks_analyze(&mut c, 3, 0, 2400);
    assert_eq!(early["ok"], true, "{early}");
    let failed = peaks_analyze(&mut c, 3, 0, 24000);
    assert_eq!(failed["ok"], false, "{failed}");
    assert!(failed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("clipping"));
    assert_eq!(peaks_page(&mut c, 3, &early, 0, 1)["ok"], true);
    assert_eq!(c.document(), before);
}

#[test]
fn sample_peaks_no_trigger_song_refuses_without_fabricating_zero_windows_or_byte_verification() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    let before = c.document();
    let result = peaks_analyze(&mut c, 0, 0, 2400);
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], "validation_error");
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no note triggers"));
    assert_eq!(result["data"], json!({}));
    assert_eq!(c.document(), before);
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 0);
}

#[test]
fn sample_peaks_above_unity_track_subtotals_are_not_clamped_or_fed_back_into_master() {
    let f = Fixture::new();
    let mut c = Client::ready(&f);
    for (sample, value) in [(61, 1.0f32), (62, -1.0f32)] {
        let name = format!("constant-{sample}.wav");
        let mut writer = hound::WavWriter::create(
            f.root.join(&name),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..4800 * 2 {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        c.ok(
            "leviathan_sample_import",
            json!({"expected_revision":sample-61,"sample_id":sample,"path":name}),
        );
    }
    let mut ops = vec![
        json!({"op":"upsert_pattern","id":1,"name":"Canceling tracks","rows":4}),
        json!({"op":"set_order","pattern_ids":[1]}),
    ];
    for track in [1, 2] {
        ops.push(json!({"op":"upsert_track","id":track,"name":format!("Signed {track}"),"note_columns":8}));
        ops.push(json!({"op":"upsert_region","id":track,"name":"Constant","sample_id":track+60,"start_frame":0,"end_frame":4800,"root_note":60}));
        ops.push(json!({"op":"assign_region","instrument_id":track,"region_id":track}));
        for column in 0..8 {
            ops.push(json!({"op":"set_cell","pattern_id":1,"track_id":track,"row":0,"column":column,"cell":{"note":{"Note":60},"instrument":track,"velocity":1.0}}));
        }
    }
    assert_eq!(edit(&mut c, 2, ops)["revision"], 3);
    let before = c.document();
    let result = peaks_analyze(&mut c, 3, 0, 2400);
    assert_eq!(result["ok"], true, "{result}");
    let window = &result["data"]["page"]["windows"][0];
    assert!(window["track_peaks"][0][0].as_f64().unwrap() > 1.0);
    assert!(window["track_peaks"][1][0].as_f64().unwrap() > 1.0);
    for channel in 0..2 {
        assert!(window["master_peak"][channel].as_f64().unwrap() < 0.00001);
    }
    assert_eq!(c.document(), before);
}
