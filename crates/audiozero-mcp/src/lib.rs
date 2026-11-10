//! An [MCP](https://modelcontextprotocol.io) server that lets an AI assistant build and edit
//! AudioZero songs.
//!
//! Every tool is a [`Session`] operation, so an assistant's edits get the same validation as a
//! project file and the same undo stack as edits made in the window. The server follows the
//! rules in `docs/SECURITY_REVIEW.md` ("Before the MCP server ships"):
//!
//! - **Transport:** stdio only. Nothing listens on the network.
//! - **No file system or plugin primitives:** no tool takes a path. `save` writes to the project
//!   the server was started with, or creates a new file (never overwriting) in the AudioZero
//!   folder under a plain name. Instruments are the built-in ones, by id; nothing loads plugin
//!   code.
//! - **Same validation as file load:** arguments go through [`Session`]'s checks and
//!   [`Project::sanitize`]. Messages and note lists have size caps.
//! - **Undo and audit:** every edit is an undo step (except mixer and transport settings, as in
//!   the window), and every call is written to an activity log (stderr), so the user can see
//!   what the assistant did and undo it.
//!
//! The protocol is small enough to implement directly: newline-delimited JSON-RPC 2.0 with
//! `initialize`, `ping`, `tools/list` and `tools/call`. That keeps the dependencies to
//! `serde_json`, which the engine already uses.
//!
//! [`Project::sanitize`]: audiozero_engine::Project::sanitize

mod tools;

use audiozero_engine::{Changes, Command, Controller, Project, Session};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

/// Largest message accepted, in bytes. Bigger ones are refused without being parsed.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// Most notes one call can add or set.
pub const MAX_NOTES_PER_CALL: usize = 20_000;
/// Activity entries kept in memory (they're also written to the log as they happen).
const MAX_ACTIVITY: usize = 500;

/// Protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Where the server's edits are heard.
pub trait Audio {
    /// Plays `project` from the start, replacing whatever was playing. Returns warnings.
    fn load(&mut self, project: &Project) -> Result<Vec<String>, String>;
    /// Sends what an edit changed (see [`Controller::apply`]).
    fn apply(&mut self, project: &Project, changes: &Changes) -> Result<Vec<String>, String>;
    /// Returns `false` if the audio thread isn't responding.
    fn send(&mut self, command: Command) -> bool;
    fn position(&self) -> f64;
    fn is_playing(&self) -> bool;
    /// Called between messages, to free what the audio thread has finished with.
    fn collect_garbage(&mut self) {}
}

impl Audio for Controller {
    fn load(&mut self, project: &Project) -> Result<Vec<String>, String> {
        Controller::load(self, project.clone()).map_err(|e| e.to_string())
    }
    fn apply(&mut self, project: &Project, changes: &Changes) -> Result<Vec<String>, String> {
        Controller::apply(self, project, changes)
    }
    fn send(&mut self, command: Command) -> bool {
        Controller::send(self, command)
    }
    fn position(&self) -> f64 {
        Controller::position(self)
    }
    fn is_playing(&self) -> bool {
        Controller::is_playing(self)
    }
    fn collect_garbage(&mut self) {
        Controller::collect_garbage(self);
    }
}

/// Where `save` may write.
#[derive(Debug, Clone)]
pub struct SaveRules {
    /// The project file the server was started with, if any. `save` writes here.
    pub file: Option<PathBuf>,
    /// Folder for new projects when there's no file yet (normally `~/Music/AudioZero`).
    pub folder: PathBuf,
}

/// What a tool works on: a session, somewhere to hear it, and where it may save. The headless
/// [`Server`] owns these; the window lends its own, so an assistant edits what's on screen.
pub struct Ctx<'a> {
    pub session: &'a mut Session,
    pub audio: &'a mut dyn Audio,
    pub save: &'a mut SaveRules,
}

impl Ctx<'_> {
    /// Sends an edit's changes to the audio thread, returning any warnings to pass on.
    fn hear(&mut self, changes: &Changes) -> Vec<String> {
        if changes.is_empty() {
            return Vec::new();
        }
        match self.audio.apply(self.session.project(), changes) {
            Ok(warnings) => warnings,
            Err(e) => vec![e],
        }
    }
}

/// The MCP protocol and the activity log, independent of which session it edits.
pub struct Protocol {
    log: Box<dyn Write>,
    activity: VecDeque<String>,
    initialized: bool,
}

impl Protocol {
    /// `log` receives the activity log, one line per call; pass `std::io::stderr()`.
    pub fn new(log: Box<dyn Write>) -> Self {
        Self {
            log,
            activity: VecDeque::new(),
            initialized: false,
        }
    }

    /// What the assistant has done, oldest first.
    pub fn activity(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.activity.iter().map(String::as_str)
    }

    pub fn record(&mut self, entry: String) {
        let _ = writeln!(self.log, "[audiozero-mcp] {entry}");
        let _ = self.log.flush();
        if self.activity.len() == MAX_ACTIVITY {
            self.activity.pop_front();
        }
        self.activity.push_back(entry);
    }

    /// Handles one JSON-RPC message and returns the reply to send, if any (notifications and
    /// responses get none).
    pub fn handle(&mut self, ctx: &mut Ctx<'_>, message: &str) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(message) {
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
            Ok(Value::Object(request)) => self.dispatch(ctx, request),
            Ok(_) => Some(error(
                Value::Null,
                -32600,
                "expected one JSON-RPC request object (batches aren't supported)",
            )),
        };
        ctx.audio.collect_garbage();
        reply.map(|v| v.to_string())
    }

    fn dispatch(
        &mut self,
        ctx: &mut Ctx<'_>,
        request: serde_json::Map<String, Value>,
    ) -> Option<Value> {
        let id = request.get("id").cloned();
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            // A response to a request we never send, or junk without an id: nothing to answer.
            return id
                .filter(|_| !request.contains_key("result") && !request.contains_key("error"))
                .map(|id| error(id, -32600, "missing method"));
        };
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            // A notification, such as notifications/initialized or notifications/cancelled.
            return None;
        };
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => self.call(ctx, &params),
            other => Err((-32601, format!("unknown method {other}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error(id, code, &message),
        })
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = asked
            .filter(|v| PROTOCOL_VERSIONS.contains(v))
            .unwrap_or(PROTOCOL_VERSIONS[0]);
        if !self.initialized {
            self.initialized = true;
            let client = params
                .pointer("/clientInfo/name")
                .and_then(Value::as_str)
                .unwrap_or("unknown client");
            self.record(format!("connected: {client}"));
        }
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "audiozero",
                "title": "AudioZero",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": tools::INSTRUCTIONS,
        })
    }

    fn call(&mut self, ctx: &mut Ctx<'_>, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "tools/call needs a tool name".to_string()))?;
        if !tools::exists(name) {
            return Err((-32602, format!("unknown tool {name}")));
        }
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => Value::Object(Default::default()),
            Some(args) => args.clone(),
        };
        let outcome = tools::call(ctx, name, arguments);
        let (text, is_error) = match outcome {
            Ok(value) => {
                self.record(format!("{name}: {}", summary(&value)));
                (value.to_string(), false)
            }
            Err(message) => {
                self.record(format!("{name} refused: {message}"));
                (message, true)
            }
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }))
    }
}

/// The headless MCP server: its own [`Session`], plus somewhere to hear it.
pub struct Server<A: Audio> {
    session: Session,
    audio: A,
    save: SaveRules,
    protocol: Protocol,
}

impl<A: Audio> Server<A> {
    /// Starts serving `project` (sanitized first) and loads it into `audio`. `log` receives the
    /// activity log, one line per call; pass `std::io::stderr()`.
    pub fn new(
        project: Project,
        save: SaveRules,
        mut audio: A,
        log: Box<dyn Write>,
    ) -> Result<Self, String> {
        let session = Session::new(project).map_err(|e| e.to_string())?;
        let warnings = audio.load(session.project())?;
        let mut protocol = Protocol::new(log);
        for warning in warnings {
            protocol.record(format!("warning: {warning}"));
        }
        Ok(Self {
            session,
            audio,
            save,
            protocol,
        })
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn audio(&self) -> &A {
        &self.audio
    }

    /// What the assistant has done, oldest first.
    pub fn activity(&self) -> impl Iterator<Item = &str> {
        self.protocol.activity()
    }

    /// Handles one JSON-RPC message; see [`Protocol::handle`].
    pub fn handle(&mut self, message: &str) -> Option<String> {
        let mut ctx = Ctx {
            session: &mut self.session,
            audio: &mut self.audio,
            save: &mut self.save,
        };
        self.protocol.handle(&mut ctx, message)
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A short form of a tool's result for the activity log.
fn summary(value: &Value) -> String {
    let mut text = value.to_string();
    if let Some((cut, _)) = text.char_indices().nth(160) {
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// What reading one line gave.
pub(crate) enum Line {
    Message,
    TooLong,
    End,
}

/// Reads up to a newline into `buf`, refusing (and skipping) lines over `max` bytes so a
/// runaway client can't make the server buffer without limit.
pub(crate) fn read_line(
    input: &mut impl BufRead,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<Line> {
    buf.clear();
    let mut too_long = false;
    loop {
        let available = match input.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(if too_long {
                Line::TooLong
            } else if buf.is_empty() {
                Line::End
            } else {
                Line::Message
            });
        }
        let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (&available[..i], i + 1),
            None => (available, available.len()),
        };
        if !too_long {
            if buf.len() + chunk.len() > max {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(chunk);
            }
        }
        let found_newline = done > chunk.len();
        input.consume(done);
        if found_newline {
            return Ok(if too_long {
                Line::TooLong
            } else {
                Line::Message
            });
        }
    }
}

/// Serves MCP over a pair of streams (normally stdin and stdout) until the input ends, passing
/// each message to `handle` (normally [`Server::handle`]).
pub fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    mut handle: impl FnMut(&str) -> Option<String>,
) -> io::Result<()> {
    let mut buf = Vec::new();
    loop {
        let reply = match read_line(&mut input, &mut buf, MAX_MESSAGE_BYTES)? {
            Line::End => return Ok(()),
            Line::TooLong => Some(
                error(
                    Value::Null,
                    -32600,
                    &format!("message is over {MAX_MESSAGE_BYTES} bytes"),
                )
                .to_string(),
            ),
            Line::Message => match std::str::from_utf8(&buf) {
                Ok(text) if text.trim().is_empty() => None,
                Ok(text) => handle(text),
                Err(_) => Some(error(Value::Null, -32700, "message isn't UTF-8").to_string()),
            },
        };
        if let Some(reply) = reply {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests;
