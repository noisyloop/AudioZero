//! The project model and its file format.
//!
//! Projects are JSON. Every project goes through [`Project::sanitize`] before the engine sees
//! it, whether it came from a file, the UI or (later) an MCP client. That one function holds
//! every limit and range, so the checks can't drift apart between entry points.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Written in every project file, so other JSON is rejected early.
pub const FORMAT_NAME: &str = "audiozero-project";
/// Bump when the format changes in a way older builds can't read.
/// Version 2 added audio clips.
pub const FORMAT_VERSION: u32 = 2;

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
    /// Longest audio file name a project can hold.
    pub const MAX_PATH_CHARS: usize = 4096;
    /// Furthest into an audio file a clip can start, in seconds.
    pub const MAX_AUDIO_OFFSET: f64 = 36_000.0;
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
    /// The song's key: its root note, 0 = C to 11 = B.
    pub key: u8,
    /// The song's scale, by id ([`crate::music::SCALES`]).
    pub scale: String,
    /// Credits and identifiers written into exported files (artist, ISRC, ...).
    #[serde(skip_serializing_if = "crate::metadata::Metadata::is_empty")]
    pub metadata: crate::metadata::Metadata,
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
            key: 0,
            scale: "major".into(),
            metadata: crate::metadata::Metadata::default(),
            tracks: Vec::new(),
        }
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Names this track in `parent`. Zero means "not assigned yet"; [`Project::sanitize`] gives
    /// every track a unique one, so it stays put when tracks move.
    #[serde(skip_serializing_if = "is_zero")]
    pub id: u32,
    /// A group track has no instrument or clips: it mixes the tracks inside it, runs its own
    /// effects on that mix, and sends the result on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub group: bool,
    /// The `id` of the group this track is in, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub instrument: Option<Device>,
    /// The arpeggiator, a MIDI effect in front of the instrument.
    pub arp: crate::arp::ArpSettings,
    pub effects: Vec<Device>,
    pub clips: Vec<Clip>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: 0,
            group: false,
            parent: None,
            name: "Track".into(),
            volume: 0.8,
            pan: 0.0,
            mute: false,
            solo: false,
            instrument: None,
            arp: crate::arp::ArpSettings::default(),
            effects: Vec::new(),
            clips: Vec::new(),
        }
    }
}

impl Track {
    /// The volume the track's clip fades give at `beat`: that of the clip that started last
    /// (muted ones don't count), or full volume before the first clip.
    pub fn fade_gain(&self, beat: f64) -> f32 {
        let mut latest: Option<&Clip> = None;
        for clip in &self.clips {
            if !clip.muted && clip.start <= beat && latest.is_none_or(|l| clip.start >= l.start) {
                latest = Some(clip);
            }
        }
        latest.and_then(|c| c.fade_gain(beat)).unwrap_or(1.0)
    }
}

/// An instrument or effect on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Device {
    /// A device built into AudioZero, such as `"operator"` or `"reverb"`, with its settings by
    /// name (see [`crate::effects::params_for`] and [`crate::synth::instrument_params`]).
    Builtin {
        id: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, f32>,
    },
    /// A third-party plugin, named by identity only.
    Plugin(PluginRef),
}

impl Device {
    /// A built-in device with its default settings.
    pub fn builtin(id: &str) -> Self {
        Self::Builtin {
            id: id.into(),
            params: BTreeMap::new(),
        }
    }

    /// The built-in id, if this is a built-in device.
    pub fn builtin_id(&self) -> Option<&str> {
        match self {
            Self::Builtin { id, .. } => Some(id),
            Self::Plugin(_) => None,
        }
    }

    /// The parameters this device has, if it's a built-in effect or instrument with any.
    pub fn params_info(&self) -> Option<&'static [crate::effects::Param]> {
        let id = self.builtin_id()?;
        crate::effects::params_for(id).or_else(|| crate::synth::instrument_params(id))
    }

    /// A parameter's value: the stored one, else the default. `None` if this isn't a built-in
    /// device with that parameter.
    pub fn param(&self, name: &str) -> Option<f32> {
        let Self::Builtin { params, .. } = self else {
            return None;
        };
        let info = self.params_info()?.iter().find(|p| p.name == name)?;
        Some(params.get(name).copied().unwrap_or(info.default))
    }

    /// Keeps only parameters the device has, clamped into range.
    fn sanitize(&mut self) {
        if let Self::Builtin { id, .. } = self {
            truncate_name(id);
        }
        let info = self.params_info();
        if let Self::Builtin { params, .. } = self {
            match info {
                None => params.clear(),
                Some(info) => {
                    params.retain(|name, _| info.iter().any(|p| p.name == name));
                    for p in info {
                        if let Some(v) = params.get_mut(p.name) {
                            *v = p.fit(*v);
                        }
                    }
                }
            }
        }
    }
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

/// A clip on the timeline: MIDI notes, or a region of an audio file when `audio` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clip {
    pub name: String,
    /// Position on the timeline, in beats.
    pub start: f64,
    pub length: f64,
    pub muted: bool,
    /// MIDI notes. Always empty for audio clips.
    pub notes: Vec<Note>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioRegion>,
    /// How long the track's volume takes to rise from silence at the start of the clip, in
    /// beats. Zero for no fade.
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub fade_in: f64,
    /// How long the volume takes to fall to silence at the end of the clip, in beats. The track
    /// then stays silent until its next clip starts.
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub fade_out: f64,
}

fn is_zero_f64(x: &f64) -> bool {
    *x == 0.0
}

/// The audio an audio clip plays. It plays at the file's own speed (no time-stretching yet), so
/// how many beats it covers depends on the tempo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioRegion {
    /// The audio file: relative to the project's folder, or absolute. Checked by
    /// [`SamplePool::resolve`](crate::sample::SamplePool::resolve) before it's opened.
    pub file: String,
    /// Where in the file the clip starts, in seconds.
    pub offset: f64,
    /// Linear gain, 1 = as recorded.
    pub gain: f32,
}

impl Default for AudioRegion {
    fn default() -> Self {
        Self {
            file: String::new(),
            offset: 0.0,
            gain: 1.0,
        }
    }
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            name: "Clip".into(),
            start: 0.0,
            length: 4.0,
            muted: false,
            notes: Vec::new(),
            audio: None,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }
}

impl Clip {
    /// The volume the clip's fades give at `beat` on the timeline (0 to 1), or `None` before
    /// the clip starts. After a clip that fades out, it stays at zero.
    pub fn fade_gain(&self, beat: f64) -> Option<f32> {
        let t = beat - self.start;
        if t < 0.0 {
            return None;
        }
        let rise = if self.fade_in > 0.0 {
            (t / self.fade_in).min(1.0)
        } else {
            1.0
        };
        let fall = if self.fade_out > 0.0 {
            ((self.length - t) / self.fade_out).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Squared, so the fade sounds even rather than dropping away at the end.
        let g = (rise * fall) as f32;
        Some(g * g)
    }

    pub fn is_audio(&self) -> bool {
        self.audio.is_some()
    }

    pub fn end(&self) -> f64 {
        self.start + self.length
    }

    /// Clamps the clip and its notes into range and drops notes outside the clip. Part of
    /// [`Project::sanitize`], and also used on its own when one track's clips are edited.
    /// Doesn't allocate.
    pub fn sanitize(&mut self) {
        use limits::MAX_BEATS;
        truncate_name(&mut self.name);
        self.start = finite(self.start, 0.0, MAX_BEATS, 0.0);
        let room = (MAX_BEATS - self.start).max(1.0 / 16.0);
        self.length = finite(self.length, 1.0 / 16.0, room, 4.0_f64.min(room));
        if let Some(audio) = &mut self.audio {
            if let Some((cut, _)) = audio.file.char_indices().nth(limits::MAX_PATH_CHARS) {
                audio.file.truncate(cut);
            }
            audio.offset = finite(audio.offset, 0.0, limits::MAX_AUDIO_OFFSET, 0.0);
            audio.gain = finite32(audio.gain, 0.0, 4.0, 1.0);
            self.notes.clear();
        }
        self.fade_in = finite(self.fade_in, 0.0, self.length, 0.0);
        self.fade_out = finite(self.fade_out, 0.0, self.length - self.fade_in, 0.0);
        let length = self.length;
        self.notes
            .retain(|n| n.start.is_finite() && n.start >= 0.0 && n.start < length);
        for note in &mut self.notes {
            note.pitch = note.pitch.min(127);
            note.length = finite(note.length, 1.0 / 128.0, MAX_BEATS, 1.0);
            note.velocity = finite32(note.velocity, 0.0, 1.0, 0.8);
        }
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
pub(crate) fn finite(value: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if value.is_finite() {
        value.clamp(lo, hi)
    } else {
        default
    }
}

pub(crate) fn finite32(value: f32, lo: f32, hi: f32, default: f32) -> f32 {
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
        let meta = std::fs::metadata(path)?;
        // A link to /dev/zero or a FIFO reports a length of 0 but never ends, so only regular
        // files are read, and never more than the limit (the file can also grow while we read).
        if !meta.is_file() {
            return Err(ProjectError::WrongFormat);
        }
        if meta.len() > limits::MAX_FILE_BYTES {
            return Err(ProjectError::TooLarge(meta.len()));
        }
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut std::io::Read::take(std::fs::File::open(path)?, limits::MAX_FILE_BYTES + 1),
            &mut text,
        )?;
        Self::from_json(&text)
    }

    /// Writes to a temporary file next to `path` and renames it, so a crash mid-save never
    /// leaves a half-written project.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        // Never through a link: a folder from someone else could hold `song.azproj.tmp` pointing
        // at one of the user's files (security review, Rust app review, finding 2).
        let mut file = crate::safe_fs::create_new(&tmp)?;
        let written = std::io::Write::write_all(&mut file, self.to_json().as_bytes());
        drop(file);
        if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
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
        self.metadata.sanitize();
        self.key %= 12;
        self.scale = crate::music::scale_by_id(&self.scale).id.into();

        for track in &mut self.tracks {
            truncate_name(&mut track.name);
            track.arp.sanitize();
            track.volume = finite32(track.volume, 0.0, 2.0, 0.8);
            track.pan = finite32(track.pan, -1.0, 1.0, 0.0);
            if let Some(instrument) = &mut track.instrument {
                instrument.sanitize();
            }
            for effect in &mut track.effects {
                effect.sanitize();
            }
            for clip in &mut track.clips {
                clip.sanitize();
            }
            if track.group {
                track.instrument = None;
                track.clips.clear();
                track.arp = crate::arp::ArpSettings::default();
            }
        }
        self.sanitize_groups();
        Ok(())
    }

    /// Gives every track a unique id, then drops parents that don't name a group and breaks any
    /// loops, so the tracks always form a tree.
    fn sanitize_groups(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut next = self.tracks.iter().map(|t| t.id).max().unwrap_or(0);
        for track in &mut self.tracks {
            if track.id == 0 || !seen.insert(track.id) {
                next += 1;
                track.id = next;
                seen.insert(next);
            }
        }
        for i in 0..self.tracks.len() {
            let valid = self.tracks[i]
                .parent
                .and_then(|id| self.index_of(id))
                .is_some_and(|p| p != i && self.tracks[p].group);
            if !valid {
                self.tracks[i].parent = None;
            }
        }
        // A loop never reaches the top, so cut it where it is found.
        for i in 0..self.tracks.len() {
            let mut at = i;
            for _ in 0..=self.tracks.len() {
                match self.parent_of(at) {
                    Some(p) => at = p,
                    None => break,
                }
                if at == i {
                    self.tracks[i].parent = None;
                    break;
                }
            }
        }
    }

    /// The index of the track with this id.
    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// The index of the group a track is in.
    pub fn parent_of(&self, index: usize) -> Option<usize> {
        self.index_of(self.tracks.get(index)?.parent?)
    }

    /// How many groups deep a track is.
    pub fn depth(&self, index: usize) -> usize {
        let mut depth = 0;
        let mut at = index;
        while let Some(p) = self.parent_of(at) {
            depth += 1;
            at = p;
            if depth > self.tracks.len() {
                break;
            }
        }
        depth
    }

    /// Whether `index` is `group` or somewhere inside it.
    pub fn is_inside(&self, index: usize, group: usize) -> bool {
        let mut at = index;
        for _ in 0..=self.tracks.len() {
            if at == group {
                return true;
            }
            match self.parent_of(at) {
                Some(p) => at = p,
                None => return false,
            }
        }
        false
    }

    /// The track at `index` and everything inside it, as a range of indices (the tracks of a
    /// group follow it, which the editing functions keep true).
    pub fn block(&self, index: usize) -> std::ops::Range<usize> {
        let mut end = index + 1;
        while end < self.tracks.len() && self.is_inside(end, index) {
            end += 1;
        }
        index..end
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
        let mut project = crate::demo::project();
        project.sanitize().unwrap(); // gives the tracks their ids, as loading does
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
    fn audio_clips_round_trip_and_are_sanitized() {
        let json = minimal(
            r#","tracks":[{"clips":[{"start":2,"length":8,"audio":{"file":"loops/drums.wav","offset":-3,"gain":99},"notes":[{"pitch":60,"start":0,"length":1,"velocity":1}]}]}]"#,
        );
        let p = Project::from_json(&json).unwrap();
        let clip = &p.tracks[0].clips[0];
        let audio = clip.audio.as_ref().unwrap();
        assert_eq!(audio.file, "loops/drums.wav");
        assert_eq!((audio.offset, audio.gain), (0.0, 4.0));
        assert!(clip.notes.is_empty(), "audio clips have no notes");
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        assert!(
            !crate::demo::project().to_json().contains("\"audio\""),
            "MIDI clips don't write an audio field"
        );
    }

    #[test]
    fn effect_params_are_kept_by_name_and_clamped() {
        let json = minimal(
            r#","tracks":[{"effects":[{"type":"builtin","id":"delay","params":{"time":99,"mix":0.5,"bogus":1}},{"type":"builtin","id":"reverb"}]}]"#,
        );
        let p = Project::from_json(&json).unwrap();
        let delay = &p.tracks[0].effects[0];
        assert_eq!(delay.param("time"), Some(2.0), "clamped to the maximum");
        assert_eq!(delay.param("mix"), Some(0.5));
        assert_eq!(delay.param("feedback"), Some(0.45), "missing means default");
        assert_eq!(delay.param("bogus"), None);
        assert!(
            !p.to_json().contains("bogus"),
            "unknown parameters are dropped"
        );
        assert!(
            !p.to_json().contains("\"params\": {}"),
            "no params written for defaults"
        );
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
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
        let mut project = crate::demo::project();
        project.sanitize().unwrap();
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn save_never_writes_through_a_link() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("audiozero-save-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        let song = dir.join("song.azproj");
        symlink(&victim, dir.join("song.azproj.tmp")).unwrap();
        assert!(Project::default().save(&song).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
        assert!(!song.exists(), "nothing was renamed into place");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn load_refuses_files_that_are_not_regular() {
        // /dev/zero reports a length of 0 and never ends: it used to be read until memory ran out.
        assert!(matches!(
            Project::load(Path::new("/dev/zero")),
            Err(ProjectError::WrongFormat)
        ));
    }
}
