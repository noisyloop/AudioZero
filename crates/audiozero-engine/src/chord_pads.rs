//! The chord pads instrument: a polyphonic sound made for chords (pads, keys, plucks, stabs),
//! with the settings the chord pads and generators use: voicing, how many notes a pad adds,
//! a bass note and the register.
//!
//! It plays ordinary notes, so a chord clip on it is edited like any other. Under the hood it is
//! [Soundgineer](crate::soundgineer), set to one of fourteen of its presets that work well for
//! chords. Switching the sound on the audio thread is a copy of a prepared set of settings: it
//! doesn't allocate.
//!
//! The voicing, extension, bass and register settings aren't used by the sound itself. The
//! window's chord pads, the computer keyboard and the progression and random-chord menus read them,
//! so what you play and what's generated follow the same settings and are saved with the song.

use crate::effects::{Kind, Param};
use crate::soundgineer::{PARAM_COUNT, Soundgineer};
use crate::synth::Instrument;

/// The sounds: the name shown, and the Soundgineer preset behind it.
pub const SOUNDS: [(&str, &str); 14] = [
    ("Warm pad", "Warm Analog Pad"),
    ("Strings", "Velvet Strings"),
    ("Choir", "Choir Pad"),
    ("Glass pad", "Glass Pad"),
    ("Dark pad", "Dark Matter"),
    ("Rhodes", "Phased Rhodes"),
    ("Lo-fi keys", "Lo-Fi EP"),
    ("Wurly", "Wurly Grit"),
    ("Organ", "Drawbar Organ"),
    ("Rave stab", "Rave Stab"),
    ("Clav", "Funk Clav"),
    ("Pluck", "Arp Nights"),
    ("Ice pluck", "Ice Pluck"),
    ("Kalimba", "Kalimba"),
];

const SOUND_NAMES: &[&str] = &[
    "Warm pad",
    "Strings",
    "Choir",
    "Glass pad",
    "Dark pad",
    "Rhodes",
    "Lo-fi keys",
    "Wurly",
    "Organ",
    "Rave stab",
    "Clav",
    "Pluck",
    "Ice pluck",
    "Kalimba",
];
const VOICING_NAMES: &[&str] = &["Root position", "Smooth", "Open"];
const EXTENSION_NAMES: &[&str] = &["Triads", "Sevenths", "Ninths"];

/// The index of a sound by name.
pub fn sound_index(name: &str) -> Option<usize> {
    SOUNDS.iter().position(|s| s.0 == name)
}

const fn p(
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    kind: Kind,
) -> Param {
    Param {
        name,
        label,
        min,
        max,
        default,
        unit: "",
        kind,
    }
}

pub static PARAMS: [Param; 6] = [
    p("sound", "Sound", 0.0, 13.0, 0.0, Kind::Choice(SOUND_NAMES)),
    p("volume", "Volume", 0.0, 1.5, 0.7, Kind::Linear),
    p(
        "voicing",
        "Voicing",
        0.0,
        2.0,
        1.0,
        Kind::Choice(VOICING_NAMES),
    ),
    p(
        "extension",
        "Pad chords",
        0.0,
        2.0,
        0.0,
        Kind::Choice(EXTENSION_NAMES),
    ),
    p("bass", "Bass note", 0.0, 1.0, 0.0, Kind::Toggle),
    p("register", "Register", 48.0, 72.0, 60.0, Kind::Whole),
];

pub const SECTIONS: [(&str, std::ops::Range<usize>); 2] = [("Sound", 0..2), ("Chords", 2..6)];

/// The presets are the sounds.
pub const PRESETS: [&str; 14] = [
    "Warm pad",
    "Strings",
    "Choir",
    "Glass pad",
    "Dark pad",
    "Rhodes",
    "Lo-fi keys",
    "Wurly",
    "Organ",
    "Rave stab",
    "Clav",
    "Pluck",
    "Ice pluck",
    "Kalimba",
];

static PRESET_VALUES: [[(&str, f32); 1]; 14] = [
    [("sound", 0.0)],
    [("sound", 1.0)],
    [("sound", 2.0)],
    [("sound", 3.0)],
    [("sound", 4.0)],
    [("sound", 5.0)],
    [("sound", 6.0)],
    [("sound", 7.0)],
    [("sound", 8.0)],
    [("sound", 9.0)],
    [("sound", 10.0)],
    [("sound", 11.0)],
    [("sound", 12.0)],
    [("sound", 13.0)],
];

pub fn preset(name: &str) -> Option<&'static [(&'static str, f32)]> {
    PRESETS
        .iter()
        .position(|n| *n == name)
        .map(|i| &PRESET_VALUES[i][..])
}

pub struct ChordPads {
    synth: Soundgineer,
    /// Every sound's full set of Soundgineer settings, made when the instrument is.
    sounds: Vec<[f32; PARAM_COUNT]>,
    values: [f32; PARAMS.len()],
    master: usize,
}

impl ChordPads {
    pub fn new() -> Self {
        let mut pads = Self {
            synth: Soundgineer::new(),
            sounds: SOUNDS
                .iter()
                .map(|s| *Soundgineer::preset_values(s.1))
                .collect(),
            values: std::array::from_fn(|i| PARAMS[i].default),
            master: Soundgineer::param_index("master.volume"),
        };
        pads.apply_sound();
        pads
    }

    fn apply_sound(&mut self) {
        let sound = (self.values[0].round() as usize).min(self.sounds.len() - 1);
        self.synth.load_values(&self.sounds[sound]);
        self.synth.set_param(self.master, self.values[1]);
    }
}

impl Default for ChordPads {
    fn default() -> Self {
        Self::new()
    }
}

impl Instrument for ChordPads {
    fn prepare(&mut self, sample_rate: f64) {
        self.synth.prepare(sample_rate);
    }

    fn note_on(&mut self, pitch: u8, velocity: f32) {
        self.synth.note_on(pitch, velocity);
    }

    fn note_off(&mut self, pitch: u8) {
        self.synth.note_off(pitch);
    }

    fn all_notes_off(&mut self) {
        self.synth.all_notes_off();
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(param) = PARAMS.get(index) else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        self.values[index] = param.fit(value);
        match index {
            0 => self.apply_sound(),
            1 => self.synth.set_param(self.master, self.values[1]),
            _ => {}
        }
    }

    fn set_tempo(&mut self, bpm: f64) {
        self.synth.set_tempo(bpm);
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.synth.render(left, right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(pads: &mut ChordPads, seconds: f64) -> (Vec<f32>, Vec<f32>) {
        let frames = (seconds * 48_000.0) as usize;
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        for (l, r) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            pads.render(l, r);
        }
        (l, r)
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    #[test]
    fn every_sound_plays_a_chord_cleanly() {
        for (i, (name, preset)) in SOUNDS.iter().enumerate() {
            assert!(crate::soundgineer::preset(preset).is_some(), "{name}");
            let mut pads = ChordPads::new();
            pads.prepare(48_000.0);
            pads.set_param(0, i as f32);
            for note in [48, 55, 60, 64, 67] {
                pads.note_on(note, 0.8);
            }
            let (l, r) = render(&mut pads, 0.6);
            assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{name}");
            let level = peak(&l).max(peak(&r));
            assert!((0.01..2.01).contains(&level), "{name} peaks at {level}");
            pads.all_notes_off();
            render(&mut pads, 0.2);
        }
    }

    #[test]
    fn the_sound_and_volume_change_what_you_hear() {
        let level = |sound: f32, volume: f32| -> f32 {
            let mut pads = ChordPads::new();
            pads.prepare(48_000.0);
            pads.set_param(0, sound);
            pads.set_param(1, volume);
            pads.note_on(60, 0.9);
            let (l, _) = render(&mut pads, 0.4);
            peak(&l)
        };
        assert!(level(0.0, 1.2) > level(0.0, 0.3) * 2.0, "volume");
        assert!((level(0.0, 0.7) - level(11.0, 0.7)).abs() > 1e-3, "sound");
        // Switching the sound while it plays keeps it finite and keeps going.
        let mut pads = ChordPads::new();
        pads.prepare(48_000.0);
        pads.note_on(60, 0.9);
        render(&mut pads, 0.1);
        for s in 0..14 {
            pads.set_param(0, s as f32);
            let (l, _) = render(&mut pads, 0.05);
            assert!(l.iter().all(|x| x.is_finite()));
        }
        // Out-of-range and not-a-number settings are ignored or fitted.
        pads.set_param(0, f32::NAN);
        pads.set_param(99, 1.0);
        pads.set_param(0, 99.0);
    }

    #[test]
    fn the_presets_are_the_sounds() {
        for (i, name) in PRESETS.iter().enumerate() {
            assert_eq!(*name, SOUNDS[i].0);
            assert_eq!(preset(name), Some(&[("sound", i as f32)][..]));
        }
        assert_eq!(preset("Nope"), None);
        let mut covered: Vec<usize> = SECTIONS.iter().flat_map(|(_, r)| r.clone()).collect();
        covered.sort_unstable();
        assert_eq!(covered, (0..PARAMS.len()).collect::<Vec<_>>());
    }
}
