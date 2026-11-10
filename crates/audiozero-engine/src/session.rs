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
        || before
            .tracks
            .iter()
            .zip(&after.tracks)
            .any(|(a, b)| a.instrument != b.instrument || a.effects != b.effects);
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
            copy.start = copy.end();
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
        let changes = s
            .set_instrument(0, Some(Device::Builtin { id: "kick".into() }))
            .unwrap();
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
    fn saving_clears_dirty() {
        let mut s = session();
        s.rename_track(0, "X").unwrap();
        assert!(s.is_dirty());
        s.mark_saved();
        assert!(!s.is_dirty());
    }
}
