//! The tools: their definitions (names, descriptions, JSON schemas) and what each one does.

use crate::{Ctx, MAX_NOTES_PER_CALL};
use audiozero_engine::project::limits::MAX_BEATS;
use audiozero_engine::session::mix_of;
use audiozero_engine::synth::BUILTINS;
use audiozero_engine::{Changes, Clip, ClipRef, Command, Device, Note, Track};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::Path;

pub const INSTRUCTIONS: &str = "\
AudioZero is a DAW. These tools edit the song that is open in this server, and it plays through \
the computer's speakers. Positions and lengths are in beats (quarter notes); a bar is \
`beats_per_bar` beats. Notes use MIDI pitch (60 = middle C) and start relative to their clip. \
Tracks, clips and notes are addressed by index; indices shift when something is removed, so call \
get_project (or get_clip) again after removing things. Every edit can be undone with undo. \
Effects (add_effect, set_effect_param) are undo steps. Mixer and transport settings (set_track_mix, set_transport) aren't undo steps. Audio clips (with \
an `audio` field) play a recording at its own speed; they have no notes, and you can move, \
trim, duplicate or remove them but not create them. Nothing is written \
to disk until you call save.";

const NOTE_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "pitch": { "type": "integer", "minimum": 0, "maximum": 127, "description": "MIDI note number; 60 is middle C" },
        "start": { "type": "number", "minimum": 0, "description": "Beats from the start of the clip; has to be inside the clip" },
        "length": { "type": "number", "exclusiveMinimum": 0, "description": "In beats" },
        "velocity": { "type": "number", "minimum": 0, "maximum": 1, "description": "0 to 1; defaults to 0.8" }
    },
    "required": ["pitch", "start", "length"],
    "additionalProperties": false
}"#;

struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    /// The JSON schema's `properties`, as JSON.
    properties: &'static str,
    required: &'static [&'static str],
    read_only: bool,
    destructive: bool,
}

macro_rules! track {
    () => {
        r#""track": { "type": "integer", "minimum": 0, "description": "Track index" }"#
    };
}

macro_rules! props {
    ($($part:expr),* $(,)?) => { concat!("{", $($part, ",",)* "\"_\": null}") };
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "get_project",
        title: "Get project",
        description: "The whole song: tempo, loop, transport, undo state, and every track with its instrument, mix and clips (clip notes are counted, not listed; use get_clip).",
        properties: "{}",
        required: &[],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "get_clip",
        title: "Get clip",
        description: "One clip with all its notes, each with its index.",
        properties: props!(
            r#""track": { "type": "integer", "minimum": 0 }"#,
            r#""clip": { "type": "integer", "minimum": 0, "description": "Clip index on the track" }"#
        ),
        required: &["track", "clip"],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "list_instruments",
        title: "List instruments",
        description: "The instruments a track can use, by id.",
        properties: "{}",
        required: &[],
        read_only: true,
        destructive: false,
    },
    Tool {
        name: "add_track",
        title: "Add track",
        description: "Adds a track at the end. Returns its index.",
        properties: props!(
            r#""name": { "type": "string" }"#,
            r#""instrument": { "type": "string", "description": "An id from list_instruments, or \"none\". Defaults to \"sine\"." }"#
        ),
        required: &[],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "remove_track",
        title: "Remove track",
        description: "Removes a track and its clips. Later tracks move down one index. Can be undone.",
        properties: props!(track!()),
        required: &["track"],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "rename_track",
        title: "Rename track",
        description: "Renames a track.",
        properties: props!(track!(), r#""name": { "type": "string" }"#),
        required: &["track", "name"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "set_instrument",
        title: "Set instrument",
        description: "Changes a track's instrument.",
        properties: props!(
            track!(),
            r#""instrument": { "type": "string", "description": "An id from list_instruments, or \"none\"" }"#
        ),
        required: &["track", "instrument"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "add_effect",
        title: "Add effect",
        description: "Adds a built-in effect (an id from list_instruments' effects) at the end of a track's effect chain. Returns its index.",
        properties: props!(
            track!(),
            r#""effect": { "type": "string", "description": "Effect id, e.g. \"reverb\"" }"#
        ),
        required: &["track", "effect"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "remove_effect",
        title: "Remove effect",
        description: "Removes an effect from a track by index. Later effects move down one. Can be undone.",
        properties: props!(
            track!(),
            r#""effect": { "type": "integer", "minimum": 0, "description": "Effect index on the track" }"#
        ),
        required: &["track", "effect"],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "set_effect_param",
        title: "Set effect parameter",
        description: "Sets one effect parameter (names and ranges are in list_instruments). An undo step; heard straight away.",
        properties: props!(
            track!(),
            r#""effect": { "type": "integer", "minimum": 0 }"#,
            r#""param": { "type": "string" }"#,
            r#""value": { "type": "number" }"#
        ),
        required: &["track", "effect", "param", "value"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "set_track_mix",
        title: "Set track mix",
        description: "Sets a track's volume, pan, mute or solo. Only the values given change. Not an undo step.",
        properties: props!(
            track!(),
            r#""volume": { "type": "number", "minimum": 0, "maximum": 2, "description": "1 is unity gain" }"#,
            r#""pan": { "type": "number", "minimum": -1, "maximum": 1 }"#,
            r#""mute": { "type": "boolean" }"#,
            r#""solo": { "type": "boolean" }"#
        ),
        required: &["track"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "add_clip",
        title: "Add clip",
        description: "Adds a MIDI clip to a track, optionally with notes, as one undo step. Returns the new clip's index.",
        properties: props!(
            track!(),
            r#""start": { "type": "number", "minimum": 0, "description": "Position on the timeline, in beats" }"#,
            r#""length": { "type": "number", "minimum": 0.0625, "description": "In beats" }"#,
            r#""name": { "type": "string" }"#,
            r#""notes": { "type": "array", "items": NOTE }"#
        ),
        required: &["track", "start", "length"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "update_clip",
        title: "Update clip",
        description: "Changes a clip's name, start, length or mute. Only the values given change. Shortening a clip drops notes that start after its new end.",
        properties: props!(
            track!(),
            r#""clip": { "type": "integer", "minimum": 0 }"#,
            r#""name": { "type": "string" }"#,
            r#""start": { "type": "number", "minimum": 0 }"#,
            r#""length": { "type": "number", "minimum": 0.0625 }"#,
            r#""muted": { "type": "boolean" }"#
        ),
        required: &["track", "clip"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "move_clip",
        title: "Move clip",
        description: "Moves a clip to another track (or along its own). Returns its new track and clip index.",
        properties: props!(
            track!(),
            r#""clip": { "type": "integer", "minimum": 0 }"#,
            r#""to_track": { "type": "integer", "minimum": 0 }"#,
            r#""start": { "type": "number", "minimum": 0, "description": "New position; defaults to where it is now" }"#
        ),
        required: &["track", "clip", "to_track"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "duplicate_clip",
        title: "Duplicate clip",
        description: "Puts a copy of a clip straight after it on the same track. Returns the copy's index.",
        properties: props!(track!(), r#""clip": { "type": "integer", "minimum": 0 }"#),
        required: &["track", "clip"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "remove_clip",
        title: "Remove clip",
        description: "Removes a clip. Later clips on the track move down one index. Can be undone.",
        properties: props!(track!(), r#""clip": { "type": "integer", "minimum": 0 }"#),
        required: &["track", "clip"],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "add_notes",
        title: "Add notes",
        description: "Adds notes to a clip, as one undo step. Each note has to start inside the clip.",
        properties: props!(
            track!(),
            r#""clip": { "type": "integer", "minimum": 0 }"#,
            r#""notes": { "type": "array", "items": NOTE, "minItems": 1 }"#
        ),
        required: &["track", "clip", "notes"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "set_notes",
        title: "Replace notes",
        description: "Replaces all of a clip's notes. An empty list clears the clip.",
        properties: props!(
            track!(),
            r#""clip": { "type": "integer", "minimum": 0 }"#,
            r#""notes": { "type": "array", "items": NOTE }"#
        ),
        required: &["track", "clip", "notes"],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "remove_notes",
        title: "Remove notes",
        description: "Removes notes from a clip by index (from get_clip).",
        properties: props!(
            track!(),
            r#""clip": { "type": "integer", "minimum": 0 }"#,
            r#""notes": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 1, "description": "Note indices" }"#
        ),
        required: &["track", "clip", "notes"],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "transpose_notes",
        title: "Transpose notes",
        description: "Moves notes up or down by semitones (clamped to 0-127). Without `notes`, moves every note in the clip.",
        properties: props!(
            track!(),
            r#""clip": { "type": "integer", "minimum": 0 }"#,
            r#""semitones": { "type": "integer", "minimum": -127, "maximum": 127 }"#,
            r#""notes": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "description": "Note indices" }"#
        ),
        required: &["track", "clip", "semitones"],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "set_transport",
        title: "Set tempo and loop",
        description: "Sets the tempo, loop or master volume. Only the values given change. Not an undo step.",
        properties: props!(
            r#""bpm": { "type": "number", "minimum": 20, "maximum": 999 }"#,
            r#""loop_enabled": { "type": "boolean" }"#,
            r#""loop_start": { "type": "number", "minimum": 0, "description": "In beats" }"#,
            r#""loop_end": { "type": "number", "minimum": 0, "description": "In beats" }"#,
            r#""master_volume": { "type": "number", "minimum": 0, "maximum": 1.5 }"#
        ),
        required: &[],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "undo",
        title: "Undo",
        description: "Undoes the last edit (yours or the user's).",
        properties: "{}",
        required: &[],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "redo",
        title: "Redo",
        description: "Redoes the last undone edit.",
        properties: "{}",
        required: &[],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "play",
        title: "Play",
        description: "Starts playback through the speakers.",
        properties: props!(
            r#""from": { "type": "number", "minimum": 0, "description": "Beat to start from; defaults to where the playhead is" }"#
        ),
        required: &[],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "stop",
        title: "Stop",
        description: "Stops playback.",
        properties: "{}",
        required: &[],
        read_only: false,
        destructive: false,
    },
    Tool {
        name: "export_video",
        title: "Export video",
        description: "Renders audio-reactive visuals for the song (or the loop) with its audio, as a new file `name`.mp4 (or .webm, or a folder of PNG frames) in the AudioZero folder. Never replaces a file. Takes a while: about as long as the song.",
        properties: props!(
            r#""name": { "type": "string", "description": "File name without extension (letters, digits, spaces, - and _)" }"#,
            r#""look": { "type": "string", "enum": ["spectrum", "rings", "particles"], "description": "Defaults to spectrum" }"#,
            r#""format": { "type": "string", "enum": ["widescreen", "shorts", "square", "portrait", "4k"], "description": "Defaults to widescreen (1920x1080); shorts is 9:16" }"#,
            r#""fps": { "type": "integer", "enum": [30, 60] }"#,
            r#""loop": { "type": "boolean", "description": "Only the loop range" }"#,
            r#""container": { "type": "string", "enum": ["mp4", "webm", "frames"], "description": "Defaults to mp4 (needs FFmpeg)" }"#
        ),
        required: &["name"],
        read_only: false,
        destructive: true,
    },
    Tool {
        name: "save",
        title: "Save",
        description: "Saves the song: to the project file this server was started with, or, if there isn't one, to a new file `name`.json in the AudioZero folder (never replacing an existing file).",
        properties: props!(
            r#""name": { "type": "string", "description": "File name without extension (letters, digits, spaces, - and _). Only used when there's no project file yet." }"#
        ),
        required: &[],
        read_only: false,
        destructive: true,
    },
];

pub fn exists(name: &str) -> bool {
    TOOLS.iter().any(|t| t.name == name)
}

pub fn definitions() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|t| {
            let mut properties: Value =
                serde_json::from_str(&t.properties.replace("NOTE", NOTE_SCHEMA))
                    .unwrap_or_else(|e| panic!("schema for {}: {e}", t.name));
            if let Value::Object(map) = &mut properties {
                map.remove("_");
            }
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": t.required,
                    "additionalProperties": false,
                },
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": t.read_only,
                    "destructiveHint": t.destructive,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

// ---- arguments ------------------------------------------------------------------------------

fn parse<T: DeserializeOwned>(arguments: Value) -> Result<T, String> {
    serde_json::from_value(arguments).map_err(|e| format!("bad arguments: {e}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackArg {
    track: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipArg {
    track: usize,
    clip: usize,
}

impl ClipArg {
    fn at(&self) -> ClipRef {
        ClipRef {
            track: self.track,
            clip: self.clip,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteArg {
    pitch: u8,
    start: f64,
    length: f64,
    #[serde(default = "default_velocity")]
    velocity: f32,
}

fn default_velocity() -> f32 {
    0.8
}

fn notes(args: Vec<NoteArg>) -> Result<Vec<Note>, String> {
    if args.len() > MAX_NOTES_PER_CALL {
        return Err(format!(
            "{} notes in one call; the limit is {MAX_NOTES_PER_CALL}",
            args.len()
        ));
    }
    Ok(args
        .into_iter()
        .map(|n| Note {
            pitch: n.pitch,
            start: n.start,
            length: n.length,
            velocity: n.velocity,
        })
        .collect())
}

fn instrument(id: &str) -> Result<Option<Device>, String> {
    if id == "none" {
        return Ok(None);
    }
    if BUILTINS.iter().any(|(builtin, _)| *builtin == id) {
        Ok(Some(Device::builtin(id)))
    } else {
        let ids: Vec<_> = BUILTINS.iter().map(|(id, _)| *id).collect();
        Err(format!(
            "unknown instrument \"{id}\"; use one of {} or \"none\"",
            ids.join(", ")
        ))
    }
}

fn describe(device: Option<&Device>) -> Value {
    match device {
        None => json!("none"),
        Some(Device::Builtin { id, .. }) => json!(id),
        Some(Device::Plugin(p)) => json!({ "plugin": p.name, "vendor": p.vendor }),
    }
}

fn check_beat(what: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && (0.0..=MAX_BEATS).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what} has to be between 0 and {MAX_BEATS} beats"))
    }
}

// ---- dispatch -------------------------------------------------------------------------------

/// Runs a tool. `Ok` is the result to show the client; `Err` is a message for it, reported as a
/// tool error so the model can correct itself.
pub fn call(s: &mut Ctx<'_>, name: &str, arguments: Value) -> Result<Value, String> {
    match name {
        "get_project" => {
            parse::<NoArgs>(arguments)?;
            Ok(project_summary(s))
        }
        "get_clip" => {
            let a: ClipArg = parse(arguments)?;
            let clip = s
                .session
                .clip(a.at())
                .ok_or_else(|| audiozero_engine::EditError::NoClip(a.at()).to_string())?;
            let notes: Vec<Value> = clip
                .notes
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    json!({ "note": i, "pitch": n.pitch, "start": n.start, "length": n.length, "velocity": f32_out(n.velocity) })
                })
                .collect();
            Ok(json!({
                "track": a.track, "clip": a.clip, "name": clip.name, "start": clip.start,
                "length": clip.length, "muted": clip.muted, "notes": notes,
                "audio": audio_of(clip),
            }))
        }
        "list_instruments" => {
            parse::<NoArgs>(arguments)?;
            let list: Vec<Value> = BUILTINS
                .iter()
                .map(|(id, name)| json!({ "id": id, "name": name }))
                .collect();
            let effects: Vec<Value> = audiozero_engine::effects::EFFECTS
                .iter()
                .map(|(id, name)| {
                    let params: Vec<Value> = audiozero_engine::effects::params_for(id)
                        .unwrap_or(&[])
                        .iter()
                        .map(|p| json!({ "name": p.name, "min": p.min, "max": p.max, "default": p.default, "unit": p.unit }))
                        .collect();
                    json!({ "id": id, "name": name, "params": params })
                })
                .collect();
            Ok(
                json!({ "instruments": list, "effects": effects, "note": "Third-party plugins aren't available to the assistant yet." }),
            )
        }
        "add_track" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                name: Option<String>,
                instrument: Option<String>,
            }
            let a: Args = parse(arguments)?;
            let instrument = instrument(a.instrument.as_deref().unwrap_or("sine"))?;
            let number = s.session.project().tracks.len() + 1;
            let track = Track {
                name: a.name.unwrap_or_else(|| format!("Track {number}")),
                instrument,
                ..Default::default()
            };
            let (index, changes) = s.session.add_track(track).map_err(|e| e.to_string())?;
            Ok(done(s, &changes, json!({ "track": index })))
        }
        "remove_track" => {
            let a: TrackArg = parse(arguments)?;
            let changes = s.session.remove_track(a.track).map_err(|e| e.to_string())?;
            Ok(done(s, &changes, json!({ "removed_track": a.track })))
        }
        "rename_track" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                name: String,
            }
            let a: Args = parse(arguments)?;
            let changes = s
                .session
                .rename_track(a.track, &a.name)
                .map_err(|e| e.to_string())?;
            let name = s.session.project().tracks[a.track].name.clone();
            Ok(done(s, &changes, json!({ "track": a.track, "name": name })))
        }
        "set_instrument" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                instrument: String,
            }
            let a: Args = parse(arguments)?;
            let device = instrument(&a.instrument)?;
            let changes = s
                .session
                .set_instrument(a.track, device)
                .map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": a.track, "instrument": a.instrument }),
            ))
        }
        "add_effect" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                effect: String,
            }
            let a: Args = parse(arguments)?;
            let (index, changes) = s
                .session
                .add_effect(a.track, &a.effect)
                .map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": a.track, "effect": index, "id": a.effect }),
            ))
        }
        "remove_effect" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                effect: usize,
            }
            let a: Args = parse(arguments)?;
            let changes = s
                .session
                .remove_effect(a.track, a.effect)
                .map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": a.track, "removed_effect": a.effect }),
            ))
        }
        "set_effect_param" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                effect: usize,
                param: String,
                value: f32,
            }
            let a: Args = parse(arguments)?;
            let changes = s
                .session
                .set_effect_param(a.track, a.effect, &a.param, a.value)
                .map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": a.track, "effect": a.effect, "param": a.param, "value": f32_out(a.value) }),
            ))
        }
        "set_track_mix" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                volume: Option<f32>,
                pan: Option<f32>,
                mute: Option<bool>,
                solo: Option<bool>,
            }
            let a: Args = parse(arguments)?;
            let track = s
                .session
                .track(a.track)
                .ok_or_else(|| audiozero_engine::EditError::NoTrack(a.track).to_string())?;
            let mut mix = mix_of(track);
            if let Some(v) = a.volume {
                range("volume", v, 0.0, 2.0)?;
                mix.volume = v;
            }
            if let Some(p) = a.pan {
                range("pan", p, -1.0, 1.0)?;
                mix.pan = p;
            }
            mix.mute = a.mute.unwrap_or(mix.mute);
            mix.solo = a.solo.unwrap_or(mix.solo);
            let changes = s
                .session
                .set_track_mix(a.track, mix)
                .map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": a.track, "volume": f32_out(mix.volume), "pan": f32_out(mix.pan), "mute": mix.mute, "solo": mix.solo }),
            ))
        }
        "add_clip" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                start: f64,
                length: f64,
                name: Option<String>,
                #[serde(default)]
                notes: Vec<NoteArg>,
            }
            let a: Args = parse(arguments)?;
            let clip = Clip {
                name: a.name.unwrap_or_else(|| "Clip".into()),
                start: a.start,
                length: a.length,
                muted: false,
                notes: notes(a.notes)?,
                audio: None,
            };
            let count = clip.notes.len();
            let (at, changes) = s
                .session
                .add_clip(a.track, clip)
                .map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": at.track, "clip": at.clip, "notes": count }),
            ))
        }
        "update_clip" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                clip: usize,
                name: Option<String>,
                start: Option<f64>,
                length: Option<f64>,
                muted: Option<bool>,
            }
            let a: Args = parse(arguments)?;
            let at = ClipRef {
                track: a.track,
                clip: a.clip,
            };
            let changes = s
                .session
                .update_clip(at, |c| {
                    if let Some(name) = a.name {
                        c.name = name;
                    }
                    c.start = a.start.unwrap_or(c.start);
                    c.length = a.length.unwrap_or(c.length);
                    c.muted = a.muted.unwrap_or(c.muted);
                })
                .map_err(|e| e.to_string())?;
            Ok(done(s, &changes, clip_summary(s, at)))
        }
        "move_clip" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                clip: usize,
                to_track: usize,
                start: Option<f64>,
            }
            let a: Args = parse(arguments)?;
            let at = ClipRef {
                track: a.track,
                clip: a.clip,
            };
            let start = match a.start {
                Some(start) => start,
                None => {
                    s.session
                        .clip(at)
                        .ok_or_else(|| audiozero_engine::EditError::NoClip(at).to_string())?
                        .start
                }
            };
            let (moved, changes) = s
                .session
                .move_clip(at, a.to_track, start)
                .map_err(|e| e.to_string())?;
            Ok(done(s, &changes, clip_summary(s, moved)))
        }
        "duplicate_clip" => {
            let a: ClipArg = parse(arguments)?;
            let (copy, changes) = s
                .session
                .duplicate_clip(a.at())
                .map_err(|e| e.to_string())?;
            Ok(done(s, &changes, clip_summary(s, copy)))
        }
        "remove_clip" => {
            let a: ClipArg = parse(arguments)?;
            let changes = s.session.remove_clip(a.at()).map_err(|e| e.to_string())?;
            Ok(done(
                s,
                &changes,
                json!({ "track": a.track, "removed_clip": a.clip }),
            ))
        }
        "add_notes" | "set_notes" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                clip: usize,
                notes: Vec<NoteArg>,
            }
            let a: Args = parse(arguments)?;
            let at = ClipRef {
                track: a.track,
                clip: a.clip,
            };
            let notes = notes(a.notes)?;
            let changes = if name == "add_notes" {
                s.session.add_notes(at, &notes).map(|(_, c)| c)
            } else {
                s.session.set_notes(at, notes)
            }
            .map_err(|e| e.to_string())?;
            Ok(done(s, &changes, clip_summary(s, at)))
        }
        "remove_notes" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                clip: usize,
                notes: Vec<usize>,
            }
            let a: Args = parse(arguments)?;
            let at = ClipRef {
                track: a.track,
                clip: a.clip,
            };
            let changes = s
                .session
                .remove_notes(at, &a.notes)
                .map_err(|e| e.to_string())?;
            Ok(done(s, &changes, clip_summary(s, at)))
        }
        "transpose_notes" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                track: usize,
                clip: usize,
                semitones: i32,
                notes: Option<Vec<usize>>,
            }
            let a: Args = parse(arguments)?;
            if !(-127..=127).contains(&a.semitones) {
                return Err("semitones has to be between -127 and 127".into());
            }
            let at = ClipRef {
                track: a.track,
                clip: a.clip,
            };
            let changes = s
                .session
                .transpose_notes(at, a.notes.as_deref(), a.semitones)
                .map_err(|e| e.to_string())?;
            Ok(done(s, &changes, clip_summary(s, at)))
        }
        "set_transport" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                bpm: Option<f64>,
                loop_enabled: Option<bool>,
                loop_start: Option<f64>,
                loop_end: Option<f64>,
                master_volume: Option<f32>,
            }
            let a: Args = parse(arguments)?;
            // Check everything before changing anything, so a bad value changes nothing.
            if let Some(bpm) = a.bpm {
                range("bpm", bpm, 20.0, 999.0)?;
            }
            if let Some(v) = a.master_volume {
                range("master_volume", v, 0.0, 1.5)?;
            }
            let p = s.session.project();
            let looping = (
                a.loop_enabled.unwrap_or(p.loop_enabled),
                a.loop_start.unwrap_or(p.loop_start),
                a.loop_end.unwrap_or(p.loop_end),
            );
            check_beat("loop_start", looping.1)?;
            check_beat("loop_end", looping.2)?;
            if looping.2 < looping.1 + 1.0 / 16.0 {
                return Err("loop_end has to be after loop_start".into());
            }
            let mut warnings = Vec::new();
            let err = |e: audiozero_engine::EditError| e.to_string();
            let c = s
                .session
                .set_loop(looping.0, looping.1, looping.2)
                .map_err(err)?;
            warnings.extend(s.hear(&c));
            if let Some(bpm) = a.bpm {
                let c = s.session.set_bpm(bpm).map_err(err)?;
                warnings.extend(s.hear(&c));
            }
            if let Some(v) = a.master_volume {
                let c = s.session.set_master_volume(v).map_err(err)?;
                warnings.extend(s.hear(&c));
            }
            let p = s.session.project();
            let mut result = json!({
                "bpm": p.bpm, "master_volume": f32_out(p.master_volume),
                "loop": { "enabled": p.loop_enabled, "start": p.loop_start, "end": p.loop_end },
            });
            if !warnings.is_empty() {
                result["warnings"] = json!(warnings);
            }
            Ok(result)
        }
        "undo" | "redo" => {
            parse::<NoArgs>(arguments)?;
            let changes = if name == "undo" {
                s.session.undo()
            } else {
                s.session.redo()
            }
            .ok_or_else(|| format!("nothing to {name}"))?;
            let state =
                json!({ "can_undo": s.session.can_undo(), "can_redo": s.session.can_redo() });
            Ok(done(s, &changes, state))
        }
        "play" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                from: Option<f64>,
            }
            let a: Args = parse(arguments)?;
            let mut ok = true;
            if let Some(from) = a.from {
                check_beat("from", from)?;
                ok &= s.audio.send(Command::Seek(from));
            }
            ok &= s.audio.send(Command::Play);
            if !ok {
                return Err("the audio engine isn't responding".into());
            }
            let from = a.from.unwrap_or_else(|| s.audio.position());
            Ok(
                json!({ "playing": true, "from": from, "song_ends_at": s.session.project().end_beats() }),
            )
        }
        "stop" => {
            parse::<NoArgs>(arguments)?;
            if !s.audio.send(Command::Stop) {
                return Err("the audio engine isn't responding".into());
            }
            Ok(json!({ "playing": false, "position": s.audio.position() }))
        }
        "export_video" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                name: String,
                look: Option<String>,
                format: Option<String>,
                fps: Option<u32>,
                #[serde(rename = "loop", default)]
                loop_only: bool,
                container: Option<String>,
            }
            let a: Args = parse(arguments)?;
            let mut settings = audiozero_visuals::VideoSettings {
                look: a.look.unwrap_or_else(|| "spectrum".into()),
                fps: a.fps.unwrap_or(30),
                range: if a.loop_only {
                    audiozero_visuals::Range::Loop
                } else {
                    audiozero_visuals::Range::Song
                },
                ..Default::default()
            };
            if !matches!(settings.fps, 30 | 60) {
                return Err("fps has to be 30 or 60".into());
            }
            settings.set_format(a.format.as_deref().unwrap_or("widescreen"))?;
            if settings.width > 3840 || settings.height > 3840 {
                return Err("that format is too large".into());
            }
            let name = plain_name(&a.name)?;
            let target = match a.container.as_deref().unwrap_or("mp4") {
                "mp4" => format!("{name}.mp4"),
                "webm" => format!("{name}.webm"),
                "frames" => name,
                other => {
                    return Err(format!(
                        "unknown container \"{other}\"; use mp4, webm or frames"
                    ));
                }
            };
            let path = s.save.folder.join(&target);
            if path.exists() {
                return Err(format!("{target} already exists; choose another name"));
            }
            std::fs::create_dir_all(&s.save.folder)
                .map_err(|e| format!("can't create the AudioZero folder: {e}"))?;
            let mut pool =
                audiozero_engine::sample::SamplePool::new(audiozero_visuals::analysis::RATE);
            pool.set_base(
                s.save
                    .file
                    .as_deref()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf),
            );
            let done = audiozero_visuals::export(
                s.session.project(),
                &mut pool,
                &settings,
                &path,
                |_, _| true,
            )?;
            Ok(json!({
                "exported": target,
                "frames": done.frames,
                "seconds": done.seconds,
                "width": settings.width,
                "height": settings.height,
                "look": settings.look,
            }))
        }
        "save" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                name: Option<String>,
            }
            let a: Args = parse(arguments)?;
            save(s, a.name.as_deref())
        }
        other => Err(format!("unknown tool {other}")),
    }
}

/// An `f32` for output, without the noise widening to `f64` adds (0.6 rather than
/// 0.6000000238418579).
fn f32_out(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

fn range<T: Into<f64> + Copy>(what: &str, value: T, lo: f64, hi: f64) -> Result<(), String> {
    let v: f64 = value.into();
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(format!("{what} has to be between {lo} and {hi}"))
    }
}

/// Sends an edit to the audio thread and adds any warnings to its result.
fn done(s: &mut Ctx<'_>, changes: &Changes, mut result: Value) -> Value {
    let warnings = s.hear(changes);
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    result
}

/// A track's effects: index, id and every parameter's current value.
fn effects_of(track: &Track) -> Value {
    track
        .effects
        .iter()
        .enumerate()
        .map(|(i, d)| match d {
            Device::Builtin { id, .. } => {
                let params: serde_json::Map<String, Value> =
                    audiozero_engine::effects::params_for(id)
                        .unwrap_or(&[])
                        .iter()
                        .map(|p| {
                            (
                                p.name.to_string(),
                                json!(f32_out(d.param(p.name).unwrap_or(p.default))),
                            )
                        })
                        .collect();
                json!({ "effect": i, "id": id, "params": params })
            }
            Device::Plugin(p) => json!({ "effect": i, "plugin": p.name }),
        })
        .collect::<Vec<_>>()
        .into()
}

/// An audio clip's file (by name only, not its full path), offset and gain; null for MIDI clips.
fn audio_of(clip: &Clip) -> Value {
    clip.audio.as_ref().map_or(Value::Null, |a| {
        let name = Path::new(&a.file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        json!({ "file": name, "offset_seconds": a.offset, "gain": f32_out(a.gain) })
    })
}

fn clip_summary(s: &Ctx<'_>, at: ClipRef) -> Value {
    match s.session.clip(at) {
        Some(c) => json!({
            "track": at.track, "clip": at.clip, "name": c.name, "start": c.start,
            "length": c.length, "muted": c.muted, "notes": c.notes.len(),
            "audio": audio_of(c),
        }),
        None => json!({ "track": at.track, "clip": at.clip }),
    }
}

fn project_summary(s: &Ctx<'_>) -> Value {
    let p = s.session.project();
    let tracks: Vec<Value> = p
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let clips: Vec<Value> = t
                .clips
                .iter()
                .enumerate()
                .map(|(j, c)| {
                    json!({ "clip": j, "name": c.name, "start": c.start, "length": c.length, "muted": c.muted, "notes": c.notes.len(), "audio": audio_of(c) })
                })
                .collect();
            json!({
                "track": i, "name": t.name, "instrument": describe(t.instrument.as_ref()),
                "effects": effects_of(t), "volume": f32_out(t.volume), "pan": f32_out(t.pan), "mute": t.mute, "solo": t.solo, "clips": clips,
            })
        })
        .collect();
    let file = s
        .save
        .file
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    json!({
        "file": file,
        "unsaved_changes": s.session.is_dirty(),
        "bpm": p.bpm,
        "beats_per_bar": p.beats_per_bar,
        "loop": { "enabled": p.loop_enabled, "start": p.loop_start, "end": p.loop_end },
        "master_volume": f32_out(p.master_volume),
        "playing": s.audio.is_playing(),
        "position": s.audio.position(),
        "song_ends_at": p.end_beats(),
        "can_undo": s.session.can_undo(),
        "can_redo": s.session.can_redo(),
        "tracks": tracks,
    })
}

/// A file name the assistant may create: plain characters only, so it can't name another folder
/// or a hidden file.
fn file_name(name: &str) -> Result<String, String> {
    plain_name(name).map(|n| format!("{n}.json"))
}

fn plain_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.chars().count() <= 64
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'));
    if ok {
        Ok(name.to_owned())
    } else {
        Err("name has to be 1 to 64 letters, digits, spaces, - or _".into())
    }
}

fn save(s: &mut Ctx<'_>, name: Option<&str>) -> Result<Value, String> {
    let path = match (&s.save.file, name) {
        (Some(file), None) => file.clone(),
        (Some(_), Some(_)) => {
            return Err(
                "this song already has a file; call save without a name to save it there".into(),
            );
        }
        (None, None) => {
            return Err("this song has no file yet; give it a name".into());
        }
        (None, Some(name)) => {
            let path = s.save.folder.join(file_name(name)?);
            if path.exists() {
                return Err(format!(
                    "{} already exists; choose another name",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            std::fs::create_dir_all(&s.save.folder)
                .map_err(|e| format!("can't create the AudioZero folder: {e}"))?;
            path
        }
    };
    s.session
        .project()
        .save(&path)
        .map_err(|e| format!("can't save: {e}"))?;
    s.session.mark_saved();
    s.save.file = Some(path.clone());
    Ok(json!({ "saved": path.file_name().map(|n| n.to_string_lossy().into_owned()) }))
}
