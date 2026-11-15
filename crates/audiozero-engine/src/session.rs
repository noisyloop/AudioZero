//! Every edit to a project, in one place, with undo.
//!
//! A [`Session`] owns the project being edited. The window and the MCP server both edit through
//! it, so every edit, whoever makes it:
//!
//! 1. is checked by [`Project::sanitize`], the same check project files get, and is rolled back
//!    if a limit is exceeded;
//! 2. becomes an undo step (except mixer and transport settings, which aren't undo steps in
//!    most DAWs);
//! 3. reports what changed for the audio thread, as [`Changes`], which
//!    [`Controller::apply`](crate::Controller::apply) sends on.
//!
//! The typed operations ([`Session::add_track`], [`Session::add_notes`] and so on) check their
//! arguments and say what was wrong in an [`EditError`]. [`Session::edit`] takes any change as a
//! closure, for the window's direct manipulation.

use crate::engine::TrackMix;
use crate::project::{Clip, Device, Note, Project, ProjectError, Track};
use std::collections::VecDeque;
use std::fmt;

/// How many steps can be undone. Each step is a whole-project snapshot.
pub const MAX_UNDO_STEPS: usize = 100;

/// A clip, by position. Positions shift when clips or tracks are added or removed, so check them
/// with [`Session::clip`] before use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipRef {
    pub track: usize,
    pub clip: usize,
}

/// Why an edit was refused. The project is left as it was.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    NoTrack(usize),
    NoClip(ClipRef),
    NoNote {
        clip: ClipRef,
        note: usize,
    },
    /// An argument is out of range, with what was wrong.
    Invalid(String),
    /// The edit would take the project past one of its limits.
    Limit(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTrack(i) => write!(f, "there is no track {i}"),
            Self::NoClip(c) => write!(f, "track {} has no clip {}", c.track, c.clip),
            Self::NoNote { clip, note } => write!(
                f,
                "clip {} on track {} has no note {note}",
                clip.clip, clip.track
            ),
            Self::Invalid(what) => f.write_str(what),
            Self::Limit(what) => write!(f, "that would exceed a limit: {what}"),
        }
    }
}

impl std::error::Error for EditError {}

impl From<ProjectError> for EditError {
    fn from(e: ProjectError) -> Self {
        match e {
            ProjectError::Limit(what) => Self::Limit(what),
            other => Self::Invalid(other.to_string()),
        }
    }
}

/// What has to be sent to the audio thread to bring it up to date after an edit.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Changes {
    /// Tracks or their devices changed: the engine has to be rebuilt.
    pub reload: bool,
    /// Tracks whose clips changed.
    pub clips: Vec<usize>,
    /// Tracks whose volume, pan, mute or solo changed.
    pub mix: Vec<usize>,
    /// (track, effect) pairs whose parameters changed, with the effects themselves unchanged.
    pub effect_params: Vec<(usize, usize)>,
    /// Instrument parameters that changed: (track, parameter index, new value).
    pub instrument_params: Vec<(usize, usize, f32)>,
    /// Tracks whose arpeggiator settings changed.
    pub arp: Vec<usize>,
    pub bpm: bool,
    pub master: bool,
    pub looping: bool,
}

impl Changes {
    /// Nothing the audio thread needs to hear about. (The project may still have changed, for
    /// example a track's name.)
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Works out what the audio thread needs to go from `before` to `after`.
pub fn changes(before: &Project, after: &Project) -> Changes {
    let reload = before.tracks.len() != after.tracks.len()
        || before.beats_per_bar != after.beats_per_bar
        || before.tracks.iter().zip(&after.tracks).any(|(a, b)| {
            !same_instrument(&a.instrument, &b.instrument)
                || a.effects.len() != b.effects.len()
                || a.effects
                    .iter()
                    .zip(&b.effects)
                    .any(|(x, y)| !same_device(x, y))
        });
    if reload {
        return Changes {
            reload,
            ..Default::default()
        };
    }
    let pairs = || before.tracks.iter().zip(&after.tracks).enumerate();
    Changes {
        reload: false,
        clips: pairs()
            .filter(|(_, (a, b))| a.clips != b.clips)
            .map(|(i, _)| i)
            .collect(),
        effect_params: pairs()
            .flat_map(|(t, (a, b))| {
                a.effects
                    .iter()
                    .zip(&b.effects)
                    .enumerate()
                    .filter(|(_, (x, y))| x != y)
                    .map(move |(e, _)| (t, e))
            })
            .collect(),
        arp: pairs()
            .filter(|(_, (a, b))| a.arp != b.arp)
            .map(|(i, _)| i)
            .collect(),
        instrument_params: pairs()
            .filter(|(_, (a, b))| a.instrument != b.instrument)
            .flat_map(|(t, (a, b))| {
                let (a, b) = (a.instrument.as_ref(), b.instrument.as_ref());
                let info = b.and_then(Device::params_info).unwrap_or_default();
                info.iter().enumerate().filter_map(move |(i, p)| {
                    let new = b?.param(p.name)?;
                    (a?.param(p.name) != Some(new)).then_some((t, i, new))
                })
            })
            .collect(),
        mix: pairs()
            .filter(|(_, (a, b))| mix_of(a) != mix_of(b))
            .map(|(i, _)| i)
            .collect(),
        bpm: before.bpm != after.bpm,
        master: before.master_volume != after.master_volume,
        looping: (before.loop_enabled, before.loop_start, before.loop_end)
            != (after.loop_enabled, after.loop_start, after.loop_end),
    }
}

/// The same device, ignoring its parameter values (which change live, without a rebuild).
fn same_device(a: &Device, b: &Device) -> bool {
    match (a, b) {
        (Device::Builtin { id: x, .. }, Device::Builtin { id: y, .. }) => x == y,
        _ => a == b,
    }
}

fn same_instrument(a: &Option<Device>, b: &Option<Device>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_device(a, b),
        (None, None) => true,
        _ => false,
    }
}

pub fn mix_of(track: &Track) -> TrackMix {
    TrackMix {
        volume: track.volume,
        pan: track.pan,
        mute: track.mute,
        solo: track.solo,
    }
}

/// Copies the settings that aren't undo steps (tempo, loop, master volume and each track's mix)
/// from `current` into a snapshot being restored, so undoing a note edit doesn't also move a
/// fader back. Tracks are matched by position.
fn keep_unrecorded(current: &Project, restored: &mut Project) {
    restored.bpm = current.bpm;
    restored.master_volume = current.master_volume;
    restored.loop_enabled = current.loop_enabled;
    restored.loop_start = current.loop_start;
    restored.loop_end = current.loop_end;
    for (to, from) in restored.tracks.iter_mut().zip(&current.tracks) {
        (to.volume, to.pan, to.mute, to.solo) = (from.volume, from.pan, from.mute, from.solo);
    }
}

/// A project being edited, with its undo history.
#[derive(Debug)]
pub struct Session {
    project: Project,
    undo: VecDeque<Project>,
    redo: Vec<Project>,
    /// The project as it was when the current gesture started, for its undo step.
    gesture: Option<Project>,
    dirty: bool,
}

impl Session {
    /// Starts editing `project`, after sanitizing it.
    pub fn new(mut project: Project) -> Result<Self, ProjectError> {
        project.sanitize()?;
        Ok(Self {
            project,
            undo: VecDeque::new(),
            redo: Vec::new(),
            gesture: None,
            dirty: false,
        })
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Whether there are changes that haven't been saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Call after saving.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.gesture.as_ref().is_some_and(|g| *g != self.project)
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn track(&self, index: usize) -> Option<&Track> {
        self.project.tracks.get(index)
    }

    pub fn clip(&self, at: ClipRef) -> Option<&Clip> {
        self.project.tracks.get(at.track)?.clips.get(at.clip)
    }

    // ---- the edit pipeline ------------------------------------------------------------------

    /// Applies `change`, sanitizes the result and keeps it, or rolls back if `change` or the
    /// sanitize fails. Returns the previous project if anything changed.
    fn apply<T>(
        &mut self,
        change: impl FnOnce(&mut Project) -> Result<T, EditError>,
    ) -> Result<(T, Option<Project>, Changes), EditError> {
        let before = self.project.clone();
        let value = match change(&mut self.project) {
            Ok(value) => value,
            Err(e) => {
                self.project = before;
                return Err(e);
            }
        };
        if let Err(e) = self.project.sanitize() {
            self.project = before;
            return Err(e.into());
        }
        if self.project == before {
            return Ok((value, None, Changes::default()));
        }
        let changes = changes(&before, &self.project);
        self.dirty = true;
        Ok((value, Some(before), changes))
    }

    fn record(&mut self, before: Project) {
        self.undo.push_back(before);
        if self.undo.len() > MAX_UNDO_STEPS {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// Applies a change as one undo step. The closure may return an error to refuse the edit.
    pub fn try_edit<T>(
        &mut self,
        change: impl FnOnce(&mut Project) -> Result<T, EditError>,
    ) -> Result<(T, Changes), EditError> {
        let (value, before, changes) = self.apply(change)?;
        if let Some(before) = before
            && self.gesture.is_none()
        {
            self.record(before);
        }
        Ok((value, changes))
    }

    /// Applies a change as one undo step.
    pub fn edit(&mut self, change: impl FnOnce(&mut Project)) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            change(p);
            Ok(())
        })
        .map(|((), changes)| changes)
    }

    /// Applies a change that isn't an undo step (mixer and transport settings). It still counts
    /// as an unsaved change.
    pub fn set(&mut self, change: impl FnOnce(&mut Project)) -> Result<Changes, EditError> {
        self.apply(|p| {
            change(p);
            Ok(())
        })
        .map(|((), _, changes)| changes)
    }

    /// Starts a gesture, such as a drag. Every edit until [`Session::end_gesture`] becomes one
    /// undo step.
    pub fn begin_gesture(&mut self) {
        if self.gesture.is_none() {
            self.gesture = Some(self.project.clone());
        }
    }

    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// Ends the current gesture, recording it as one undo step if it changed anything.
    pub fn end_gesture(&mut self) {
        if let Some(start) = self.gesture.take()
            && start != self.project
        {
            self.record(start);
        }
    }

    /// Goes back one step. Mixer and transport settings stay as they are.
    pub fn undo(&mut self) -> Option<Changes> {
        self.end_gesture();
        let mut previous = self.undo.pop_back()?;
        keep_unrecorded(&self.project, &mut previous);
        let before = std::mem::replace(&mut self.project, previous);
        let changes = changes(&before, &self.project);
        self.redo.push(before);
        self.dirty = true;
        Some(changes)
    }

    pub fn redo(&mut self) -> Option<Changes> {
        self.end_gesture();
        let mut next = self.redo.pop()?;
        keep_unrecorded(&self.project, &mut next);
        let before = std::mem::replace(&mut self.project, next);
        let changes = changes(&before, &self.project);
        self.undo.push_back(before);
        self.dirty = true;
        Some(changes)
    }

    // ---- tracks -----------------------------------------------------------------------------

    /// Adds a track at the end and returns its index.
    pub fn add_track(&mut self, track: Track) -> Result<(usize, Changes), EditError> {
        self.try_edit(|p| {
            p.tracks.push(track);
            Ok(p.tracks.len() - 1)
        })
    }

    /// Adds a ready-made drum beat ([`crate::drums::BEATS`]) as a clip, repeated `repeats` times.
    ///
    /// With a `track` (which needs the drum rack) the clip goes at `start` beats, or after the
    /// track's last clip, on a bar line. With none, a new "Drums" track with the beat's kit is
    /// made and the clip goes at `start` (default 0). In a song with no clips yet, the tempo is
    /// set to the beat's. All one undo step. Returns the clip, the track's index, and whether
    /// the tempo was set.
    pub fn add_drum_beat(
        &mut self,
        track: Option<usize>,
        beat_id: &str,
        repeats: usize,
        start: Option<f64>,
    ) -> Result<((ClipRef, bool), Changes), EditError> {
        let beat = crate::drums::beat_by_id(beat_id).ok_or_else(|| {
            let ids: Vec<_> = crate::drums::BEATS.iter().map(|b| b.id).collect();
            EditError::Invalid(format!(
                "unknown beat \"{beat_id}\"; use one of {}",
                ids.join(", ")
            ))
        })?;
        if !(1..=64).contains(&repeats) {
            return Err(EditError::Invalid(format!(
                "repeats has to be 1 to 64, got {repeats}"
            )));
        }
        if let Some(start) = start {
            check_beat("start", start)?;
        }
        self.try_edit(|p| {
            let empty = p.tracks.iter().all(|t| t.clips.is_empty());
            let index = match track {
                Some(index) => {
                    check_track(p, index)?;
                    if p.tracks[index].instrument.as_ref().and_then(Device::builtin_id)
                        != Some("drums")
                    {
                        return Err(EditError::Invalid(format!(
                            "track {index} doesn't have the drum rack; set its instrument to \"drums\""
                        )));
                    }
                    index
                }
                None => {
                    let kit = crate::drums::KITS
                        .iter()
                        .position(|k| k.id == beat.genre)
                        .unwrap_or(0);
                    let mut device = Device::builtin("drums");
                    if let Device::Builtin { params, .. } = &mut device {
                        params.insert("kit".into(), kit as f32);
                    }
                    p.tracks.push(Track {
                        name: "Drums".into(),
                        instrument: Some(device),
                        ..Default::default()
                    });
                    p.tracks.len() - 1
                }
            };
            let bar = f64::from(p.beats_per_bar);
            let start = start.unwrap_or_else(|| {
                let end = p.tracks[index]
                    .clips
                    .iter()
                    .map(Clip::end)
                    .fold(0.0, f64::max);
                (end / bar).ceil() * bar
            });
            let clip = beat.clip(repeats, start, p.beats_per_bar);
            check_clip_shape(&clip)?;
            for note in &clip.notes {
                check_note(note, clip.length)?;
            }
            let clips = &mut p.tracks[index].clips;
            clips.push(clip);
            let at = ClipRef {
                track: index,
                clip: clips.len() - 1,
            };
            if empty {
                p.bpm = beat.bpm;
            }
            Ok((at, empty))
        })
    }

    pub fn remove_track(&mut self, index: usize) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_track(p, index)?;
            p.tracks.remove(index);
            Ok(())
        })
        .map(|((), c)| c)
    }

    pub fn rename_track(&mut self, index: usize, name: &str) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_track(p, index)?;
            p.tracks[index].name = name.trim().to_owned();
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Sets or removes a track's instrument. This is an undo step.
    pub fn set_instrument(
        &mut self,
        index: usize,
        instrument: Option<Device>,
    ) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_track(p, index)?;
            p.tracks[index].instrument = instrument;
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Adds a built-in effect at the end of a track's chain and returns its index.
    pub fn add_effect(&mut self, track: usize, id: &str) -> Result<(usize, Changes), EditError> {
        if crate::effects::params_for(id).is_none() {
            let ids: Vec<_> = crate::effects::EFFECTS.iter().map(|(id, _)| *id).collect();
            return Err(EditError::Invalid(format!(
                "unknown effect \"{id}\"; use one of {}",
                ids.join(", ")
            )));
        }
        self.try_edit(|p| {
            check_track(p, track)?;
            let effects = &mut p.tracks[track].effects;
            effects.push(Device::builtin(id));
            Ok(effects.len() - 1)
        })
    }

    pub fn remove_effect(&mut self, track: usize, effect: usize) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_effect(p, track, effect)?;
            p.tracks[track].effects.remove(effect);
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Sets one parameter of an effect. An undo step (a whole knob drag is one, inside a
    /// gesture), heard without rebuilding the engine.
    pub fn set_effect_param(
        &mut self,
        track: usize,
        effect: usize,
        name: &str,
        value: f32,
    ) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_effect(p, track, effect)?;
            let Device::Builtin { id, params } = &mut p.tracks[track].effects[effect] else {
                return Err(EditError::Invalid(
                    "plugin parameters aren't supported yet".into(),
                ));
            };
            let info = crate::effects::params_for(id)
                .and_then(|ps| ps.iter().find(|p| p.name == name))
                .ok_or_else(|| {
                    EditError::Invalid(format!("effect \"{id}\" has no parameter \"{name}\""))
                })?;
            check_range(
                name,
                f64::from(value),
                f64::from(info.min),
                f64::from(info.max),
            )?;
            params.insert(name.to_owned(), value);
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Sets a track's arpeggiator (out-of-range values are refused, not clamped). An undo step,
    /// heard without rebuilding the engine.
    pub fn set_arp(
        &mut self,
        track: usize,
        settings: crate::arp::ArpSettings,
    ) -> Result<Changes, EditError> {
        check_range("gate", f64::from(settings.gate), 0.05, 1.0)?;
        check_range("swing", f64::from(settings.swing), 0.0, 0.5)?;
        if !(1..=4).contains(&settings.octaves) {
            return Err(EditError::Invalid(format!(
                "octaves has to be 1 to 4, got {}",
                settings.octaves
            )));
        }
        self.try_edit(|p| {
            check_track(p, track)?;
            p.tracks[track].arp = settings;
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Sets one parameter of a track's built-in instrument. An undo step (a whole knob drag is
    /// one, inside a gesture), heard without rebuilding the engine. Stepped parameters
    /// (switches, choices, whole numbers) are rounded.
    pub fn set_instrument_param(
        &mut self,
        track: usize,
        name: &str,
        value: f32,
    ) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_track(p, track)?;
            let device = p.tracks[track]
                .instrument
                .as_mut()
                .ok_or_else(|| EditError::Invalid(format!("track {track} has no instrument")))?;
            let info = device
                .params_info()
                .and_then(|ps| ps.iter().find(|p| p.name == name))
                .ok_or_else(|| {
                    EditError::Invalid(format!("the instrument has no parameter \"{name}\""))
                })?;
            check_range(
                name,
                f64::from(value),
                f64::from(info.min),
                f64::from(info.max),
            )?;
            if let Device::Builtin { params, .. } = device {
                params.insert(name.to_owned(), info.fit(value));
            }
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Sets every parameter of a track's built-in instrument from one of its presets
    /// ([`crate::synth::instrument_presets`]). One undo step.
    pub fn set_instrument_preset(
        &mut self,
        track: usize,
        preset: &str,
    ) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_track(p, track)?;
            let Some(Device::Builtin { id, params }) = &mut p.tracks[track].instrument else {
                return Err(EditError::Invalid(format!(
                    "track {track} has no built-in instrument"
                )));
            };
            let values = crate::synth::instrument_preset(id, preset).ok_or_else(|| {
                let names = crate::synth::instrument_presets(id);
                EditError::Invalid(if names.is_empty() {
                    format!("\"{id}\" has no presets")
                } else {
                    format!("no preset \"{preset}\"; use one of {}", names.join(", "))
                })
            })?;
            *params = values
                .iter()
                .map(|&(name, value)| (name.to_owned(), value))
                .collect();
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Sets a track's volume, pan, mute and solo. Not an undo step.
    pub fn set_track_mix(&mut self, index: usize, mix: TrackMix) -> Result<Changes, EditError> {
        check_track(&self.project, index)?;
        self.set(|p| {
            let t = &mut p.tracks[index];
            (t.volume, t.pan, t.mute, t.solo) = (mix.volume, mix.pan, mix.mute, mix.solo);
        })
    }

    // ---- clips ------------------------------------------------------------------------------

    /// Adds a clip, with any notes it has, to a track and returns where it went.
    pub fn add_clip(&mut self, track: usize, clip: Clip) -> Result<(ClipRef, Changes), EditError> {
        check_clip_shape(&clip)?;
        if clip.is_audio() && !clip.notes.is_empty() {
            return Err(EditError::Invalid("audio clips can't have notes".into()));
        }
        for note in &clip.notes {
            check_note(note, clip.length)?;
        }
        self.try_edit(|p| {
            check_track(p, track)?;
            let clips = &mut p.tracks[track].clips;
            clips.push(clip);
            Ok(ClipRef {
                track,
                clip: clips.len() - 1,
            })
        })
    }

    pub fn remove_clip(&mut self, at: ClipRef) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            p.tracks[at.track].clips.remove(at.clip);
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Puts a copy of a clip straight after it on the same track.
    pub fn duplicate_clip(&mut self, at: ClipRef) -> Result<(ClipRef, Changes), EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let clips = &mut p.tracks[at.track].clips;
            let mut copy = clips[at.clip].clone();
            // Straight after the clip if that's free, else after whatever is in the way, so
            // copies never pile up on top of other clips.
            copy.start = copy.end();
            loop {
                let (start, end) = (copy.start, copy.end());
                let blocker = clips
                    .iter()
                    .filter(|c| c.start < end - 1e-9 && c.end() > start + 1e-9)
                    .map(Clip::end)
                    .fold(None, |furthest: Option<f64>, e| {
                        Some(furthest.map_or(e, |f| f.max(e)))
                    });
                match blocker {
                    Some(end) => copy.start = end,
                    None => break,
                }
            }
            check_beat("start", copy.end())?;
            clips.push(copy);
            Ok(ClipRef {
                track: at.track,
                clip: clips.len() - 1,
            })
        })
    }

    /// Moves a clip to `start` on track `to_track` (which can be its own track) and returns where
    /// it went.
    pub fn move_clip(
        &mut self,
        at: ClipRef,
        to_track: usize,
        start: f64,
    ) -> Result<(ClipRef, Changes), EditError> {
        check_beat("start", start)?;
        self.try_edit(|p| {
            check_clip(p, at)?;
            check_track(p, to_track)?;
            if to_track == at.track {
                p.tracks[at.track].clips[at.clip].start = start;
                return Ok(at);
            }
            let mut clip = p.tracks[at.track].clips.remove(at.clip);
            clip.start = start;
            let clips = &mut p.tracks[to_track].clips;
            clips.push(clip);
            Ok(ClipRef {
                track: to_track,
                clip: clips.len() - 1,
            })
        })
    }

    /// Changes a clip in place (name, start, length, mute). Notes that end up outside the clip
    /// are dropped, as they are when a project loads.
    pub fn update_clip(
        &mut self,
        at: ClipRef,
        change: impl FnOnce(&mut Clip),
    ) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let clip = &mut p.tracks[at.track].clips[at.clip];
            change(clip);
            check_clip_shape(clip)?;
            Ok(())
        })
        .map(|((), c)| c)
    }

    // ---- notes ------------------------------------------------------------------------------

    /// Adds notes to a clip. Every note has to start inside the clip. Returns the clip's note
    /// count afterwards; the new notes are at the end.
    pub fn add_notes(
        &mut self,
        at: ClipRef,
        notes: &[Note],
    ) -> Result<(usize, Changes), EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let clip = &mut p.tracks[at.track].clips[at.clip];
            no_audio(clip)?;
            for note in notes {
                check_note(note, clip.length)?;
            }
            clip.notes.extend_from_slice(notes);
            Ok(clip.notes.len())
        })
    }

    /// Replaces all of a clip's notes.
    pub fn set_notes(&mut self, at: ClipRef, notes: Vec<Note>) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let clip = &mut p.tracks[at.track].clips[at.clip];
            no_audio(clip)?;
            for note in &notes {
                check_note(note, clip.length)?;
            }
            clip.notes = notes;
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Copies notes to straight after the last of them, as Ctrl+D does, and returns the
    /// copies' indices. With a `grid` (in beats, 0 for none) the copies keep the originals'
    /// place on it: the distance is rounded up to whole grid steps. The clip grows by whole bars if the copies need the room and no later
    /// clip on the track is in the way; copies that still don't fit are left out.
    pub fn duplicate_notes(
        &mut self,
        at: ClipRef,
        indices: &[usize],
        grid: f64,
    ) -> Result<(Vec<usize>, Changes), EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let bar = f64::from(p.beats_per_bar);
            let clips = &p.tracks[at.track].clips;
            let clip = &clips[at.clip];
            no_audio(clip)?;
            if let Some(&note) = indices.iter().find(|&&i| i >= clip.notes.len()) {
                return Err(EditError::NoNote { clip: at, note });
            }
            let mut picked: Vec<usize> = indices.to_vec();
            picked.sort_unstable();
            picked.dedup();
            let chosen: Vec<Note> = picked.iter().map(|&i| clip.notes[i]).collect();
            let Some(first) = chosen.iter().map(|n| n.start).reduce(f64::min) else {
                return Ok(Vec::new());
            };
            let last = chosen
                .iter()
                .map(|n| n.start + n.length)
                .fold(first, f64::max);
            let mut shift = last - first;
            if grid.is_finite() && grid > 1e-6 {
                shift = ((shift / grid) - 1e-6).ceil().max(1.0) * grid;
            }
            // How far the clip can grow: to the next clip on the track.
            let room = clips
                .iter()
                .filter(|c| c.start >= clip.end() - 1e-9)
                .map(|c| c.start - clip.start)
                .fold(crate::project::limits::MAX_BEATS - clip.start, f64::min);
            let wanted = ((last + shift) / bar).ceil() * bar;
            let length = clip.length.max(wanted.min(room));

            let clip = &mut p.tracks[at.track].clips[at.clip];
            clip.length = length;
            let mut copies = Vec::new();
            for note in chosen {
                let copy = Note {
                    start: note.start + shift,
                    ..note
                };
                if copy.start < length - 1e-9 {
                    copies.push(clip.notes.len());
                    clip.notes.push(copy);
                }
            }
            Ok(copies)
        })
    }

    /// Removes notes by index. Every index has to exist; duplicates are ignored.
    pub fn remove_notes(&mut self, at: ClipRef, indices: &[usize]) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let notes = &mut p.tracks[at.track].clips[at.clip].notes;
            if let Some(&note) = indices.iter().find(|&&i| i >= notes.len()) {
                return Err(EditError::NoNote { clip: at, note });
            }
            let mut remove = vec![false; notes.len()];
            for &i in indices {
                remove[i] = true;
            }
            let mut flags = remove.into_iter();
            notes.retain(|_| !flags.next().unwrap_or(false));
            Ok(())
        })
        .map(|((), c)| c)
    }

    /// Moves notes up or down by `semitones`, clamped to the MIDI range. `None` means every note
    /// in the clip.
    pub fn transpose_notes(
        &mut self,
        at: ClipRef,
        indices: Option<&[usize]>,
        semitones: i32,
    ) -> Result<Changes, EditError> {
        self.try_edit(|p| {
            check_clip(p, at)?;
            let notes = &mut p.tracks[at.track].clips[at.clip].notes;
            let shift = |n: &mut Note| {
                n.pitch = (i32::from(n.pitch) + semitones).clamp(0, 127) as u8;
            };
            match indices {
                None => notes.iter_mut().for_each(shift),
                Some(indices) => {
                    if let Some(&note) = indices.iter().find(|&&i| i >= notes.len()) {
                        return Err(EditError::NoNote { clip: at, note });
                    }
                    let mut seen = vec![false; notes.len()];
                    for &i in indices {
                        if !std::mem::replace(&mut seen[i], true) {
                            shift(&mut notes[i]);
                        }
                    }
                }
            }
            Ok(())
        })
        .map(|((), c)| c)
    }

    // ---- transport (not undo steps) ---------------------------------------------------------

    pub fn set_bpm(&mut self, bpm: f64) -> Result<Changes, EditError> {
        check_range("bpm", bpm, 20.0, 999.0)?;
        self.set(|p| p.bpm = bpm)
    }

    /// Sets the song's key (0 = C to 11 = B) and scale ([`crate::music::SCALES`]). Notes
    /// already written stay as they are.
    pub fn set_key(&mut self, key: u8, scale: &str) -> Result<Changes, EditError> {
        if key > 11 {
            return Err(EditError::Invalid(format!(
                "the key has to be 0 (C) to 11 (B), got {key}"
            )));
        }
        let Some(scale) = crate::music::SCALES.iter().find(|s| s.id == scale) else {
            let ids: Vec<_> = crate::music::SCALES.iter().map(|s| s.id).collect();
            return Err(EditError::Invalid(format!(
                "unknown scale \"{scale}\"; use one of {}",
                ids.join(", ")
            )));
        };
        self.set(|p| {
            p.key = key;
            p.scale = scale.id.into();
        })
    }

    pub fn set_master_volume(&mut self, volume: f32) -> Result<Changes, EditError> {
        check_range("master volume", f64::from(volume), 0.0, 1.5)?;
        self.set(|p| p.master_volume = volume)
    }

    /// Sets the loop. The end has to be after the start.
    pub fn set_loop(&mut self, enabled: bool, start: f64, end: f64) -> Result<Changes, EditError> {
        check_beat("loop start", start)?;
        check_beat("loop end", end)?;
        if end < start + 1.0 / 16.0 {
            return Err(EditError::Invalid(
                "the loop has to end at least 1/16 beat after it starts".into(),
            ));
        }
        self.set(|p| (p.loop_enabled, p.loop_start, p.loop_end) = (enabled, start, end))
    }
}

fn no_audio(clip: &Clip) -> Result<(), EditError> {
    if clip.is_audio() {
        Err(EditError::Invalid(
            "that's an audio clip; it has no notes".into(),
        ))
    } else {
        Ok(())
    }
}

fn check_track(p: &Project, index: usize) -> Result<(), EditError> {
    if index < p.tracks.len() {
        Ok(())
    } else {
        Err(EditError::NoTrack(index))
    }
}

fn check_effect(p: &Project, track: usize, effect: usize) -> Result<(), EditError> {
    check_track(p, track)?;
    if effect < p.tracks[track].effects.len() {
        Ok(())
    } else {
        Err(EditError::Invalid(format!(
            "track {track} has no effect {effect}"
        )))
    }
}

fn check_clip(p: &Project, at: ClipRef) -> Result<(), EditError> {
    check_track(p, at.track)?;
    if at.clip < p.tracks[at.track].clips.len() {
        Ok(())
    } else {
        Err(EditError::NoClip(at))
    }
}

fn check_range(what: &str, value: f64, lo: f64, hi: f64) -> Result<(), EditError> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(EditError::Invalid(format!(
            "{what} has to be between {lo} and {hi} (got {value})"
        )))
    }
}

fn check_beat(what: &str, value: f64) -> Result<(), EditError> {
    use crate::project::limits::MAX_BEATS;
    check_range(what, value, 0.0, MAX_BEATS)
}

/// The typed operations reject out-of-range values instead of clamping them, so a client finds
/// out its request wasn't what it meant.
fn check_clip_shape(clip: &Clip) -> Result<(), EditError> {
    use crate::project::limits::MAX_BEATS;
    check_beat("clip start", clip.start)?;
    check_range("clip length", clip.length, 1.0 / 16.0, MAX_BEATS)?;
    if clip.end() > MAX_BEATS {
        return Err(EditError::Invalid(format!(
            "the clip would end after beat {MAX_BEATS}"
        )));
    }
    Ok(())
}

fn check_note(note: &Note, clip_length: f64) -> Result<(), EditError> {
    use crate::project::limits::MAX_BEATS;
    if note.pitch > 127 {
        return Err(EditError::Invalid(format!(
            "pitch has to be 0 to 127 (got {})",
            note.pitch
        )));
    }
    if !(note.start.is_finite() && note.start >= 0.0 && note.start < clip_length) {
        return Err(EditError::Invalid(format!(
            "a note starts at beat {} of the clip, but the clip is {clip_length} beats long",
            note.start
        )));
    }
    check_range("note length", note.length, 1.0 / 128.0, MAX_BEATS)?;
    check_range("velocity", f64::from(note.velocity), 0.0, 1.0)
}

#[cfg(test)]
mod drum_tests {
    use super::*;

    #[test]
    fn a_beat_makes_a_drum_track_with_its_kit_and_tempo() {
        let mut s = Session::new(Project::default()).unwrap();
        let ((at, tempo_set), changes) = s.add_drum_beat(None, "trap-classic", 2, None).unwrap();
        assert!(
            tempo_set && changes.reload,
            "the rebuilt engine carries the tempo"
        );
        let p = s.project();
        assert_eq!(p.bpm, 140.0);
        assert_eq!(p.tracks[at.track].name, "Drums");
        let device = p.tracks[at.track].instrument.as_ref().unwrap();
        assert_eq!(device.builtin_id(), Some("drums"));
        assert_eq!(device.param("kit"), Some(5.0), "the trap kit");
        let clip = &p.tracks[at.track].clips[at.clip];
        assert_eq!((clip.start, clip.length), (0.0, 8.0));
        assert!(clip.notes.len() > 20);
        // One undo step takes the track and its clip back. The tempo stays: it's never an
        // undo step, like the other transport settings.
        s.undo().unwrap();
        assert!(s.project().tracks.is_empty());
        assert_eq!(s.project().bpm, 140.0);
    }

    #[test]
    fn beats_append_to_a_drum_track_and_leave_the_tempo_alone() {
        let mut s = Session::new(Project::default()).unwrap();
        let ((first, _), _) = s.add_drum_beat(None, "techno-basic", 1, None).unwrap();
        let ((second, tempo_set), _) = s
            .add_drum_beat(Some(first.track), "techno-driving", 2, None)
            .unwrap();
        assert!(!tempo_set);
        assert_eq!(s.project().bpm, 130.0);
        let clips = &s.project().tracks[0].clips;
        assert_eq!(
            (clips[0].start, clips[1].start, clips[1].length),
            (0.0, 4.0, 8.0)
        );
        assert_eq!(second.clip, 1);
        let ((third, _), _) = s
            .add_drum_beat(Some(0), "techno-minimal", 1, Some(32.0))
            .unwrap();
        assert_eq!(s.project().tracks[0].clips[third.clip].start, 32.0);
    }

    #[test]
    fn bad_beat_requests_are_refused() {
        let mut project = Project::default();
        project.tracks.push(Track::default());
        project.tracks.push(Track {
            instrument: Some(Device::builtin("sine")),
            ..Default::default()
        });
        let mut s = Session::new(project).unwrap();
        assert!(s.add_drum_beat(None, "nope", 1, None).is_err());
        assert!(s.add_drum_beat(None, "techno-basic", 0, None).is_err());
        assert!(s.add_drum_beat(None, "techno-basic", 65, None).is_err());
        assert!(
            s.add_drum_beat(None, "techno-basic", 1, Some(-1.0))
                .is_err()
        );
        assert!(s.add_drum_beat(Some(9), "techno-basic", 1, None).is_err());
        let m = s
            .add_drum_beat(Some(1), "techno-basic", 1, None)
            .unwrap_err();
        assert!(m.to_string().contains("drum rack"), "{m}");
        assert!(
            s.add_drum_beat(Some(0), "techno-basic", 1, None).is_err(),
            "no instrument"
        );
        assert_eq!(s.project().tracks.len(), 2, "nothing was added");
        assert!(!s.can_undo());
    }
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;

    fn clip(start: f64, length: f64) -> Clip {
        Clip {
            start,
            length,
            ..Default::default()
        }
    }

    fn session(clips: Vec<Clip>) -> Session {
        let mut project = Project::default();
        project.tracks.push(Track {
            clips,
            ..Default::default()
        });
        Session::new(project).unwrap()
    }

    #[test]
    fn duplicated_clips_never_land_on_other_clips() {
        let mut s = session(vec![clip(0.0, 4.0), clip(4.0, 4.0), clip(10.0, 2.0)]);
        let at = ClipRef { track: 0, clip: 0 };
        let (copy, _) = s.duplicate_clip(at).unwrap();
        let starts = |s: &Session| -> Vec<f64> {
            s.project().tracks[0]
                .clips
                .iter()
                .map(|c| c.start)
                .collect()
        };
        assert_eq!(starts(&s)[copy.clip], 12.0, "after the clips in the way");
        // Again from the original: it goes after the first copy, not on top of it.
        let (again, _) = s.duplicate_clip(at).unwrap();
        assert_eq!(starts(&s)[again.clip], 16.0);
        let clips = &s.project().tracks[0].clips;
        for (i, a) in clips.iter().enumerate() {
            for b in &clips[i + 1..] {
                assert!(a.end() <= b.start || b.end() <= a.start, "no overlap");
            }
        }
        // With room straight after, that's where it goes.
        let mut s = session(vec![clip(0.0, 4.0)]);
        let (copy, _) = s.duplicate_clip(at).unwrap();
        assert_eq!(s.project().tracks[0].clips[copy.clip].start, 4.0);
    }

    fn note(pitch: u8, start: f64, length: f64) -> Note {
        Note {
            pitch,
            start,
            length,
            velocity: 0.8,
        }
    }

    #[test]
    fn duplicated_notes_follow_the_selection() {
        let mut first = clip(0.0, 4.0);
        first.notes = vec![note(60, 0.0, 0.5), note(64, 0.5, 0.5), note(67, 3.0, 1.0)];
        let mut s = session(vec![first]);
        let at = ClipRef { track: 0, clip: 0 };
        let (copies, _) = s.duplicate_notes(at, &[1, 0, 0], 0.0).unwrap();
        assert_eq!(copies, vec![3, 4]);
        let notes = &s.project().tracks[0].clips[0].notes;
        assert_eq!((notes[3].pitch, notes[3].start), (60, 1.0));
        assert_eq!((notes[4].pitch, notes[4].start), (64, 1.5));
        assert_eq!(s.project().tracks[0].clips[0].length, 4.0, "it fitted");

        // Duplicating everything doubles the clip.
        let all: Vec<usize> = (0..5).collect();
        let (copies, _) = s.duplicate_notes(at, &all, 0.0).unwrap();
        assert_eq!(copies.len(), 5);
        assert_eq!(s.project().tracks[0].clips[0].length, 8.0);
        s.undo().unwrap();
        assert_eq!(s.project().tracks[0].clips[0].length, 4.0);
        assert_eq!(s.project().tracks[0].clips[0].notes.len(), 5);

        assert!(s.duplicate_notes(at, &[99], 0.0).is_err());
        assert_eq!(
            s.duplicate_notes(at, &[], 0.0).unwrap().0,
            Vec::<usize>::new()
        );
    }

    #[test]
    fn duplicated_notes_stay_on_the_grid() {
        let mut first = clip(0.0, 8.0);
        // A little short of a beat each, as played or generated notes often are.
        first.notes = vec![note(60, 0.5, 0.45), note(64, 1.0, 0.95)];
        let mut s = session(vec![first]);
        let at = ClipRef { track: 0, clip: 0 };
        s.duplicate_notes(at, &[0, 1], 0.5).unwrap();
        let notes = &s.project().tracks[0].clips[0].notes;
        assert_eq!(
            (notes[2].start, notes[3].start),
            (2.0, 2.5),
            "1.45 rounds up to 1.5"
        );
        // An exact fit isn't pushed a step further.
        s.duplicate_notes(at, &[1], 1.0).unwrap();
        let notes = &s.project().tracks[0].clips[0].notes;
        assert_eq!(notes[4].start, 2.0);
    }

    #[test]
    fn a_clip_only_grows_into_free_space() {
        let mut first = clip(0.0, 4.0);
        first.notes = vec![note(60, 0.0, 1.0), note(62, 3.0, 1.0)];
        let mut s = session(vec![first, clip(6.0, 4.0)]);
        let at = ClipRef { track: 0, clip: 0 };
        let (copies, _) = s.duplicate_notes(at, &[0, 1], 0.0).unwrap();
        let clip = &s.project().tracks[0].clips[0];
        assert_eq!(clip.length, 6.0, "up to the next clip");
        assert_eq!(
            copies.len(),
            1,
            "the copy that would start past the end is left out"
        );
        assert_eq!(clip.notes[2].start, 4.0);
    }

    #[test]
    fn the_key_is_a_setting_not_an_undo_step() {
        let mut s = session(vec![]);
        s.set_key(9, "minor").unwrap();
        assert_eq!((s.project().key, s.project().scale.as_str()), (9, "minor"));
        assert!(!s.can_undo());
        assert!(s.set_key(12, "minor").is_err());
        assert!(s.set_key(0, "wobbly").is_err());
        let json = s
            .project()
            .to_json()
            .replace("\"minor\"", "\"nope\"")
            .replace("\"key\": 9", "\"key\": 250");
        let p = Project::from_json(&json).unwrap();
        assert_eq!((p.key, p.scale.as_str()), (250 % 12, "major"));
    }
}

#[cfg(test)]
mod instrument_tests {
    use super::*;
    use crate::demo;

    fn operator_session() -> Session {
        let mut s = Session::new(demo::project()).unwrap();
        let changes = s
            .set_instrument(0, Some(Device::builtin("operator")))
            .unwrap();
        assert!(changes.reload, "a new instrument rebuilds the engine");
        s
    }

    #[test]
    fn instrument_params_change_live_and_undo() {
        let mut s = operator_session();
        let changes = s.set_instrument_param(0, "opBLevel", 0.5).unwrap();
        assert!(!changes.reload);
        let index = crate::operator::PARAMS
            .iter()
            .position(|p| p.name == "opBLevel")
            .unwrap();
        assert_eq!(changes.instrument_params, vec![(0, index, 0.5)]);
        let device = |s: &Session| s.project().tracks[0].instrument.clone().unwrap();
        assert_eq!(device(&s).param("opBLevel"), Some(0.5));

        // Stepped parameters are stored whole.
        s.set_instrument_param(0, "algorithm", 2.6).unwrap();
        assert_eq!(device(&s).param("algorithm"), Some(3.0));

        assert!(s.set_instrument_param(0, "opBLevel", 7.0).is_err());
        assert!(s.set_instrument_param(0, "wobble", 0.5).is_err());
        assert!(s.set_instrument_param(9, "volume", 0.5).is_err());
        let kick = s.project().tracks.len() - 1;
        assert!(
            s.set_instrument_param(kick, "volume", 0.5).is_err(),
            "Kick Me has no parameters"
        );

        let changes = s.undo().unwrap();
        assert_eq!(changes.instrument_params.len(), 1, "undo is heard live too");
        assert_eq!(device(&s).param("algorithm"), Some(0.0));
    }

    #[test]
    fn a_preset_replaces_every_setting_in_one_step() {
        let mut s = operator_session();
        s.set_instrument_param(0, "opDLevel", 0.9).unwrap();
        let changes = s.set_instrument_preset(0, "Kick").unwrap();
        assert!(!changes.reload);
        let device = s.project().tracks[0].instrument.clone().unwrap();
        assert_eq!(device.param("pitchEnvAmount"), Some(36.0));
        assert_eq!(device.param("opDLevel"), Some(0.0), "back to the default");
        assert!(
            changes.instrument_params.len() > 5,
            "every changed parameter is sent"
        );
        assert!(s.set_instrument_preset(0, "Tuba").is_err());
        let kick = s.project().tracks.len() - 1;
        assert!(s.set_instrument_preset(kick, "Init").is_err());
        s.undo().unwrap();
        let device = s.project().tracks[0].instrument.clone().unwrap();
        assert_eq!(device.param("opDLevel"), Some(0.9));
    }

    #[test]
    fn operator_settings_survive_a_save_and_are_clamped_on_load() {
        let mut s = operator_session();
        s.set_instrument_preset(0, "FM Bass").unwrap();
        let reloaded = Project::from_json(&s.project().to_json()).unwrap();
        assert_eq!(
            reloaded.tracks[0].instrument,
            s.project().tracks[0].instrument
        );

        let json = s
            .project()
            .to_json()
            .replace("\"cutoff\": 1600.0", "\"cutoff\": 1e9, \"bogus\": 1.0");
        let odd = Project::from_json(&json).unwrap();
        let device = odd.tracks[0].instrument.clone().unwrap();
        assert_eq!(device.param("cutoff"), Some(20_000.0));
        assert_eq!(device.param("bogus"), None);
    }

    #[test]
    fn the_engine_plays_operator_with_its_saved_settings() {
        use crate::Engine;
        let render = |preset: &str| {
            let mut s = operator_session();
            s.set_instrument_preset(0, preset).unwrap();
            let mut engine = Engine::new(s.project().clone(), 48_000.0, 512).unwrap();
            assert!(engine.warnings().is_empty(), "{:?}", engine.warnings());
            engine.play();
            engine.render(48_000).0
        };
        let (init, bass) = (render("Init"), render("FM Bass"));
        assert!(init.iter().any(|x| x.abs() > 0.01), "Operator is heard");
        assert_ne!(init, bass, "the preset changes the sound");

        // A live change reaches the instrument.
        let s = operator_session();
        let mut engine = Engine::new(s.project().clone(), 48_000.0, 512).unwrap();
        let mut quiet = Engine::new(s.project().clone(), 48_000.0, 512).unwrap();
        quiet.set_instrument_param(0, 1, 0.0); // volume
        quiet.set_instrument_param(99, 1, 0.0); // no such track: ignored
        for e in [&mut engine, &mut quiet] {
            for t in 1..s.project().tracks.len() {
                e.set_track_mix(
                    t,
                    TrackMix {
                        volume: 0.0,
                        pan: 0.0,
                        mute: true,
                        solo: false,
                    },
                );
            }
            e.play();
        }
        let (loud, silent) = (engine.render(24_000).0, quiet.render(24_000).0);
        assert!(loud.iter().any(|x| x.abs() > 0.01));
        assert!(silent.iter().all(|x| x.abs() < 1e-6), "volume 0 is silent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn note(pitch: u8, start: f64) -> Note {
        Note {
            pitch,
            start,
            length: 1.0,
            velocity: 0.8,
        }
    }

    fn session() -> Session {
        Session::new(demo::project()).unwrap()
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut s = session();
        let original = s.project().clone();
        let (index, changes) = s.add_track(Track::default()).unwrap();
        assert_eq!(index, original.tracks.len());
        assert!(changes.reload && s.is_dirty());
        let undone = s.undo().unwrap();
        assert!(undone.reload);
        assert_eq!(*s.project(), original);
        s.redo().unwrap();
        assert_eq!(s.project().tracks.len(), original.tracks.len() + 1);
        assert!(!s.can_redo());
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut s = session();
        s.rename_track(0, "A").unwrap();
        s.undo();
        assert!(s.can_redo());
        s.rename_track(0, "B").unwrap();
        assert!(!s.can_redo());
    }

    #[test]
    fn history_is_bounded() {
        let mut s = session();
        for i in 0..MAX_UNDO_STEPS + 20 {
            s.rename_track(0, &format!("T{i}")).unwrap();
        }
        assert_eq!(s.undo.len(), MAX_UNDO_STEPS);
    }

    #[test]
    fn device_changes_are_undo_steps() {
        let mut s = session();
        let before = s.project().tracks[0].instrument.clone();
        let changes = s.set_instrument(0, Some(Device::builtin("kick"))).unwrap();
        assert!(changes.reload);
        s.undo();
        assert_eq!(s.project().tracks[0].instrument, before);
    }

    #[test]
    fn nothing_changed_is_not_an_undo_step() {
        let mut s = session();
        let name = s.project().tracks[0].name.clone();
        let changes = s.rename_track(0, &name).unwrap();
        assert!(changes.is_empty());
        assert!(!s.can_undo() && !s.is_dirty());
    }

    #[test]
    fn note_edits_only_resend_that_track() {
        let mut s = session();
        let at = ClipRef { track: 1, clip: 0 };
        let (_, changes) = s.add_notes(at, &[note(40, 1.0)]).unwrap();
        assert_eq!(
            changes,
            Changes {
                clips: vec![1],
                ..Default::default()
            }
        );
    }

    #[test]
    fn mixer_and_tempo_are_not_undo_steps_and_survive_undo() {
        let mut s = session();
        s.rename_track(0, "Renamed").unwrap();
        s.set_bpm(140.0).unwrap();
        let mix = TrackMix {
            volume: 0.1,
            ..mix_of(&s.project().tracks[0])
        };
        let changes = s.set_track_mix(0, mix).unwrap();
        assert_eq!(changes.mix, vec![0]);
        s.undo();
        assert_ne!(s.project().tracks[0].name, "Renamed");
        assert_eq!(
            (s.project().bpm, s.project().tracks[0].volume),
            (140.0, 0.1)
        );
        assert!(!s.can_undo(), "only the rename was an undo step");
    }

    #[test]
    fn a_gesture_is_one_undo_step() {
        let mut s = session();
        let original = s.project().clone();
        let at = ClipRef { track: 0, clip: 0 };
        s.begin_gesture();
        for i in 1..=5 {
            s.edit(|p| p.tracks[0].clips[0].start = f64::from(i))
                .unwrap();
        }
        s.end_gesture();
        assert_eq!(s.clip(at).unwrap().start, 5.0);
        s.undo();
        assert_eq!(*s.project(), original);
        assert!(!s.can_undo());
    }

    #[test]
    fn undo_mid_gesture_ends_it_first() {
        let mut s = session();
        let original = s.project().clone();
        s.begin_gesture();
        s.edit(|p| p.tracks[0].clips[0].start = 3.0).unwrap();
        assert!(s.can_undo());
        s.undo();
        assert!(!s.in_gesture());
        assert_eq!(*s.project(), original);
    }

    #[test]
    fn bad_references_and_values_are_refused_without_changes() {
        let mut s = session();
        let original = s.project().clone();
        let tracks = original.tracks.len();
        assert_eq!(s.remove_track(tracks), Err(EditError::NoTrack(tracks)));
        let missing = ClipRef {
            track: 0,
            clip: 999,
        };
        assert_eq!(s.remove_clip(missing), Err(EditError::NoClip(missing)));
        let at = ClipRef { track: 0, clip: 0 };
        let length = s.clip(at).unwrap().length;
        assert!(matches!(
            s.add_notes(at, &[note(60, 0.0), note(60, length)]),
            Err(EditError::Invalid(_))
        ));
        assert!(matches!(
            s.remove_notes(at, &[0, 10_000]),
            Err(EditError::NoNote { note: 10_000, .. })
        ));
        assert!(s.set_bpm(f64::NAN).is_err());
        assert!(s.set_bpm(5.0).is_err());
        assert!(s.set_loop(true, 8.0, 4.0).is_err());
        assert!(
            s.update_clip(at, |c| c.length = f64::INFINITY).is_err(),
            "the closure's change is rolled back"
        );
        assert_eq!(*s.project(), original);
        assert!(!s.can_undo() && !s.is_dirty());
    }

    #[test]
    fn limits_roll_the_edit_back() {
        let mut s = Session::new(Project::default()).unwrap();
        let result = s.edit(|p| {
            p.tracks = vec![Track::default(); crate::project::limits::MAX_TRACKS + 1];
        });
        assert!(matches!(result, Err(EditError::Limit(_))));
        assert!(s.project().tracks.is_empty() && !s.can_undo());
    }

    #[test]
    fn clips_move_between_tracks() {
        let mut s = session();
        let at = ClipRef { track: 0, clip: 0 };
        let clip = s.clip(at).unwrap().clone();
        let before_on_1 = s.project().tracks[1].clips.len();
        let (moved, changes) = s.move_clip(at, 1, 32.0).unwrap();
        assert_eq!(moved.track, 1);
        assert_eq!(moved.clip, before_on_1);
        let now = s.clip(moved).unwrap();
        assert_eq!((now.start, &now.notes), (32.0, &clip.notes));
        assert_eq!(changes.clips, vec![0, 1]);
    }

    #[test]
    fn duplicate_goes_straight_after() {
        let mut s = session();
        let at = ClipRef { track: 0, clip: 0 };
        let end = s.clip(at).unwrap().end();
        let (copy, _) = s.duplicate_clip(at).unwrap();
        assert_eq!(s.clip(copy).unwrap().start, end);
    }

    #[test]
    fn remove_and_transpose_notes_by_index() {
        let mut s = session();
        let (at, _) = s
            .add_clip(
                0,
                Clip {
                    start: 64.0,
                    ..Default::default()
                },
            )
            .unwrap();
        s.set_notes(at, vec![note(60, 0.0), note(62, 1.0), note(64, 2.0)])
            .unwrap();
        s.transpose_notes(at, Some(&[0, 0, 2]), 12).unwrap();
        let pitches = |s: &Session| {
            s.clip(at)
                .unwrap()
                .notes
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>()
        };
        assert_eq!(pitches(&s), vec![72, 62, 76], "a repeated index moves once");
        s.transpose_notes(at, None, 100).unwrap();
        assert_eq!(pitches(&s), vec![127, 127, 127]);
        s.remove_notes(at, &[1, 1]).unwrap();
        assert_eq!(s.clip(at).unwrap().notes.len(), 2);
    }

    #[test]
    fn mix_and_transport_changes_are_detected() {
        let before = demo::project();
        let mut after = before.clone();
        after.tracks[0].mute = true;
        after.bpm = 100.0;
        after.loop_enabled = true;
        let c = changes(&before, &after);
        assert_eq!(
            (c.mix, c.bpm, c.looping, c.reload),
            (vec![0], true, true, false)
        );
    }

    #[test]
    fn audio_clips_take_no_notes() {
        let mut s = session();
        let audio = Clip {
            start: 80.0,
            audio: Some(crate::project::AudioRegion {
                file: "a.wav".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let with_notes = Clip {
            notes: vec![note(60, 0.0)],
            ..audio.clone()
        };
        assert!(s.add_clip(0, with_notes).is_err());
        let (at, _) = s.add_clip(0, audio).unwrap();
        assert!(s.add_notes(at, &[note(60, 0.0)]).is_err());
        assert!(s.set_notes(at, vec![note(60, 0.0)]).is_err());
        assert!(s.set_notes(at, Vec::new()).is_err());
    }

    #[test]
    fn effects_add_set_and_remove_with_undo() {
        let mut s = session();
        assert!(s.add_effect(0, "flanger9000").is_err());
        let (index, changes) = s.add_effect(0, "delay").unwrap();
        assert_eq!(index, 0);
        assert!(changes.reload, "a new effect rebuilds the engine");
        let changes = s.set_effect_param(0, 0, "mix", 0.9).unwrap();
        assert!(!changes.reload, "a parameter change is heard live");
        assert_eq!(changes.effect_params, vec![(0, 0)]);
        assert_eq!(s.project().tracks[0].effects[0].param("mix"), Some(0.9));
        assert!(
            s.set_effect_param(0, 0, "mix", 7.0).is_err(),
            "out of range"
        );
        assert!(s.set_effect_param(0, 0, "wobble", 0.5).is_err());
        assert!(s.set_effect_param(0, 3, "mix", 0.5).is_err());

        s.begin_gesture();
        for v in [0.1, 0.2, 0.3] {
            s.set_effect_param(0, 0, "feedback", v).unwrap();
        }
        s.end_gesture();
        s.undo();
        assert_eq!(
            s.project().tracks[0].effects[0].param("feedback"),
            Some(0.45),
            "a whole drag undoes in one step"
        );
        s.undo();
        assert_eq!(s.project().tracks[0].effects[0].param("mix"), Some(0.35));
        assert!(s.remove_effect(0, 0).unwrap().reload);
        assert!(s.project().tracks[0].effects.is_empty());
        s.undo();
        assert_eq!(s.project().tracks[0].effects.len(), 1, "removal undoes");
    }

    #[test]
    fn saving_clears_dirty() {
        let mut s = session();
        s.rename_track(0, "X").unwrap();
        assert!(s.is_dirty());
        s.mark_saved();
        assert!(!s.is_dirty());
    }
}
