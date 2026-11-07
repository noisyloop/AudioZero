//! The project model and its file format.
//!
//! Projects are JSON. Every project goes through [`Project::sanitize`] before the engine sees
//! it, whether it came from a file, the UI or (later) an MCP client. That one function holds
//! every limit and range, so the checks can't drift apart between entry points.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Written in every project file, so other JSON is rejected early.
pub const FORMAT_NAME: &str = "audiozero-project";
/// Bump when the format changes in a way older builds can't read.
pub const FORMAT_VERSION: u32 = 1;

/// Hard limits. A project over any of these fails to load with a clear message instead of
/// exhausting memory or stalling the audio thread.
pub mod limits {
    pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
    pub const MAX_TRACKS: usize = 256;
    pub const MAX_CLIPS_PER_TRACK: usize = 4096;
    pub const MAX_NOTES_TOTAL: usize = 1_000_000;
    pub const MAX_EFFECTS_PER_TRACK: usize = 32;
    /// Furthest position anything can sit at, in beats (about 14 hours at 120 BPM).
    pub const MAX_BEATS: f64 = 100_000.0;
    pub const MAX_NAME_CHARS: usize = 256;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    /// Must be [`FORMAT_NAME`]. A file without it is rejected, not treated as an empty project.
    #[serde(default = "missing_format")]
    pub format: String,
    pub version: u32,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub loop_enabled: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub master_volume: f32,
    pub tracks: Vec<Track>,
}

fn missing_format() -> String {
    String::new()
}

impl Default for Project {
    fn default() -> Self {
        Self {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION,
            bpm: 120.0,
            beats_per_bar: 4,
            loop_enabled: false,
            loop_start: 0.0,
            loop_end: 16.0,
            master_volume: 0.85,
            tracks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub instrument: Option<Device>,
    pub effects: Vec<Device>,
    pub clips: Vec<Clip>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            name: "Track".into(),
            volume: 0.8,
            pan: 0.0,
            mute: false,
            solo: false,
            instrument: None,
            effects: Vec::new(),
            clips: Vec::new(),
        }
    }
}

/// An instrument or effect on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Device {
    /// A device built into AudioZero, such as `"sine"`.
    Builtin { id: String },
    /// A third-party plugin, named by identity only.
    Plugin(PluginRef),
}

/// Names a plugin by its identity. There is deliberately no file path: the host resolves the
/// identity against plugins the user scanned or added, so opening a project can never make
/// AudioZero load a binary the project chose (security review, finding 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginRef {
    pub format: PluginFormat,
    /// The CLAP plugin id, or the VST3 class id as hex.
    pub id: String,
    /// Shown when the plugin is missing.
    pub name: String,
    pub vendor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginFormat {
    Clap,
    Vst3,
}

/// A MIDI clip. Audio clips come later, with a bounded decoder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clip {
    pub name: String,
    /// Position on the timeline, in beats.
    pub start: f64,
    pub length: f64,
    pub muted: bool,
    pub notes: Vec<Note>,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: "Clip".into(),
            start: 0.0,
            length: 4.0,
            muted: false,
            notes: Vec::new(),
        }
    }
}

impl Clip {
    pub fn end(&self) -> f64 {
        self.start + self.length
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    /// Relative to the clip start, in beats.
    pub start: f64,
    pub length: f64,
    pub velocity: f32,
}

#[derive(Debug)]
pub enum ProjectError {
    Io(std::io::Error),
    TooLarge(u64),
    Parse(serde_json::Error),
    WrongFormat,
    NewerVersion(u32),
    Limit(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::TooLarge(n) => write!(
                f,
                "project file is {n} bytes; the limit is {}",
                limits::MAX_FILE_BYTES
            ),
            Self::Parse(e) => write!(f, "not a valid project file: {e}"),
            Self::WrongFormat => write!(f, "not an AudioZero project"),
            Self::NewerVersion(v) => write!(
                f,
                "project format version {v} is newer than this build supports ({FORMAT_VERSION}); update AudioZero"
            ),
            Self::Limit(what) => write!(f, "project is too large: {what}"),
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<std::io::Error> for ProjectError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Returns `value` clamped to `[lo, hi]`, or `default` if it isn't finite.
/// (`f64::clamp` would pass NaN straight through, which is how `bpm="nan"` got into the C++ engine.)
fn finite(value: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if value.is_finite() {
        value.clamp(lo, hi)
    } else {
        default
    }
}

fn finite32(value: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(lo, hi)
    } else {
        default
    }
}

fn truncate_name(name: &mut String) {
    if let Some((cut, _)) = name.char_indices().nth(limits::MAX_NAME_CHARS) {
        name.truncate(cut);
    }
}

impl Project {
    pub fn from_json(text: &str) -> Result<Self, ProjectError> {
        if text.len() as u64 > limits::MAX_FILE_BYTES {
            return Err(ProjectError::TooLarge(text.len() as u64));
        }
        let mut project: Project = serde_json::from_str(text).map_err(ProjectError::Parse)?;
        if project.format != FORMAT_NAME {
            return Err(ProjectError::WrongFormat);
        }
        if project.version > FORMAT_VERSION {
            return Err(ProjectError::NewerVersion(project.version));
        }
        project.sanitize()?;
        Ok(project)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a project always serializes")
    }

    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        let size = std::fs::metadata(path)?.len();
        if size > limits::MAX_FILE_BYTES {
            return Err(ProjectError::TooLarge(size));
        }
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// Writes to a temporary file next to `path` and renames it, so a crash mid-save never
    /// leaves a half-written project.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Enforces the limits and clamps every value into range. Fails only when a limit is
    /// exceeded; out-of-range values are fixed rather than rejected.
    pub fn sanitize(&mut self) -> Result<(), ProjectError> {
        use limits::*;
        if self.tracks.len() > MAX_TRACKS {
            return Err(ProjectError::Limit(format!(
                "{} tracks (limit {MAX_TRACKS})",
                self.tracks.len()
            )));
        }
        let mut notes = 0usize;
        for track in &self.tracks {
            if track.clips.len() > MAX_CLIPS_PER_TRACK {
                return Err(ProjectError::Limit(format!(
                    "{} clips on track \"{}\" (limit {MAX_CLIPS_PER_TRACK})",
                    track.clips.len(),
                    track.name
                )));
            }
            if track.effects.len() > MAX_EFFECTS_PER_TRACK {
                return Err(ProjectError::Limit(format!(
                    "{} effects on track \"{}\" (limit {MAX_EFFECTS_PER_TRACK})",
                    track.effects.len(),
                    track.name
                )));
            }
            notes += track.clips.iter().map(|c| c.notes.len()).sum::<usize>();
        }
        if notes > MAX_NOTES_TOTAL {
            return Err(ProjectError::Limit(format!(
                "{notes} notes (limit {MAX_NOTES_TOTAL})"
            )));
        }

        self.format = FORMAT_NAME.into();
        self.version = FORMAT_VERSION;
        self.bpm = finite(self.bpm, 20.0, 999.0, 120.0);
        self.beats_per_bar = self.beats_per_bar.clamp(1, 16);
        self.loop_start = finite(self.loop_start, 0.0, MAX_BEATS, 0.0);
        self.loop_end = finite(self.loop_end, 0.0, MAX_BEATS, 16.0);
        if self.loop_end < self.loop_start + 1.0 / 16.0 {
            self.loop_enabled = false;
            self.loop_end = self.loop_start + 4.0;
        }
        self.master_volume = finite32(self.master_volume, 0.0, 1.5, 0.85);

        for track in &mut self.tracks {
            truncate_name(&mut track.name);
            track.volume = finite32(track.volume, 0.0, 2.0, 0.8);
            track.pan = finite32(track.pan, -1.0, 1.0, 0.0);
            for clip in &mut track.clips {
                truncate_name(&mut clip.name);
                clip.start = finite(clip.start, 0.0, MAX_BEATS, 0.0);
                let room = (MAX_BEATS - clip.start).max(1.0 / 16.0);
                clip.length = finite(clip.length, 1.0 / 16.0, room, 4.0_f64.min(room));
                let clip_length = clip.length;
                clip.notes
                    .retain(|n| n.start.is_finite() && n.start >= 0.0 && n.start < clip_length);
                for note in &mut clip.notes {
                    note.pitch = note.pitch.min(127);
                    note.length = finite(note.length, 1.0 / 128.0, MAX_BEATS, 1.0);
                    note.velocity = finite32(note.velocity, 0.0, 1.0, 0.8);
                }
            }
        }
        Ok(())
    }

    /// Where the last clip ends, in beats.
    pub fn end_beats(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(Clip::end)
            .fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(extra: &str) -> String {
        format!(r#"{{"format":"audiozero-project","version":1{extra}}}"#)
    }

    #[test]
    fn round_trips() {
        let project = crate::demo::project();
        let back = Project::from_json(&project.to_json()).unwrap();
        assert_eq!(project, back);
    }

    #[test]
    fn rejects_other_json_and_newer_versions() {
        assert!(matches!(
            Project::from_json("{}"),
            Err(ProjectError::WrongFormat)
        ));
        assert!(matches!(
            Project::from_json(r#"{"format":"audiozero-project","version":99}"#),
            Err(ProjectError::NewerVersion(99))
        ));
        assert!(matches!(
            Project::from_json("not json"),
            Err(ProjectError::Parse(_))
        ));
    }

    #[test]
    fn json_cannot_smuggle_nan_or_infinity() {
        // JSON has no NaN; serde_json rejects the bare word and out-of-range numbers.
        assert!(Project::from_json(&minimal(r#","bpm":NaN"#)).is_err());
        assert!(Project::from_json(&minimal(r#","bpm":1e999"#)).is_err());
    }

    #[test]
    fn sanitize_clamps_values_set_in_code() {
        let mut p = Project {
            bpm: f64::NAN,
            master_volume: f32::INFINITY,
            ..Default::default()
        };
        p.tracks.push(Track {
            volume: 1e6,
            pan: -7.0,
            clips: vec![Clip {
                start: -5.0,
                length: f64::INFINITY,
                notes: vec![
                    Note {
                        pitch: 200,
                        start: 0.0,
                        length: f64::NAN,
                        velocity: 9.0,
                    },
                    Note {
                        pitch: 60,
                        start: -1.0,
                        length: 1.0,
                        velocity: 0.5,
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        });
        p.sanitize().unwrap();
        assert_eq!(p.bpm, 120.0);
        assert_eq!(p.master_volume, 0.85);
        let t = &p.tracks[0];
        assert_eq!((t.volume, t.pan), (2.0, -1.0));
        let c = &t.clips[0];
        assert_eq!((c.start, c.length), (0.0, 4.0));
        assert_eq!(
            c.notes.len(),
            1,
            "the note before the clip start is dropped"
        );
        assert_eq!(
            c.notes[0],
            Note {
                pitch: 127,
                start: 0.0,
                length: 1.0,
                velocity: 1.0
            }
        );
    }

    #[test]
    fn enforces_count_limits() {
        let mut p = Project {
            tracks: vec![Track::default(); limits::MAX_TRACKS + 1],
            ..Default::default()
        };
        assert!(matches!(p.sanitize(), Err(ProjectError::Limit(_))));
    }

    #[test]
    fn plugins_are_stored_by_identity_without_a_path() {
        let json = minimal(
            r#","tracks":[{"instrument":{"type":"plugin","format":"clap","id":"com.example.synth","name":"Synth","vendor":"Example","path":"/tmp/evil.clap"}}]"#,
        );
        let p = Project::from_json(&json).unwrap();
        let Some(Device::Plugin(r)) = &p.tracks[0].instrument else {
            panic!("expected a plugin")
        };
        assert_eq!(r.id, "com.example.synth");
        assert!(
            !p.to_json().contains("evil"),
            "unknown fields such as a path are dropped"
        );
    }

    #[test]
    fn save_and_load_through_a_file() {
        let dir = std::env::temp_dir().join(format!("audiozero-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.azproj.json");
        let project = crate::demo::project();
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
