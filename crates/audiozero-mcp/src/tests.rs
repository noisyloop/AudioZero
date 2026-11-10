use super::*;
use audiozero_engine::{Device, demo};

/// Records what reached the "audio thread".
#[derive(Default)]
struct Fake {
    loads: usize,
    applied: Vec<Changes>,
    commands: Vec<String>,
    playing: bool,
}

impl Audio for Fake {
    fn load(&mut self, _: &Project) -> Result<Vec<String>, String> {
        self.loads += 1;
        Ok(Vec::new())
    }
    fn apply(&mut self, _: &Project, changes: &Changes) -> Result<Vec<String>, String> {
        self.applied.push(changes.clone());
        Ok(Vec::new())
    }
    fn send(&mut self, command: Command) -> bool {
        match command {
            Command::Play => self.playing = true,
            Command::Stop => self.playing = false,
            _ => {}
        }
        self.commands.push(match command {
            Command::Play => "play".into(),
            Command::Stop => "stop".into(),
            Command::Seek(b) => format!("seek {b}"),
            _ => "other".into(),
        });
        true
    }
    fn position(&self) -> f64 {
        0.0
    }
    fn is_playing(&self) -> bool {
        self.playing
    }
}

fn temp_folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "audiozero-mcp-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn server_with(project: Project, file: Option<PathBuf>, folder: PathBuf) -> Server<Fake> {
    let rules = SaveRules { file, folder };
    Server::new(project, rules, Fake::default(), Box::new(io::sink())).unwrap()
}

fn server() -> Server<Fake> {
    server_with(demo::project(), None, temp_folder("unused"))
}

fn request(server: &mut Server<Fake>, id: u64, method: &str, params: Value) -> Value {
    let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let reply = server
        .handle(&message.to_string())
        .expect("a request gets a reply");
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], id);
    reply
}

/// Calls a tool and returns (is_error, parsed result or the error text).
fn tool(server: &mut Server<Fake>, name: &str, arguments: Value) -> (bool, Value) {
    let reply = request(
        server,
        1,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    let is_error = result["isError"].as_bool().unwrap();
    let value = if is_error {
        json!(text)
    } else {
        serde_json::from_str(text).unwrap()
    };
    (is_error, value)
}

fn ok(server: &mut Server<Fake>, name: &str, arguments: Value) -> Value {
    let (is_error, value) = tool(server, name, arguments);
    assert!(!is_error, "{name} failed: {value}");
    value
}

fn refused(server: &mut Server<Fake>, name: &str, arguments: Value) -> String {
    let (is_error, value) = tool(server, name, arguments);
    assert!(is_error, "{name} should have been refused, got {value}");
    value.as_str().unwrap().to_owned()
}

#[test]
fn initialize_negotiates_the_version() {
    let mut s = server();
    let reply = request(
        &mut s,
        1,
        "initialize",
        json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }),
    );
    let result = &reply["result"];
    assert_eq!(result["protocolVersion"], "2025-03-26");
    assert!(result["capabilities"]["tools"].is_object());
    assert_eq!(result["serverInfo"]["name"], "audiozero");
    assert!(result["instructions"].as_str().unwrap().contains("beats"));

    let reply = request(
        &mut s,
        2,
        "initialize",
        json!({ "protocolVersion": "1999-01-01" }),
    );
    assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    assert!(s.activity().any(|a| a == "connected: test"));
}

#[test]
fn tools_list_is_well_formed() {
    let mut s = server();
    let reply = request(&mut s, 1, "tools/list", json!({}));
    let tools = reply["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 20);
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), tools.len(), "tool names are unique");
    for t in tools {
        let schema = &t["inputSchema"];
        assert_eq!(schema["type"], "object");
        let properties = schema["properties"].as_object().unwrap();
        assert!(!properties.contains_key("_"));
        for required in schema["required"].as_array().unwrap() {
            assert!(
                properties.contains_key(required.as_str().unwrap()),
                "{}: required {required} isn't a property",
                t["name"]
            );
        }
        assert!(t["annotations"]["readOnlyHint"].is_boolean());
        assert!(!t["description"].as_str().unwrap().is_empty());
    }
    let add_notes = tools.iter().find(|t| t["name"] == "add_notes").unwrap();
    assert_eq!(
        add_notes["inputSchema"]["properties"]["notes"]["items"]["required"],
        json!(["pitch", "start", "length"])
    );
}

#[test]
fn protocol_edge_cases() {
    let mut s = server();
    assert_eq!(
        s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None,
        "notifications get no reply"
    );
    assert_eq!(
        s.handle(r#"{"jsonrpc":"2.0","id":5,"result":{}}"#),
        None,
        "responses get no reply"
    );
    let reply = request(&mut s, 3, "ping", json!({}));
    assert_eq!(reply["result"], json!({}));
    let reply = request(&mut s, 4, "resources/list", json!({}));
    assert_eq!(reply["error"]["code"], -32601);

    let parse: Value = serde_json::from_str(&s.handle("{not json").unwrap()).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
    let batch: Value = serde_json::from_str(&s.handle("[]").unwrap()).unwrap();
    assert_eq!(batch["error"]["code"], -32600);

    let reply = request(&mut s, 6, "tools/call", json!({ "name": "format_disk" }));
    assert_eq!(reply["error"]["code"], -32602);
}

#[test]
fn builds_a_song_and_undoes_it() {
    let mut s = server_with(Project::default(), None, temp_folder("unused"));
    let track = ok(
        &mut s,
        "add_track",
        json!({ "name": "Drums", "instrument": "kick" }),
    );
    assert_eq!(track["track"], 0);
    let clip = ok(
        &mut s,
        "add_clip",
        json!({
            "track": 0, "start": 0, "length": 4, "name": "Four on the floor",
            "notes": [
                { "pitch": 36, "start": 0, "length": 0.5 },
                { "pitch": 36, "start": 1, "length": 0.5, "velocity": 0.6 },
                { "pitch": 36, "start": 2, "length": 0.5 },
                { "pitch": 36, "start": 3, "length": 0.5 }
            ]
        }),
    );
    assert_eq!(
        (clip["clip"].clone(), clip["notes"].clone()),
        (json!(0), json!(4))
    );

    let detail = ok(&mut s, "get_clip", json!({ "track": 0, "clip": 0 }));
    assert_eq!(detail["notes"][1]["velocity"], 0.6);
    assert_eq!(
        detail["notes"][0]["velocity"], 0.8,
        "velocity defaults to 0.8"
    );

    ok(&mut s, "duplicate_clip", json!({ "track": 0, "clip": 0 }));
    ok(
        &mut s,
        "transpose_notes",
        json!({ "track": 0, "clip": 1, "semitones": 2 }),
    );
    let project = ok(&mut s, "get_project", json!({}));
    assert_eq!(project["tracks"][0]["instrument"], "kick");
    assert_eq!(project["tracks"][0]["clips"][1]["start"], 4.0);
    assert_eq!(project["song_ends_at"], 8.0);
    assert_eq!(project["unsaved_changes"], true);

    // Undo the transpose, the duplicate, the clip and the track.
    for _ in 0..4 {
        ok(&mut s, "undo", json!({}));
    }
    assert!(s.session().project().tracks.is_empty());
    let message = refused(&mut s, "undo", json!({}));
    assert_eq!(message, "nothing to undo");
    ok(&mut s, "redo", json!({}));
    assert_eq!(s.session().project().tracks[0].name, "Drums");
}

#[test]
fn edits_reach_the_audio_thread() {
    let mut s = server();
    assert_eq!(s.audio().loads, 1, "the project is loaded at start");
    ok(&mut s, "add_track", json!({}));
    assert!(s.audio().applied.last().unwrap().reload);
    ok(
        &mut s,
        "add_notes",
        json!({ "track": 1, "clip": 0, "notes": [{ "pitch": 50, "start": 0.5, "length": 1 }] }),
    );
    assert_eq!(s.audio().applied.last().unwrap().clips, vec![1]);
    ok(&mut s, "set_track_mix", json!({ "track": 0, "mute": true }));
    assert_eq!(s.audio().applied.last().unwrap().mix, vec![0]);
    assert!(s.session().project().tracks[0].mute);

    ok(&mut s, "play", json!({ "from": 8 }));
    assert_eq!(s.audio().commands, vec!["seek 8", "play"]);
    let project = ok(&mut s, "get_project", json!({}));
    assert_eq!(project["playing"], true);
    ok(&mut s, "stop", json!({}));
    assert!(!s.audio().playing);
}

#[test]
fn bad_requests_are_tool_errors_and_change_nothing() {
    let mut s = server();
    let before = s.session().project().clone();
    let applied = s.audio().applied.len();

    let m = refused(
        &mut s,
        "add_track",
        json!({ "instrument": "/tmp/evil.vst3" }),
    );
    assert!(m.contains("unknown instrument"), "{m}");
    let m = refused(&mut s, "remove_track", json!({ "track": 99 }));
    assert!(m.contains("no track 99"), "{m}");
    let m = refused(&mut s, "get_clip", json!({ "track": 0, "clip": 99 }));
    assert!(m.contains("no clip 99"), "{m}");
    let m = refused(
        &mut s,
        "rename_track",
        json!({ "track": 0, "name": "x", "path": "/etc" }),
    );
    assert!(m.contains("unknown field"), "{m}");
    let m = refused(
        &mut s,
        "add_notes",
        json!({ "track": 0, "clip": 0, "notes": [{ "pitch": 300, "start": 0, "length": 1 }] }),
    );
    assert!(m.contains("bad arguments"), "{m}");
    let m = refused(
        &mut s,
        "add_notes",
        json!({ "track": 0, "clip": 0, "notes": [{ "pitch": 60, "start": 9999, "length": 1 }] }),
    );
    assert!(m.contains("clip is"), "{m}");
    let m = refused(
        &mut s,
        "add_clip",
        json!({ "track": 0, "start": -1, "length": 4 }),
    );
    assert!(m.contains("clip start"), "{m}");
    let m = refused(
        &mut s,
        "set_transport",
        json!({ "loop_enabled": true, "bpm": 5000 }),
    );
    assert!(m.contains("bpm"), "{m}");
    let m = refused(&mut s, "play", json!({ "from": -4 }));
    assert!(m.contains("from"), "{m}");
    let too_many: Vec<Value> = (0..MAX_NOTES_PER_CALL + 1)
        .map(|_| json!({ "pitch": 60, "start": 0, "length": 1 }))
        .collect();
    let m = refused(
        &mut s,
        "set_notes",
        json!({ "track": 0, "clip": 0, "notes": too_many }),
    );
    assert!(m.contains("limit"), "{m}");

    assert_eq!(*s.session().project(), before);
    assert!(!s.session().can_undo() && !s.session().is_dirty());
    assert_eq!(s.audio().applied.len(), applied);
    assert!(s.activity().any(|a| a.starts_with("add_track refused")));
}

#[test]
fn transport_settings_apply_together() {
    let mut s = server();
    let r = ok(
        &mut s,
        "set_transport",
        json!({ "bpm": 128, "loop_enabled": true, "loop_start": 4, "loop_end": 12, "master_volume": 1.0 }),
    );
    assert_eq!(r["bpm"], 128.0);
    let p = s.session().project();
    assert_eq!(
        (p.loop_enabled, p.loop_start, p.loop_end),
        (true, 4.0, 12.0)
    );
    assert!(
        !s.session().can_undo(),
        "transport settings aren't undo steps"
    );
}

#[test]
fn instruments_are_built_ins_only() {
    let mut s = server();
    let list = ok(&mut s, "list_instruments", json!({}));
    let ids: Vec<&str> = list["instruments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"sine") && ids.contains(&"kick"));
    ok(
        &mut s,
        "set_instrument",
        json!({ "track": 0, "instrument": "none" }),
    );
    assert_eq!(s.session().project().tracks[0].instrument, None);
    ok(
        &mut s,
        "set_instrument",
        json!({ "track": 0, "instrument": "kick" }),
    );
    assert_eq!(
        s.session().project().tracks[0].instrument,
        Some(Device::Builtin { id: "kick".into() })
    );
}

#[test]
fn save_creates_new_files_only_in_the_folder() {
    let folder = temp_folder("save");
    let mut s = server_with(demo::project(), None, folder.clone());
    let m = refused(&mut s, "save", json!({}));
    assert!(m.contains("give it a name"), "{m}");
    for bad in ["../escape", "a/b", ".hidden", "", "x\\y", "C:evil"] {
        let m = refused(&mut s, "save", json!({ "name": bad }));
        assert!(m.contains("letters, digits"), "{bad}: {m}");
    }
    ok(
        &mut s,
        "rename_track",
        json!({ "track": 0, "name": "Saved" }),
    );
    let r = ok(&mut s, "save", json!({ "name": "My song" }));
    assert_eq!(r["saved"], "My song.json");
    let path = folder.join("My song.json");
    assert_eq!(Project::load(&path).unwrap().tracks[0].name, "Saved");
    assert!(!s.session().is_dirty());

    // Now it has a file: later saves go there, and a name is refused.
    refused(&mut s, "save", json!({ "name": "Other" }));
    ok(
        &mut s,
        "rename_track",
        json!({ "track": 0, "name": "Again" }),
    );
    ok(&mut s, "save", json!({}));
    assert_eq!(Project::load(&path).unwrap().tracks[0].name, "Again");

    // A new server can't replace the existing file.
    let mut other = server_with(demo::project(), None, folder.clone());
    let m = refused(&mut other, "save", json!({ "name": "My song" }));
    assert!(m.contains("already exists"), "{m}");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn save_writes_to_the_project_it_was_started_with() {
    let folder = temp_folder("file");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("song.json");
    demo::project().save(&file).unwrap();
    let mut s = server_with(
        Project::load(&file).unwrap(),
        Some(file.clone()),
        folder.clone(),
    );
    ok(&mut s, "remove_track", json!({ "track": 0 }));
    let project = ok(&mut s, "get_project", json!({}));
    assert_eq!(project["file"], "song.json", "only the file name is shown");
    ok(&mut s, "save", json!({}));
    assert_eq!(
        Project::load(&file).unwrap().tracks.len(),
        demo::project().tracks.len() - 1
    );
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn serves_newline_delimited_messages() {
    let mut s = server();
    let huge = format!("{}\n", "x".repeat(MAX_MESSAGE_BYTES + 10));
    let input = format!(
        "{}\n\n{huge}{}\n{}",
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        // The last message has no newline before the input ends.
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "add_track", "arguments": {} } }),
    );
    let mut output = Vec::new();
    serve(io::Cursor::new(input), &mut output, |m| s.handle(m)).unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 3, "initialize, too long, add_track");
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["error"]["code"], -32600);
    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(
        s.session().project().tracks.len(),
        demo::project().tracks.len() + 1
    );
}

#[test]
fn works_with_the_real_engine() {
    use audiozero_engine::control::channel;
    let (controller, mut player) = channel(48_000.0, 512);
    let rules = SaveRules {
        file: None,
        folder: temp_folder("unused"),
    };
    let mut s = Server::new(demo::project(), rules, controller, Box::new(io::sink())).unwrap();
    let call = |s: &mut Server<Controller>, name: &str, args: Value| {
        let m = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
        let reply: Value = serde_json::from_str(&s.handle(&m.to_string()).unwrap()).unwrap();
        assert_eq!(reply["result"]["isError"], false, "{reply}");
    };
    call(&mut s, "add_track", json!({ "instrument": "kick" }));
    call(&mut s, "play", json!({ "from": 4 }));
    let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
    player.process(&mut l, &mut r);
    assert!(s.audio().is_playing());
    assert!(s.audio().position() > 4.0);
    assert!(l.iter().any(|x| x.abs() > 0.001), "the demo is audible");
}
