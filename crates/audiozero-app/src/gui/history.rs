//! Undo and redo, and working out what the audio thread needs to hear after a change.

use audiozero_engine::Project;

/// How many steps can be undone.
const MAX_STEPS: usize = 100;

/// Whole-project snapshots. Simple and impossible to get out of step with the project; the cost
/// is memory, which `MAX_STEPS` bounds.
#[derive(Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl History {
    /// Records the project as it was before an edit.
    pub fn push(&mut self, before: Project) {
        self.undo.push(before);
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Returns the project to go back to, remembering `current` for redo.
    pub fn undo(&mut self, current: &Project) -> Option<Project> {
        let previous = self.undo.pop()?;
        self.redo.push(current.clone());
        Some(previous)
    }

    pub fn redo(&mut self, current: &Project) -> Option<Project> {
        let next = self.redo.pop()?;
        self.undo.push(current.clone());
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Copies the settings that aren't undo steps (tempo, loop, master volume and each track's mix)
/// from `current` into a snapshot being restored, so undoing a note edit doesn't also move a
/// fader back. Tracks are matched by position.
pub fn keep_unrecorded(current: &Project, restored: &mut Project) {
    restored.bpm = current.bpm;
    restored.master_volume = current.master_volume;
    restored.loop_enabled = current.loop_enabled;
    restored.loop_start = current.loop_start;
    restored.loop_end = current.loop_end;
    for (to, from) in restored.tracks.iter_mut().zip(&current.tracks) {
        (to.volume, to.pan, to.mute, to.solo) = (from.volume, from.pan, from.mute, from.solo);
    }
}

/// What has to be sent to the audio thread to bring it from `before` to `after`.
#[derive(Debug, Default, PartialEq)]
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
            .filter(|(_, (a, b))| {
                (a.volume, a.pan, a.mute, a.solo) != (b.volume, b.pan, b.mute, b.solo)
            })
            .map(|(i, _)| i)
            .collect(),
        bpm: before.bpm != after.bpm,
        master: before.master_volume != after.master_volume,
        looping: (before.loop_enabled, before.loop_start, before.loop_end)
            != (after.loop_enabled, after.loop_start, after.loop_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audiozero_engine::{Note, Track, demo};

    #[test]
    fn undo_and_redo_walk_the_snapshots() {
        let mut history = History::default();
        let a = demo::project();
        let mut b = a.clone();
        b.bpm = 90.0;
        history.push(a.clone());
        assert_eq!(history.undo(&b), Some(a.clone()));
        assert_eq!(history.redo(&a), Some(b.clone()));
        assert!(history.can_undo() && !history.can_redo());
        history.push(b);
        assert!(!history.can_redo(), "a new edit clears redo");
    }

    #[test]
    fn history_is_bounded() {
        let mut history = History::default();
        for _ in 0..MAX_STEPS + 20 {
            history.push(Project::default());
        }
        assert_eq!(history.undo.len(), MAX_STEPS);
    }

    #[test]
    fn note_edits_only_resend_that_track() {
        let before = demo::project();
        let mut after = before.clone();
        after.tracks[1].clips[0].notes.push(Note {
            pitch: 40,
            start: 1.0,
            length: 1.0,
            velocity: 0.7,
        });
        assert_eq!(
            changes(&before, &after),
            Changes {
                clips: vec![1],
                ..Default::default()
            }
        );
    }

    #[test]
    fn undo_keeps_the_current_mix_and_tempo() {
        let current = {
            let mut p = demo::project();
            p.bpm = 140.0;
            p.tracks[0].volume = 0.1;
            p
        };
        let mut restored = demo::project();
        restored.tracks[0].clips.clear();
        keep_unrecorded(&current, &mut restored);
        assert_eq!((restored.bpm, restored.tracks[0].volume), (140.0, 0.1));
        assert!(
            restored.tracks[0].clips.is_empty(),
            "the clips still come from the snapshot"
        );
    }

    #[test]
    fn adding_a_track_rebuilds_the_engine() {
        let before = demo::project();
        let mut after = before.clone();
        after.tracks.push(Track::default());
        assert!(changes(&before, &after).reload);
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
}
