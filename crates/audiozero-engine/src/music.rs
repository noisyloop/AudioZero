//! Keys and scales, and a melody generator that stays in key.
//!
//! A song has a key (its root note, 0 = C) and a scale. The piano roll marks the notes in the
//! scale, can keep new notes on it, and can write a random melody in it.

use crate::project::Note;

/// A scale: the id projects store, its name, and its notes as semitones above the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    pub id: &'static str,
    pub name: &'static str,
    pub steps: &'static [u8],
}

const fn scale(id: &'static str, name: &'static str, steps: &'static [u8]) -> Scale {
    Scale { id, name, steps }
}

pub const SCALES: [Scale; 12] = [
    scale("major", "Major", &[0, 2, 4, 5, 7, 9, 11]),
    scale("minor", "Minor", &[0, 2, 3, 5, 7, 8, 10]),
    scale("dorian", "Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    scale("phrygian", "Phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    scale("lydian", "Lydian", &[0, 2, 4, 6, 7, 9, 11]),
    scale("mixolydian", "Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    scale("harmonic_minor", "Harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
    scale("melodic_minor", "Melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
    scale("major_pentatonic", "Major pentatonic", &[0, 2, 4, 7, 9]),
    scale("minor_pentatonic", "Minor pentatonic", &[0, 3, 5, 7, 10]),
    scale("blues", "Blues", &[0, 3, 5, 6, 7, 10]),
    scale(
        "chromatic",
        "Chromatic (all notes)",
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    ),
];

/// The twelve keys, by root note.
pub const KEYS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// The scale with this id, or major if there's no such scale.
pub fn scale_by_id(id: &str) -> Scale {
    SCALES
        .iter()
        .copied()
        .find(|s| s.id == id)
        .unwrap_or(SCALES[0])
}

/// "C minor", "F# Dorian".
pub fn key_name(key: u8, scale_id: &str) -> String {
    let scale = scale_by_id(scale_id);
    format!(
        "{} {}",
        KEYS[usize::from(key % 12)],
        scale.name.to_lowercase()
    )
}

/// Whether `pitch` is a note of the scale in `key`.
pub fn in_scale(pitch: u8, key: u8, scale: Scale) -> bool {
    let degree = (u16::from(pitch) + 12 - u16::from(key % 12)) % 12;
    scale.steps.iter().any(|&s| u16::from(s) == degree)
}

/// The nearest pitch that's in the scale (the lower one when two are as near).
pub fn snap_to_scale(pitch: u8, key: u8, scale: Scale) -> u8 {
    let pitch = pitch.min(127);
    for distance in 0..12u8 {
        for candidate in [pitch.checked_sub(distance), pitch.checked_add(distance)] {
            if let Some(p) = candidate.filter(|&p| p <= 127 && in_scale(p, key, scale)) {
                return p;
            }
        }
    }
    pitch
}

/// Every pitch of the scale from `low` to `high`, lowest first.
pub fn scale_pitches(key: u8, scale: Scale, low: u8, high: u8) -> Vec<u8> {
    (low..=high.min(127))
        .filter(|&p| in_scale(p, key, scale))
        .collect()
}

/// A small random number generator (xorshift): the same seed gives the same melody.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Spread the seed's bits, and never start at zero (xorshift would stay there).
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn bits(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number from 0 up to (not including) `n`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            ((self.bits() >> 11) % n as u64) as usize
        }
    }

    /// A number from 0 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.bits() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[self.below(from.len())]
    }
}

/// What a random melody should be like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MelodySettings {
    pub key: u8,
    pub scale: Scale,
    /// How long the melody is, in beats.
    pub length: f64,
    /// The grid its notes sit on, in beats (for example 0.25 for sixteenths).
    pub step: f64,
    /// The pitch the melody stays around.
    pub centre: u8,
}

/// Writes a melody in key: one note at a time, on the grid, moving mostly by step with the odd
/// leap, starting on a note of the key's chord and ending on the root. The same seed and
/// settings always give the same melody.
pub fn melody(seed: u64, settings: MelodySettings) -> Vec<Note> {
    let mut rng = Rng::new(seed);
    let step = if settings.step.is_finite() && settings.step >= 1.0 / 32.0 {
        settings.step
    } else {
        0.25
    };
    let steps = (settings.length / step).floor() as usize;
    let centre = settings.centre.clamp(24, 103);
    let pitches = scale_pitches(settings.key, settings.scale, centre - 10, centre + 14);
    if steps == 0 || pitches.is_empty() {
        return Vec::new();
    }
    let root = |p: u8| (u16::from(p) + 12 - u16::from(settings.key % 12)) % 12 == 0;
    // Start on the root nearest the centre, or its third or fifth (two and four scale notes up).
    let home = pitches
        .iter()
        .position(|&p| root(p) && p + 6 >= centre)
        .or_else(|| pitches.iter().position(|&p| root(p)))
        .unwrap_or(pitches.len() / 2);
    let mut degree = (home + rng.pick(&[0, 0, 2, 4])).min(pitches.len() - 1);

    // How many grid steps make a beat, to favour notes on the beat.
    let per_beat = (1.0 / step).round().max(1.0) as usize;
    let mut notes: Vec<Note> = Vec::new();
    let mut at = 0;
    while at < steps {
        let on_beat = at % per_beat == 0;
        let chance = if at == 0 {
            1.0
        } else if on_beat {
            0.8
        } else {
            0.45
        };
        if rng.unit() > chance {
            at += 1;
            continue;
        }
        let length = rng.pick(&[1, 1, 1, 2, 2, 2, 3, 4]).min(steps - at);
        let movement: i32 = rng.pick(&[-3, -2, -1, -1, -1, 0, 1, 1, 1, 2, 3, 4, -4]);
        let moved = degree as i32 + movement;
        // Turn back at the edges of the range instead of sticking to them.
        degree = if moved < 0 || moved >= pitches.len() as i32 {
            (degree as i32 - movement).clamp(0, pitches.len() as i32 - 1) as usize
        } else {
            moved as usize
        };
        notes.push(Note {
            pitch: pitches[degree],
            start: at as f64 * step,
            // A little short of the full length, so repeated notes are heard apart.
            length: length as f64 * step * 0.95,
            velocity: if on_beat {
                0.8 + 0.15 * rng.unit()
            } else {
                0.55 + 0.25 * rng.unit()
            },
        });
        at += length;
    }
    // End at home.
    if let Some(last) = notes.last_mut() {
        last.pitch = pitches[home];
    }
    notes
}

/// New pitches in key for `notes`, keeping when they play and how long. Each note moves to a
/// scale note near where the melody already was.
pub fn repitch(seed: u64, notes: &mut [Note], key: u8, scale: Scale) {
    let mut rng = Rng::new(seed);
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| notes[a].start.total_cmp(&notes[b].start));
    let Some(&first) = order.first() else {
        return;
    };
    let centre = notes[first].pitch.clamp(12, 115);
    let pitches = scale_pitches(key, scale, centre - 12, centre + 12);
    if pitches.is_empty() {
        return;
    }
    let mut degree = pitches
        .iter()
        .position(|&p| p >= snap_to_scale(centre, key, scale))
        .unwrap_or(pitches.len() / 2);
    for index in order {
        let movement: i32 = rng.pick(&[-2, -1, -1, 0, 1, 1, 2, 3, -3]);
        degree = (degree as i32 + movement).clamp(0, pitches.len() as i32 - 1) as usize;
        notes[index].pitch = pitches[degree];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(key: u8, scale: &str) -> MelodySettings {
        MelodySettings {
            key,
            scale: scale_by_id(scale),
            length: 16.0,
            step: 0.25,
            centre: 64,
        }
    }

    #[test]
    fn scales_know_their_notes() {
        let major = scale_by_id("major");
        let c_major: Vec<u8> = scale_pitches(0, major, 60, 72);
        assert_eq!(c_major, vec![60, 62, 64, 65, 67, 69, 71, 72]);
        assert!(in_scale(66, 7, major), "F# is in G major");
        assert!(!in_scale(65, 7, major), "F isn't");
        assert_eq!(scale_by_id("nonsense"), major);
        assert_eq!(key_name(9, "minor"), "A minor");
        assert_eq!(key_name(6, "dorian"), "F# dorian");
        let mut ids: Vec<_> = SCALES.iter().map(|s| s.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), SCALES.len());
        for s in SCALES {
            assert_eq!(s.steps[0], 0, "{} starts on the root", s.id);
            assert!(s.steps.windows(2).all(|w| w[0] < w[1] && w[1] < 12));
        }
    }

    #[test]
    fn snapping_goes_to_the_nearest_scale_note() {
        let major = scale_by_id("major");
        assert_eq!(snap_to_scale(60, 0, major), 60);
        assert_eq!(snap_to_scale(61, 0, major), 60, "the lower of two as near");
        assert_eq!(snap_to_scale(66, 0, major), 65);
        assert_eq!(snap_to_scale(127, 0, major), 127);
        assert_eq!(
            snap_to_scale(0, 1, major),
            0,
            "C is in C# major (its seventh)"
        );
        let pentatonic = scale_by_id("minor_pentatonic");
        assert_eq!(snap_to_scale(62, 9, pentatonic), 62);
        assert_eq!(snap_to_scale(63, 9, pentatonic), 62);
    }

    #[test]
    fn melodies_stay_in_key_on_the_grid_and_inside_the_clip() {
        for seed in 0..200 {
            for (key, scale) in [
                (0, "major"),
                (9, "minor"),
                (6, "blues"),
                (3, "minor_pentatonic"),
            ] {
                let s = settings(key, scale);
                let notes = melody(seed, s);
                assert!(notes.len() >= 8, "a 4-bar melody has notes (seed {seed})");
                let mut end = 0.0;
                for n in &notes {
                    assert!(
                        in_scale(n.pitch, key, s.scale),
                        "seed {seed}: {} in key",
                        n.pitch
                    );
                    assert!((n.start / 0.25).fract() == 0.0, "on the grid");
                    assert!(n.start >= end - 1e-9, "one note at a time");
                    assert!(n.length > 0.0 && n.start + n.length <= s.length + 1e-9);
                    assert!((0.0..=1.0).contains(&n.velocity));
                    assert!(
                        (50..=80).contains(&n.pitch),
                        "around the centre: {}",
                        n.pitch
                    );
                    end = n.start + n.length;
                }
                assert_eq!(notes[0].start, 0.0, "starts on the first beat");
                let last = notes.last().unwrap().pitch;
                assert_eq!((last + 12 - key) % 12, 0, "ends on the root");
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_melody_and_others_differ() {
        let s = settings(0, "major");
        assert_eq!(melody(7, s), melody(7, s));
        let different = (0..20)
            .filter(|&seed| melody(seed, s) != melody(7, s))
            .count();
        assert!(different >= 18);
        assert!(melody(1, MelodySettings { length: 0.0, ..s }).is_empty());
        assert!(
            !melody(1, MelodySettings { step: 0.0, ..s }).is_empty(),
            "a bad grid falls back"
        );
    }

    #[test]
    fn repitching_keeps_the_rhythm() {
        let s = settings(2, "dorian");
        let original = melody(3, s);
        let mut notes = original.clone();
        repitch(11, &mut notes, 2, s.scale);
        assert_eq!(notes.len(), original.len());
        for (a, b) in notes.iter().zip(&original) {
            assert_eq!(
                (a.start, a.length, a.velocity),
                (b.start, b.length, b.velocity)
            );
            assert!(in_scale(a.pitch, 2, s.scale));
        }
        assert!(notes.iter().zip(&original).any(|(a, b)| a.pitch != b.pitch));
        repitch(1, &mut [], 0, s.scale);
    }
}
