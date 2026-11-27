//! What the player can change about the controller, kept as a small JSON file.

use crate::input::Btn;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Largest profile file that's read.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// How many buttons can play notes.
pub const MAX_NOTE_BUTTONS: usize = 16;

/// The default note buttons, from the root up: the four face buttons, then the D-pad, and the
/// D-pad's last button is the root an octave up.
pub const DEFAULT_NOTE_BUTTONS: [Btn; 8] = [
    Btn::Cross,
    Btn::Circle,
    Btn::Square,
    Btn::Triangle,
    Btn::DpadLeft,
    Btn::DpadUp,
    Btn::DpadRight,
    Btn::DpadDown,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Whether the controller plays at all.
    pub enabled: bool,
    /// The MIDI channel (1 to 16) its messages are shown on in the monitor.
    pub channel: u8,
    /// The octave of the root, as in MIDI (4 = middle C = 60), like the computer keyboard.
    pub octave: i8,
    /// The velocity used when L2 isn't pulled, 0 to 1.
    pub velocity: f32,
    /// Stick travel (0 to 0.9) that counts as centred.
    pub dead_zone: f32,
    /// 0 for none up to 0.95 for very smooth (and slow) analog controls.
    pub smoothing: f32,
    /// The buttons that play the scale, from its root up. Position is the scale degree.
    pub note_buttons: Vec<Btn>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            enabled: true,
            channel: 1,
            octave: 4,
            velocity: 0.8,
            dead_zone: 0.12,
            smoothing: 0.3,
            note_buttons: DEFAULT_NOTE_BUTTONS.to_vec(),
        }
    }
}

impl Profile {
    /// Drops anything out of range, so a hand-edited file can't do harm.
    pub fn sanitized(mut self) -> Self {
        let finite = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        self.channel = self.channel.clamp(1, 16);
        self.octave = self.octave.clamp(-1, 8);
        self.velocity = finite(self.velocity, 0.05, 1.0, 0.8);
        self.dead_zone = finite(self.dead_zone, 0.0, 0.9, 0.12);
        self.smoothing = finite(self.smoothing, 0.0, 0.95, 0.3);
        // One button, one note: later repeats are dropped.
        let mut seen = Vec::new();
        for b in std::mem::take(&mut self.note_buttons) {
            if !seen.contains(&b) && seen.len() < MAX_NOTE_BUTTONS {
                seen.push(b);
            }
        }
        if seen.is_empty() {
            seen = DEFAULT_NOTE_BUTTONS.to_vec();
        }
        self.note_buttons = seen;
        self
    }

    /// Reads a profile from JSON. Anything that isn't a valid profile gives the defaults.
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str::<Self>(text)
            .unwrap_or_default()
            .sanitized()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a profile is plain data")
    }

    /// The saved profile, or the defaults if there is none (or it can't be read).
    pub fn load(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_BYTES => {}
            _ => return Self::default(),
        }
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |t| Self::from_json(&t))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        std::fs::write(path, self.to_json()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips() {
        let p = Profile {
            channel: 5,
            octave: 3,
            ..Profile::default()
        };
        assert_eq!(Profile::from_json(&p.to_json()), p);
    }

    #[test]
    fn garbage_and_out_of_range_values_are_tamed() {
        assert_eq!(Profile::from_json("not json"), Profile::default());
        let p = Profile::from_json(
            r#"{"channel":99,"octave":-40,"velocity":9,"dead_zone":2,"smoothing":5,"note_buttons":[]}"#,
        );
        assert_eq!((p.channel, p.octave), (16, -1));
        assert_eq!((p.velocity, p.dead_zone, p.smoothing), (1.0, 0.9, 0.95));
        assert_eq!(p.note_buttons, DEFAULT_NOTE_BUTTONS.to_vec());
    }

    #[test]
    fn duplicate_note_buttons_are_dropped() {
        let p = Profile {
            note_buttons: vec![Btn::Cross, Btn::Cross, Btn::Circle],
            ..Profile::default()
        }
        .sanitized();
        assert_eq!(p.note_buttons, vec![Btn::Cross, Btn::Circle]);
    }
}
