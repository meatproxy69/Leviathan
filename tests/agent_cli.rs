use aqua_pattern_core::{editor::Editor, mcp::Server, session::Session};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "leviathan-agent-cli-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_agent(root: &std::path::Path, requests: &[Vec<u8>]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_leviathan_agent"))
        .arg("--workspace-root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in requests {
            stdin.write_all(request).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "agent failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn line(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn direct_agent_discovers_all_operations_and_roundtrips_saved_document() {
    let workspace = Workspace::new();
    let responses = run_agent(
        &workspace.0,
        &[
            line(json!({"command":"discover"})),
            line(
                json!({"command":"call","name":"leviathan_project_read","arguments":{"section":"summary"}}),
            ),
            line(
                json!({"command":"call","name":"leviathan_edit_batch","arguments":{"expected_revision":0,"operations":[{"op":"set_song","title":"Direct Agent Roundtrip"}]}}),
            ),
            line(
                json!({"command":"call","name":"leviathan_project_save","arguments":{"expected_revision":1,"path":"roundtrip.leviathan.json"}}),
            ),
            line(
                json!({"command":"call","name":"leviathan_project_new","arguments":{"expected_revision":2}}),
            ),
            line(
                json!({"command":"call","name":"leviathan_project_open","arguments":{"expected_revision":3,"path":"roundtrip.leviathan.json"}}),
            ),
            line(
                json!({"command":"call","name":"leviathan_project_read","arguments":{"section":"document"}}),
            ),
        ],
    );
    assert_eq!(responses.len(), 7);
    let tools = responses[0]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 64);
    assert!(tools.iter().all(|tool| tool["inputSchema"].is_object()));
    assert_eq!(responses[1]["revision"], 0);
    assert_eq!(responses[2]["revision"], 1);
    assert_eq!(responses[3]["revision"], 2);
    assert_eq!(responses[4]["revision"], 3);
    assert_eq!(responses[5]["revision"], 4);
    assert_eq!(
        responses[6]["data"]["song"]["title"],
        "Direct Agent Roundtrip"
    );
    assert!(workspace.0.join("roundtrip.leviathan.json").is_file());
}

#[test]
fn direct_agent_returns_structured_errors_and_recovers_after_bad_or_oversized_lines() {
    let workspace = Workspace::new();
    let oversized = vec![b'x'; 1024 * 1024 + 1];
    let responses = run_agent(
        &workspace.0,
        &[
            b"not json".to_vec(),
            line(json!({"command":"call","name":"not_a_capability","arguments":{}})),
            oversized,
            line(json!({"command":"help"})),
        ],
    );
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["error"]["code"], "parse_error");
    assert_eq!(responses[1]["error"]["code"], "capability_unavailable");
    assert_eq!(responses[2]["error"]["code"], "message_too_large");
    assert_eq!(responses[3]["protocol"], "leviathan-agent/1");
}

#[test]
fn live_document_bridge_uses_shared_editor_and_tracks_external_revision_and_owner() {
    let workspace = Workspace::new();
    let mut server = Server::new(&workspace.0).unwrap();
    let mut editor = Editor::default();
    let mut session = Session::default();
    editor.edit_mode = true;
    editor.row = 3;
    editor.column = 0;
    session.notice = "GUI session sentinel".into();
    let owner = editor.document_owner();

    assert_eq!(
        server.agent_revision_with_document(&mut editor, &mut session),
        0
    );
    let capabilities = server.agent_request_with_document(
        &mut editor,
        &mut session,
        json!({"command":"call","name":"leviathan_capabilities","arguments":{}}),
    );
    assert_eq!(capabilities["data"]["scope"], "bound_live_gui_document");
    assert!(capabilities["data"]["unavailable"]["gui_instance_control"]
        .as_str()
        .unwrap()
        .contains("bound to the live GUI"));
    let mix_read = server.agent_request_with_document(
        &mut editor,
        &mut session,
        json!({"command":"call","name":"leviathan_project_read","arguments":{"section":"track_mix"}}),
    );
    assert!(mix_read["data"]["notice"]
        .as_str()
        .unwrap()
        .contains("bound live GUI document"));
    let preview = server.agent_request_with_document(
        &mut editor, &mut session,
        json!({"command":"call","name":"leviathan_track_mix_preview","arguments":{"expected_revision":0,"config":{"version":1,"settings":[]}}}),
    );
    assert_eq!(preview["ok"], true);
    let token = preview["data"]["preview_token"]
        .as_str()
        .unwrap()
        .to_owned();

    let changed = server.agent_request_with_document(
        &mut editor, &mut session,
        json!({"command":"call","name":"leviathan_edit_batch","arguments":{"expected_revision":0,"operations":[{"op":"set_song","title":"Changed through live editor"}]}}),
    );
    assert_eq!(changed["ok"], true);
    assert_eq!(editor.song.title, "Changed through live editor");
    assert_eq!(editor.revision(), 1);
    assert!(editor.can_undo());
    assert!(editor.owns_document(&owner));
    assert_eq!(editor.row, 3);
    assert_eq!(editor.column, 0);
    assert!(editor.edit_mode, "bridge must not alter GUI Edit Mode");
    assert_eq!(session.notice, "GUI session sentinel");
    assert_eq!(
        server.agent_revision_with_document(&mut editor, &mut session),
        1
    );
    let stale_commit = server.agent_request_with_document(
        &mut editor, &mut session,
        json!({"command":"call","name":"leviathan_track_mix_commit","arguments":{"expected_revision":0,"preview_token":token}}),
    );
    assert_eq!(stale_commit["ok"], false);
    assert_eq!(stale_commit["error"]["code"], "revision_conflict");

    let mut replacement = Editor::default();
    replacement.edit_mode = true;
    let mut candidate = replacement.song.clone();
    candidate.title = "Different owner, same editor revision".into();
    replacement.apply_document_edit(candidate).unwrap();
    replacement.edit_mode = false;
    assert_eq!(replacement.revision(), editor.revision());
    assert!(!replacement.owns_document(&owner));
    let replacement_owner = replacement.document_owner();
    let current = server.agent_revision_with_document(&mut replacement, &mut session);
    assert_eq!(
        current, 2,
        "owner change advances the session revision even when Editor revisions match"
    );
    assert!(replacement.owns_document(&replacement_owner));
    assert!(
        !replacement.edit_mode,
        "bridge must not force Edit Mode on a replacement document"
    );

    let rejected = server.agent_request_with_document(
        &mut replacement, &mut session,
        json!({"command":"call","name":"leviathan_edit_batch","arguments":{"expected_revision":2,"operations":[{"op":"set_song","title":"Must remain unchanged"}]}}),
    );
    assert_eq!(rejected["ok"], false);
    assert_eq!(
        replacement.song.title,
        "Different owner, same editor revision"
    );
    assert!(!replacement.edit_mode);
    assert!(replacement.owns_document(&replacement_owner));
    assert_eq!(session.notice, "GUI session sentinel");
    assert!(server
        .agent_resolve_path("safe.leviathan.json", false)
        .unwrap()
        .starts_with(&workspace.0));
    assert!(server
        .agent_resolve_path("../outside.leviathan.json", false)
        .is_err());
}
