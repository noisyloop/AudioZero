//! Chords: what a chord is, how to voice it, how to play it as a pattern, how to write a
//! progression in key, and how to write a bass line under one.
//!
//! * **Theory.** A [`Chord`] is a root and a [`Quality`]. The chords of a key come from stacking
//!   thirds on the scale ([`diatonic_chord`]), so the same code gives C major's C Dm Em F G Am
//!   Bdim and A harmonic minor's Am Bdim C+ Dm E F G#dim. Pentatonic, blues and chromatic scales
//!   use their relative major or minor.
//! * **Numerals.** A progression is written as roman numerals relative to the key ([`Numeral`]:
//!   semitones above the key, plus a quality), so it can be played in any key.
//! * **Voicing.** [`voice`] picks the actual notes: root position, open, or the inversion that
//!   moves least from the last chord.
//! * **Comping.** [`comp_clip`] turns a list of steps into a clip of ordinary notes in one of
//!   several patterns (held, stabs, offbeats, arpeggios...), so a chord clip plays on any
//!   instrument and is edited in the piano roll like any other.
//! * **Random progressions.** [`random_progression`] walks the usual chord-to-chord moves for a
//!   major or minor key, from plain triads to sevenths, ninths, borrowed chords and secondary
//!   dominants, and always the same for a given seed.
//! * **Bass lines.** [`chord_events`] reads the chords back out of a clip's notes and
//!   [`bass_line`] writes a bass part under them in one of eight styles.

use crate::music::{KEYS, Rng, Scale, scale_by_id};
use crate::project::{Clip, Note};

/// The major scale's steps: roman numerals are relative to it, as usual.
pub const MAJOR_STEPS: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

// ------------------------------------------------------------------------------------ chords

/// What kind of chord, by the notes above its root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality {
    Major,
    Minor,
    Dim,
    Aug,
    Sus2,
    Sus4,
    Major6,
    Minor6,
    Dom7,
    Maj7,
    Min7,
    MinMaj7,
    HalfDim7,
    Dim7,
    Add9,
    Dom9,
    Maj9,
    Min9,
}

impl Quality {
    /// Every quality, simplest first (the order [`identify`] tries them in).
    pub const ALL: [Quality; 18] = [
        Quality::Major,
        Quality::Minor,
        Quality::Dim,
        Quality::Aug,
        Quality::Sus2,
        Quality::Sus4,
        Quality::Major6,
        Quality::Minor6,
        Quality::Dom7,
        Quality::Maj7,
        Quality::Min7,
        Quality::MinMaj7,
        Quality::HalfDim7,
        Quality::Dim7,
        Quality::Add9,
        Quality::Dom9,
        Quality::Maj9,
        Quality::Min9,
    ];

    /// The notes above the root, in semitones (9ths are written an octave up: 14).
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Quality::Major => &[0, 4, 7],
            Quality::Minor => &[0, 3, 7],
            Quality::Dim => &[0, 3, 6],
            Quality::Aug => &[0, 4, 8],
            Quality::Sus2 => &[0, 2, 7],
            Quality::Sus4 => &[0, 5, 7],
            Quality::Major6 => &[0, 4, 7, 9],
            Quality::Minor6 => &[0, 3, 7, 9],
            Quality::Dom7 => &[0, 4, 7, 10],
            Quality::Maj7 => &[0, 4, 7, 11],
            Quality::Min7 => &[0, 3, 7, 10],
            Quality::MinMaj7 => &[0, 3, 7, 11],
            Quality::HalfDim7 => &[0, 3, 6, 10],
            Quality::Dim7 => &[0, 3, 6, 9],
            Quality::Add9 => &[0, 4, 7, 14],
            Quality::Dom9 => &[0, 4, 7, 10, 14],
            Quality::Maj9 => &[0, 4, 7, 11, 14],
            Quality::Min9 => &[0, 3, 7, 10, 14],
        }
    }

    /// The chord symbol's ending: "", "m", "7", "maj7", "m7b5"...
    pub fn symbol(self) -> &'static str {
        match self {
            Quality::Major => "",
            Quality::Minor => "m",
            Quality::Dim => "dim",
            Quality::Aug => "aug",
            Quality::Sus2 => "sus2",
            Quality::Sus4 => "sus4",
            Quality::Major6 => "6",
            Quality::Minor6 => "m6",
            Quality::Dom7 => "7",
            Quality::Maj7 => "maj7",
            Quality::Min7 => "m7",
            Quality::MinMaj7 => "m(maj7)",
            Quality::HalfDim7 => "m7b5",
            Quality::Dim7 => "dim7",
            Quality::Add9 => "add9",
            Quality::Dom9 => "9",
            Quality::Maj9 => "maj9",
            Quality::Min9 => "m9",
        }
    }

    /// Whether roman numerals write it in lower case (it has a minor or diminished third).
    pub fn is_minor_like(self) -> bool {
        matches!(
            self,
            Quality::Minor
                | Quality::Dim
                | Quality::Minor6
                | Quality::Min7
                | Quality::MinMaj7
                | Quality::HalfDim7
                | Quality::Dim7
                | Quality::Min9
        )
    }

    /// The pitch classes above the root, as a 12-bit set.
    fn mask(self) -> u16 {
        self.intervals().iter().fold(0, |m, &i| m | 1 << (i % 12))
    }
}

/// A chord: a root (0 = C to 11 = B) and a quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub root: u8,
    pub quality: Quality,
}

impl Chord {
    pub fn new(root: u8, quality: Quality) -> Self {
        Self {
            root: root % 12,
            quality,
        }
    }

    /// "C", "Am7", "F#dim", "Bbmaj7" (sharps only: "A#maj7").
    pub fn name(&self) -> String {
        format!("{}{}", KEYS[usize::from(self.root)], self.quality.symbol())
    }

    /// The pitch classes (0 to 11), root first.
    pub fn pitch_classes(&self) -> Vec<u8> {
        self.quality
            .intervals()
            .iter()
            .map(|i| (self.root + i) % 12)
            .collect()
    }
}

/// The chord a set of pitch classes makes, if any. `lowest` is the bass note's pitch class: it's
/// tried as the root first, so C6 and Am7 (the same four notes) are told apart by what's lowest.
/// A chord with its fifth left out still counts.
pub fn identify(pcs: &[u8], lowest: Option<u8>) -> Option<Chord> {
    let mask = pcs.iter().fold(0u16, |m, &p| m | 1 << (p % 12));
    if mask.count_ones() < 2 {
        return None;
    }
    let mut roots: Vec<u8> = Vec::new();
    if let Some(low) = lowest.map(|l| l % 12).filter(|l| mask >> l & 1 == 1) {
        roots.push(low);
    }
    let first = roots.first().copied();
    roots.extend((0..12).filter(|r| mask >> r & 1 == 1 && Some(*r) != first));
    let relative = |root: u8| -> u16 {
        (0..12).fold(0, |m, i| {
            if mask >> ((root + i) % 12) & 1 == 1 {
                m | 1 << i
            } else {
                m
            }
        })
    };
    // An exact match first.
    for &root in &roots {
        let rel = relative(root);
        if let Some(q) = Quality::ALL.iter().find(|q| q.mask() == rel) {
            return Some(Chord::new(root, *q));
        }
    }
    // Then the nearest chord that has all these notes (a fifth left out, say).
    for &root in &roots {
        let rel = relative(root);
        let best = Quality::ALL
            .iter()
            .filter(|q| rel & !q.mask() == 0 && rel & 0b1_1011_0000 != 0)
            .min_by_key(|q| (q.mask() & !rel).count_ones());
        if let Some(q) = best {
            return Some(Chord::new(root, *q));
        }
    }
    None
}

// ------------------------------------------------------------------------------------ scales

/// The seven-note scale a scale's chords come from: itself if it has seven notes, otherwise its
/// relative minor (if it has a minor third: minor pentatonic, blues) or major.
pub fn parent_scale(scale: Scale) -> Scale {
    if scale.steps.len() == 7 {
        scale
    } else if scale.steps.len() < 12 && scale.steps.contains(&3) {
        scale_by_id("minor")
    } else {
        scale_by_id("major")
    }
}

/// How many notes a chord built from the scale has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extension {
    Triad,
    Seventh,
    Ninth,
}

impl Extension {
    pub const ALL: [Extension; 3] = [Extension::Triad, Extension::Seventh, Extension::Ninth];

    pub fn name(self) -> &'static str {
        match self {
            Extension::Triad => "Triads",
            Extension::Seventh => "Sevenths",
            Extension::Ninth => "Ninths",
        }
    }
}

fn classify(rel: u16) -> Option<Quality> {
    Quality::ALL.iter().copied().find(|q| q.mask() == rel)
}

/// The chord on scale degree `degree` (0 for the first) of `scale` in `key`, built by stacking
/// thirds. Where a stack isn't one of the known chords the simpler chord is used.
pub fn diatonic_chord(key: u8, scale: Scale, degree: usize, extension: Extension) -> Chord {
    let steps = parent_scale(scale).steps;
    let d = degree % 7;
    let root = steps[d];
    let up = |k: usize| u16::from((steps[(d + k) % 7] + 12 - root) % 12);
    let mask = |ks: &[usize]| ks.iter().fold(1u16, |m, &k| m | 1 << up(k));
    let triad = mask(&[2, 4]);
    let seventh = mask(&[2, 4, 6]);
    let ninth = mask(&[2, 4, 6, 8]);
    let quality = match extension {
        Extension::Ninth => classify(ninth),
        _ => None,
    }
    .or_else(|| match extension {
        Extension::Triad => None,
        _ => classify(seventh),
    })
    .or_else(|| classify(triad))
    .unwrap_or(if up(2) == 3 {
        Quality::Minor
    } else {
        Quality::Major
    });
    Chord::new((key % 12 + root) % 12, quality)
}

// ------------------------------------------------------------------------------------ numerals

/// A chord relative to the key: its root's semitones above the key, and its quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Numeral {
    pub offset: u8,
    pub quality: Quality,
}

const ROMAN: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

impl Numeral {
    pub fn new(offset: u8, quality: Quality) -> Self {
        Self {
            offset: offset % 12,
            quality,
        }
    }

    /// The chord in `key`.
    pub fn resolve(&self, key: u8) -> Chord {
        Chord::new((key % 12 + self.offset) % 12, self.quality)
    }

    /// Reads "I", "bVII", "ii7", "Vmaj7", "viidim", "IVsus4", "V9"... Lower case is minor,
    /// upper case major; a "b" or "#" in front moves the root a semitone.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("\"{text}\" isn't a chord numeral");
        let mut rest = text;
        let mut accidental = 0i32;
        if let Some(r) = rest.strip_prefix('b') {
            (accidental, rest) = (-1, r);
        } else if let Some(r) = rest.strip_prefix('#') {
            (accidental, rest) = (1, r);
        }
        let end = rest
            .find(|c: char| !"IViv".contains(c))
            .unwrap_or(rest.len());
        let (roman, suffix) = rest.split_at(end);
        let upper = roman.chars().all(|c| c.is_ascii_uppercase());
        let lower = roman.chars().all(|c| c.is_ascii_lowercase());
        if roman.is_empty() || !(upper || lower) {
            return Err(bad());
        }
        let degree = ROMAN
            .iter()
            .position(|r| r.eq_ignore_ascii_case(roman))
            .ok_or_else(bad)?;
        let quality = match (suffix, upper) {
            ("", true) => Quality::Major,
            ("", false) => Quality::Minor,
            ("7", true) => Quality::Dom7,
            ("7", false) => Quality::Min7,
            ("maj7", true) => Quality::Maj7,
            ("maj7", false) => Quality::MinMaj7,
            ("7b5", false) | ("m7b5", false) => Quality::HalfDim7,
            ("dim", false) => Quality::Dim,
            ("dim7", false) => Quality::Dim7,
            ("aug", true) | ("+", true) => Quality::Aug,
            ("sus2", _) => Quality::Sus2,
            ("sus4", _) => Quality::Sus4,
            ("6", true) => Quality::Major6,
            ("6", false) => Quality::Minor6,
            ("9", true) => Quality::Dom9,
            ("9", false) => Quality::Min9,
            ("maj9", true) => Quality::Maj9,
            ("add9", true) => Quality::Add9,
            _ => return Err(bad()),
        };
        let offset = (i32::from(MAJOR_STEPS[degree]) + accidental).rem_euclid(12) as u8;
        Ok(Self::new(offset, quality))
    }

    /// The numeral as text, the way [`Numeral::parse`] reads it ("bVII", "ii7", "V7").
    pub fn text(&self) -> String {
        const SPELL: [(&str, usize); 12] = [
            ("", 0),
            ("b", 1),
            ("", 1),
            ("b", 2),
            ("", 2),
            ("", 3),
            ("#", 3),
            ("", 4),
            ("b", 5),
            ("", 5),
            ("b", 6),
            ("", 6),
        ];
        let (accidental, degree) = SPELL[usize::from(self.offset)];
        let roman = if self.quality.is_minor_like() {
            ROMAN[degree].to_ascii_lowercase()
        } else {
            ROMAN[degree].to_owned()
        };
        let suffix = match self.quality {
            Quality::Major | Quality::Minor => "",
            Quality::Dom7 | Quality::Min7 => "7",
            Quality::Maj7 | Quality::MinMaj7 => "maj7",
            Quality::HalfDim7 => "7b5",
            Quality::Dim => "dim",
            Quality::Dim7 => "dim7",
            Quality::Aug => "aug",
            Quality::Sus2 => "sus2",
            Quality::Sus4 => "sus4",
            Quality::Major6 | Quality::Minor6 => "6",
            Quality::Dom9 | Quality::Min9 => "9",
            Quality::Maj9 => "maj9",
            Quality::Add9 => "add9",
        };
        format!("{accidental}{roman}{suffix}")
    }
}

/// A chord and how long it lasts, in beats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub chord: Chord,
    pub beats: f64,
}

// ------------------------------------------------------------------------------------ voicing

/// How a chord's notes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voicing {
    /// Root position every time.
    Root,
    /// The inversion that moves least from the last chord, so the notes glide.
    Smooth,
    /// Root position with the third lifted an octave: wide and airy.
    Open,
}

impl Voicing {
    pub const ALL: [Voicing; 3] = [Voicing::Root, Voicing::Smooth, Voicing::Open];

    pub fn name(self) -> &'static str {
        match self {
            Voicing::Root => "Root position",
            Voicing::Smooth => "Smooth (voice leading)",
            Voicing::Open => "Open",
        }
    }
}

/// The notes of `chord` around `register` (a MIDI note, about the middle of the chord).
/// `previous` is the last chord's notes, which Smooth voicing stays close to. With `bass` the
/// root is added again in the octave from C2 to B2.
pub fn voice(
    chord: Chord,
    style: Voicing,
    previous: Option<&[u8]>,
    register: u8,
    bass: bool,
) -> Vec<u8> {
    let register = i32::from(register.clamp(36, 96));
    // The root nearest a fourth below the register.
    let target = register - 4;
    let root0 = target + (i32::from(chord.root) - target).rem_euclid(12)
        - if (i32::from(chord.root) - target).rem_euclid(12) > 6 {
            12
        } else {
            0
        };
    let base: Vec<i32> = chord
        .quality
        .intervals()
        .iter()
        .map(|&i| root0 + i32::from(i))
        .collect();
    let mut notes = match style {
        Voicing::Root => base,
        Voicing::Open => {
            let mut open = base;
            if open.len() >= 3 {
                open[1] += 12;
            }
            open
        }
        Voicing::Smooth => match previous.filter(|p| !p.is_empty()) {
            None => base,
            Some(prev) => {
                let mut prev: Vec<i32> = prev.iter().map(|&p| i32::from(p)).collect();
                prev.sort_unstable();
                let cost = |candidate: &[i32]| -> i32 {
                    let moved: i32 = candidate
                        .iter()
                        .zip(&prev)
                        .map(|(a, b)| (a - b).abs())
                        .sum();
                    let count = (candidate.len() as i32 - prev.len() as i32).abs() * 3;
                    let (low, high) = (candidate[0], candidate[candidate.len() - 1]);
                    let range = (register - 12 - low).max(0) + (high - register - 14).max(0);
                    moved + count + range * 4
                };
                let mut best: Option<(i32, Vec<i32>)> = None;
                for inversion in 0..base.len() {
                    for octave in [-12, 0, 12] {
                        let mut candidate: Vec<i32> = base
                            .iter()
                            .enumerate()
                            .map(|(i, &n)| if i < inversion { n + 12 } else { n } + octave)
                            .collect();
                        candidate.sort_unstable();
                        let c = cost(&candidate);
                        if best.as_ref().is_none_or(|(b, _)| c < *b) {
                            best = Some((c, candidate));
                        }
                    }
                }
                best.map_or_else(Vec::new, |(_, n)| n)
            }
        },
    };
    notes.sort_unstable();
    if bass {
        let mut low = 36 + (i32::from(chord.root) - 36).rem_euclid(12);
        if low > 47 {
            low -= 12;
        }
        notes.insert(0, low);
    }
    notes.dedup();
    notes
        .into_iter()
        .filter_map(|n| u8::try_from(n).ok().filter(|&n| n <= 127))
        .collect()
}

// ------------------------------------------------------------------------------------ comping

/// How a chord is played over its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Held for the whole chord.
    Block,
    /// On every beat.
    Hits,
    /// Short stabs on the off-beats (house).
    Offbeat,
    /// A syncopated bar: 1, the "and" of 2, 3, and the "and" of 3.
    Syncopated,
    /// Sixteenth notes up through the chord.
    ArpUp,
    /// Sixteenth notes up and back down.
    ArpUpDown,
    /// Held, with the notes rolled in a little apart, like a strummed guitar.
    Strum,
    /// Steady eighth notes.
    Pulse,
}

impl Pattern {
    pub const ALL: [Pattern; 8] = [
        Pattern::Block,
        Pattern::Hits,
        Pattern::Offbeat,
        Pattern::Syncopated,
        Pattern::ArpUp,
        Pattern::ArpUpDown,
        Pattern::Strum,
        Pattern::Pulse,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Pattern::Block => "Held",
            Pattern::Hits => "Hits (every beat)",
            Pattern::Offbeat => "Off-beat stabs",
            Pattern::Syncopated => "Syncopated",
            Pattern::ArpUp => "Arpeggio up",
            Pattern::ArpUpDown => "Arpeggio up and down",
            Pattern::Strum => "Strum",
            Pattern::Pulse => "Eighth-note pulse",
        }
    }
}

/// What to play and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comp {
    pub pattern: Pattern,
    pub voicing: Voicing,
    /// The middle of the chords, as a MIDI note.
    pub register: u8,
    /// Add the root low down.
    pub bass_note: bool,
    /// Seeds the small differences in how hard each note is played.
    pub seed: u64,
}

impl Default for Comp {
    fn default() -> Self {
        Self {
            pattern: Pattern::Block,
            voicing: Voicing::Smooth,
            register: 60,
            bass_note: false,
            seed: 1,
        }
    }
}

fn push(out: &mut Vec<Note>, pitch: u8, start: f64, length: f64, velocity: f32) {
    out.push(Note {
        pitch,
        start,
        length: length.max(1.0 / 32.0),
        velocity: velocity.clamp(0.05, 1.0),
    });
}

/// Plays `notes` (one voiced chord) from `at` for `beats`, in `pattern`.
fn comp_chord(
    pattern: Pattern,
    notes: &[u8],
    at: f64,
    beats: f64,
    rng: &mut Rng,
    out: &mut Vec<Note>,
) {
    let strong = |rng: &mut Rng| 0.82 + 0.12 * rng.unit();
    let soft = |rng: &mut Rng| 0.6 + 0.15 * rng.unit();
    match pattern {
        Pattern::Block => {
            for &p in notes {
                let v = strong(rng);
                push(out, p, at, beats * 0.97, v);
            }
        }
        Pattern::Strum => {
            for (i, &p) in notes.iter().enumerate() {
                let lag = 0.05 * i as f64;
                let v = strong(rng);
                push(out, p, at + lag, beats * 0.97 - lag, v);
            }
        }
        Pattern::Hits => {
            for b in 0..beats.floor() as usize {
                for &p in notes {
                    let v = if b % 4 == 0 { strong(rng) } else { soft(rng) };
                    push(out, p, at + b as f64, 0.9, v);
                }
            }
        }
        Pattern::Offbeat => {
            let mut t = 0.5;
            while t < beats {
                for &p in notes {
                    let v = soft(rng) + 0.1;
                    push(out, p, at + t, 0.25f64.min(beats - t), v);
                }
                t += 1.0;
            }
        }
        Pattern::Pulse => {
            let mut t = 0.0;
            while t < beats {
                let on_beat = t.fract() == 0.0;
                for &p in notes {
                    let v = if on_beat { strong(rng) } else { soft(rng) };
                    push(out, p, at + t, 0.45f64.min(beats - t), v);
                }
                t += 0.5;
            }
        }
        Pattern::Syncopated => {
            const BAR: [(f64, f64); 4] = [(0.0, 1.0), (1.5, 0.5), (2.5, 0.5), (3.0, 0.75)];
            let mut bar = 0.0;
            while bar < beats {
                for (offset, length) in BAR {
                    let t = bar + offset;
                    if t < beats {
                        for &p in notes {
                            let v = if offset == 0.0 {
                                strong(rng)
                            } else {
                                soft(rng)
                            };
                            push(out, p, at + t, length.min(beats - t), v);
                        }
                    }
                }
                bar += 4.0;
            }
        }
        Pattern::ArpUp | Pattern::ArpUpDown => {
            if notes.is_empty() {
                return;
            }
            let mut order: Vec<u8> = notes.to_vec();
            if pattern == Pattern::ArpUpDown && notes.len() > 2 {
                order.extend(notes[1..notes.len() - 1].iter().rev());
            }
            let mut t = 0.0;
            let mut i = 0;
            while t < beats {
                let v = if t.fract() == 0.0 {
                    strong(rng)
                } else {
                    soft(rng)
                };
                push(
                    out,
                    order[i % order.len()],
                    at + t,
                    0.22f64.min(beats - t),
                    v,
                );
                t += 0.25;
                i += 1;
            }
        }
    }
}

/// A clip of ordinary notes that plays `steps` `repeats` times from `start`, long enough for
/// whole bars. Smooth voicing carries on from one chord to the next across the repeats too.
pub fn comp_clip(
    name: &str,
    steps: &[Step],
    repeats: usize,
    start: f64,
    beats_per_bar: u32,
    comp: Comp,
) -> Clip {
    let bar = f64::from(beats_per_bar.max(1));
    let total: f64 = steps.iter().map(|s| s.beats).sum::<f64>() * repeats as f64;
    let length = ((total / bar).ceil() * bar).max(bar);
    let mut rng = Rng::new(comp.seed);
    let mut notes = Vec::new();
    let mut at = 0.0;
    let mut previous: Vec<u8> = Vec::new();
    for _ in 0..repeats {
        for step in steps {
            let voiced = voice(
                step.chord,
                comp.voicing,
                Some(&previous).filter(|p| !p.is_empty()).map(Vec::as_slice),
                comp.register,
                comp.bass_note,
            );
            comp_chord(comp.pattern, &voiced, at, step.beats, &mut rng, &mut notes);
            // Smooth voicing follows the upper notes, not the added bass note.
            previous = if comp.bass_note && voiced.len() > 1 {
                voiced[1..].to_vec()
            } else {
                voiced
            };
            at += step.beats;
        }
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    Clip {
        name: name.into(),
        start,
        length,
        notes,
        ..Default::default()
    }
}

// ------------------------------------------------------------------------------------ random

/// Whether a key sounds major or minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Major,
    Minor,
}

impl Mood {
    /// Minor if the scale has a minor third (minor, dorian, phrygian, blues...), else major. The
    /// chromatic scale, which has every note, counts as major.
    pub fn of_scale(scale: Scale) -> Mood {
        if scale.steps.len() < 12 && scale.steps.contains(&3) {
            Mood::Minor
        } else {
            Mood::Major
        }
    }
}

/// What a random progression should be like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomSettings {
    pub mood: Mood,
    /// How many chords: 2, 3, 4, 6 or 8.
    pub chords: usize,
    /// How many bars they fill.
    pub bars: usize,
    /// 0 plain triads, 1 some sevenths, 2 sevenths, ninths and borrowed chords, 3 also secondary
    /// dominants.
    pub spice: u8,
    pub beats_per_bar: u32,
}

/// The way chords usually follow each other, as (next, weight), for a numeral's text.
fn moves(mood: Mood, from: &str) -> &'static [(&'static str, u32)] {
    match (mood, from) {
        (Mood::Major, "I") => &[("IV", 3), ("V", 3), ("vi", 3), ("ii", 2), ("iii", 1)],
        (Mood::Major, "ii") => &[("V", 5), ("IV", 1), ("viidim", 1)],
        (Mood::Major, "iii") => &[("vi", 3), ("IV", 3), ("ii", 1)],
        (Mood::Major, "IV") => &[("V", 4), ("I", 3), ("ii", 2), ("vi", 1)],
        (Mood::Major, "V") => &[("I", 4), ("vi", 3), ("IV", 2)],
        (Mood::Major, "vi") => &[("IV", 3), ("ii", 3), ("V", 2), ("iii", 1)],
        (Mood::Major, _) => &[("I", 3), ("iii", 1)],
        (Mood::Minor, "i") => &[
            ("bVI", 3),
            ("iv", 3),
            ("bVII", 3),
            ("v", 1),
            ("bIII", 2),
            ("V", 1),
        ],
        (Mood::Minor, "bIII") => &[("bVII", 3), ("iv", 2), ("bVI", 2), ("i", 2)],
        (Mood::Minor, "iv") => &[("bVII", 3), ("i", 2), ("V", 2), ("bVI", 1)],
        (Mood::Minor, "v") => &[("i", 3), ("bVI", 2), ("bIII", 1)],
        (Mood::Minor, "V") => &[("i", 5), ("bVI", 1)],
        (Mood::Minor, "bVI") => &[("bVII", 3), ("bIII", 2), ("i", 2), ("iv", 1), ("V", 1)],
        (Mood::Minor, _) => &[("i", 4), ("bIII", 2), ("bVI", 1), ("iv", 1)],
    }
}

fn starts(mood: Mood) -> &'static [(&'static str, u32)] {
    match mood {
        Mood::Major => &[("I", 14), ("vi", 4), ("IV", 2)],
        Mood::Minor => &[("i", 16), ("bVI", 2), ("iv", 2)],
    }
}

fn endings(mood: Mood) -> &'static [(&'static str, u32)] {
    match mood {
        Mood::Major => &[("V", 9), ("IV", 3), ("I", 5), ("vi", 3)],
        Mood::Minor => &[("bVII", 8), ("V", 5), ("v", 3), ("bVI", 4)],
    }
}

fn weighted(rng: &mut Rng, options: &[(&'static str, u32)]) -> &'static str {
    let total: u32 = options.iter().map(|o| o.1).sum();
    let mut roll = rng.below(total as usize) as u32;
    for &(name, weight) in options {
        if roll < weight {
            return name;
        }
        roll -= weight;
    }
    options[0].0
}

/// How long each chord lasts when `chords` of them fill `total` beats.
pub fn durations(rng: &mut Rng, chords: usize, total: f64) -> Vec<f64> {
    match chords {
        3 => {
            let mut d = vec![total / 2.0, total / 4.0, total / 4.0];
            d.rotate_left(rng.below(3));
            d
        }
        6 => vec![
            total / 4.0,
            total / 8.0,
            total / 8.0,
            total / 4.0,
            total / 8.0,
            total / 8.0,
        ],
        n => vec![total / n.max(1) as f64; n.max(1)],
    }
}

/// A random progression in the major or minor mood: the numerals, each with its length in
/// beats. The same seed and settings always give the same one.
pub fn random_progression(seed: u64, settings: RandomSettings) -> Vec<(Numeral, f64)> {
    let mut rng = Rng::new(seed ^ 0xC40D_5EED);
    let mood = settings.mood;
    let count = settings.chords.clamp(2, 8);
    let total = settings.bars.clamp(1, 16) as f64 * f64::from(settings.beats_per_bar.max(1));
    let mut names: Vec<&'static str> = vec![weighted(&mut rng, starts(mood))];
    while names.len() < count {
        let last = names.len() + 1 == count;
        let from = *names.last().expect("there is a first chord");
        let mut next = if last {
            weighted(&mut rng, endings(mood))
        } else {
            weighted(&mut rng, moves(mood, from))
        };
        for _ in 0..6 {
            if next != from {
                break;
            }
            next = weighted(&mut rng, moves(mood, from));
        }
        names.push(next);
    }
    let mut numerals: Vec<Numeral> = names
        .iter()
        .map(|n| Numeral::parse(n).expect("the tables only hold valid numerals"))
        .collect();

    let spice = settings.spice.min(3);
    let chance = |rng: &mut Rng, p: f32| rng.unit() < p;
    // Borrowed chords: the minor iv, the flat sixth and the flat seventh in a major key.
    if spice >= 2 && mood == Mood::Major {
        for n in numerals.iter_mut().skip(1) {
            if chance(&mut rng, 0.12 * f32::from(spice)) {
                *n = match (n.offset, n.quality) {
                    (5, Quality::Major) => Numeral::new(5, Quality::Minor),
                    (9, Quality::Minor) => Numeral::new(8, Quality::Major),
                    (7, Quality::Major) => Numeral::new(10, Quality::Major),
                    _ => *n,
                };
            }
        }
    }
    // Secondary dominants: a dominant seventh that leads into the next chord.
    if spice >= 3 && numerals.len() >= 3 {
        for i in 1..numerals.len() - 1 {
            if chance(&mut rng, 0.3) && numerals[i + 1].quality != Quality::Dim {
                numerals[i] = Numeral::new(numerals[i + 1].offset + 7, Quality::Dom7);
            }
        }
    }
    // Sevenths and ninths.
    let seventh_chance = [0.0, 0.35, 0.6, 0.85][usize::from(spice)];
    let ninth_chance = [0.0, 0.0, 0.3, 0.55][usize::from(spice)];
    for n in &mut numerals {
        if !chance(&mut rng, seventh_chance) {
            continue;
        }
        let seventh = match (mood, n.quality) {
            (_, Quality::Dom7) => Quality::Dom7,
            (Mood::Major, Quality::Major) if n.offset == 7 => Quality::Dom7,
            (_, Quality::Major) if n.offset == 10 && mood == Mood::Minor => Quality::Dom7,
            (_, Quality::Major) if n.offset == 7 => Quality::Dom7,
            (_, Quality::Major) => Quality::Maj7,
            (_, Quality::Minor) => Quality::Min7,
            (_, Quality::Dim) => Quality::HalfDim7,
            (_, q) => q,
        };
        let extended = match (seventh, chance(&mut rng, ninth_chance)) {
            (Quality::Maj7, true) => Quality::Maj9,
            (Quality::Min7, true) => Quality::Min9,
            (Quality::Dom7, true) => Quality::Dom9,
            (q, _) => q,
        };
        *n = Numeral::new(n.offset, extended);
    }
    let lengths = durations(&mut rng, numerals.len(), total);
    numerals.into_iter().zip(lengths).collect()
}

// ------------------------------------------------------------------------------------ bass

/// How a bass part moves under the chords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BassStyle {
    /// One long root note per chord.
    Root,
    /// Root and fifth, a beat each.
    RootFifth,
    /// Eighth notes jumping an octave (disco).
    Octave,
    /// Roots on the off-beats (house).
    Offbeat,
    /// Four notes a bar through the chord, leading into the next (jazz).
    Walking,
    /// Long, heavy hits with gaps (trap, halftime).
    Sub808,
    /// A busy, rolling line (drum and bass).
    Rolling,
    /// Funky, syncopated roots with the odd fifth and octave.
    Syncopated,
}

impl BassStyle {
    pub const ALL: [BassStyle; 8] = [
        BassStyle::Root,
        BassStyle::RootFifth,
        BassStyle::Octave,
        BassStyle::Offbeat,
        BassStyle::Walking,
        BassStyle::Sub808,
        BassStyle::Rolling,
        BassStyle::Syncopated,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BassStyle::Root => "Root notes",
            BassStyle::RootFifth => "Root and fifth",
            BassStyle::Octave => "Octave eighths",
            BassStyle::Offbeat => "Off-beat roots",
            BassStyle::Walking => "Walking",
            BassStyle::Sub808 => "808 sub",
            BassStyle::Rolling => "Rolling",
            BassStyle::Syncopated => "Syncopated",
        }
    }

    /// The lowest note of the line's range, a MIDI note. Soundgineer's bass presets play an
    /// octave below the note, so these sound from about 55 Hz (and 43 Hz for the 808).
    fn low(self) -> u8 {
        match self {
            BassStyle::Sub808 => 41,
            _ => 45,
        }
    }
}

/// A chord and the stretch of a clip it covers, in beats from the clip's start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChordEvent {
    pub start: f64,
    pub length: f64,
    pub chord: Chord,
}

/// Reads the chords back out of a clip's notes: for each beat, the notes sounding in it are
/// named (see [`identify`]), and beats with the same chord are joined. Works for held chords,
/// stabs and arpeggios alike, as long as the chords change on a beat. Beats with too few notes
/// to name carry on the chord before.
pub fn chord_events(notes: &[Note], clip_length: f64) -> Vec<ChordEvent> {
    let beats = clip_length.floor().max(0.0) as usize;
    let mut events: Vec<ChordEvent> = Vec::new();
    for b in 0..beats {
        let (from, to) = (b as f64, b as f64 + 1.0);
        let sounding: Vec<&Note> = notes
            .iter()
            .filter(|n| n.start < to - 0.02 && n.start + n.length > from + 0.02)
            .collect();
        let pcs: Vec<u8> = sounding.iter().map(|n| n.pitch % 12).collect();
        let lowest = sounding.iter().map(|n| n.pitch).min().map(|p| p % 12);
        let found = identify(&pcs, lowest);
        match (found, events.last_mut()) {
            (Some(chord), Some(last)) if last.chord == chord => last.length += 1.0,
            (Some(chord), _) => events.push(ChordEvent {
                start: from,
                length: 1.0,
                chord,
            }),
            (None, Some(last)) if !sounding.is_empty() || last.start + last.length > from => {
                last.length += 1.0
            }
            (None, _) => {}
        }
    }
    events
}

/// A bass part under `events`, as notes from the clip's start (the same start as the events).
/// The same seed always gives the same line.
pub fn bass_line(events: &[ChordEvent], style: BassStyle, seed: u64) -> Vec<Note> {
    let mut rng = Rng::new(seed ^ 0xBA55);
    let low = i32::from(style.low());
    let root_of = |chord: &Chord| -> i32 { low + (i32::from(chord.root) - low).rem_euclid(12) };
    let mut out: Vec<Note> = Vec::new();
    for (i, event) in events.iter().enumerate() {
        let root = root_of(&event.chord);
        let next_root = events.get(i + 1).map_or(root, |e| root_of(&e.chord));
        let third = root
            + if event.chord.quality.is_minor_like() {
                3
            } else {
                4
            };
        let end = event.length;
        let mut add = |pitch: i32, at: f64, length: f64, velocity: f32| {
            let (at, length) = (at.max(0.0), length.min(end - at));
            if length > 1.0 / 32.0
                && at < end
                && let Ok(p) = u8::try_from(pitch.clamp(0, 127))
            {
                push(&mut out, p, event.start + at, length, velocity);
            }
        };
        let accent = |rng: &mut Rng| 0.85 + 0.1 * rng.unit();
        let ghost = |rng: &mut Rng| 0.6 + 0.12 * rng.unit();
        match style {
            BassStyle::Root => {
                add(root, 0.0, end * 0.95, accent(&mut rng));
            }
            BassStyle::RootFifth => {
                let mut b = 0.0;
                let mut k = 0;
                while b < end {
                    let pitch = if k % 4 == 2 { root + 7 } else { root };
                    add(
                        pitch,
                        b,
                        0.9,
                        if k % 4 == 0 {
                            accent(&mut rng)
                        } else {
                            ghost(&mut rng)
                        },
                    );
                    b += 1.0;
                    k += 1;
                }
            }
            BassStyle::Octave => {
                let mut b = 0.0;
                let mut k = 0;
                while b < end {
                    let pitch = if k % 2 == 0 { root } else { root + 12 };
                    add(
                        pitch,
                        b,
                        0.45,
                        if k % 2 == 0 {
                            accent(&mut rng)
                        } else {
                            ghost(&mut rng)
                        },
                    );
                    b += 0.5;
                    k += 1;
                }
            }
            BassStyle::Offbeat => {
                let mut b = 0.5;
                while b < end {
                    add(root, b, 0.4, accent(&mut rng));
                    b += 1.0;
                }
            }
            BassStyle::Walking => {
                let mut b = 0.0;
                let mut k = 0;
                while b < end {
                    let pitch = match k % 4 {
                        0 => root,
                        1 => {
                            if rng.unit() < 0.5 {
                                third
                            } else {
                                root + 7
                            }
                        }
                        2 => root + 7,
                        _ => {
                            // Step into the next chord from a semitone away, above or below.
                            let target = next_root;
                            if target == root {
                                root + 5
                            } else if rng.unit() < 0.5 {
                                target - 1
                            } else {
                                target + 1
                            }
                        }
                    };
                    add(
                        pitch,
                        b,
                        0.95,
                        if k % 4 == 0 {
                            accent(&mut rng)
                        } else {
                            ghost(&mut rng)
                        },
                    );
                    b += 1.0;
                    k += 1;
                }
            }
            BassStyle::Sub808 => {
                const BAR: [(f64, f64); 3] = [(0.0, 1.5), (2.5, 0.5), (3.0, 1.0)];
                let mut bar = 0.0;
                while bar < end {
                    for (offset, length) in BAR {
                        add(
                            root,
                            bar + offset,
                            length * 0.95,
                            if offset == 0.0 { 1.0 } else { 0.8 },
                        );
                    }
                    bar += 4.0;
                }
            }
            BassStyle::Rolling => {
                const BAR: [(f64, f64); 5] = [
                    (0.0, 0.75),
                    (0.75, 0.75),
                    (1.5, 0.5),
                    (2.0, 1.0),
                    (3.25, 0.75),
                ];
                let mut bar = 0.0;
                while bar < end {
                    for (k, (offset, length)) in BAR.into_iter().enumerate() {
                        let pitch = if k == 4 && rng.unit() < 0.5 {
                            root + 12
                        } else {
                            root
                        };
                        add(
                            pitch,
                            bar + offset,
                            length * 0.9,
                            if k == 0 {
                                accent(&mut rng)
                            } else {
                                ghost(&mut rng)
                            },
                        );
                    }
                    bar += 4.0;
                }
            }
            BassStyle::Syncopated => {
                const BAR: [(f64, f64); 5] = [
                    (0.0, 0.75),
                    (1.0, 0.5),
                    (1.75, 0.5),
                    (2.5, 0.5),
                    (3.0, 0.75),
                ];
                let mut bar = 0.0;
                while bar < end {
                    for (k, (offset, length)) in BAR.into_iter().enumerate() {
                        let roll = rng.unit();
                        let pitch = match k {
                            2 if roll < 0.5 => root + 7,
                            3 if roll < 0.4 => root + 12,
                            _ => root,
                        };
                        add(
                            pitch,
                            bar + offset,
                            length * 0.9,
                            if k == 0 {
                                accent(&mut rng)
                            } else {
                                ghost(&mut rng)
                            },
                        );
                    }
                    bar += 4.0;
                }
            }
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::SCALES;

    fn c(root: u8, q: Quality) -> Chord {
        Chord::new(root, q)
    }

    fn scale(id: &str) -> Scale {
        scale_by_id(id)
    }

    #[test]
    fn chords_have_names_and_notes() {
        assert_eq!(c(0, Quality::Major).name(), "C");
        assert_eq!(c(9, Quality::Min7).name(), "Am7");
        assert_eq!(c(10, Quality::Maj7).name(), "A#maj7");
        assert_eq!(c(11, Quality::HalfDim7).name(), "Bm7b5");
        assert_eq!(c(7, Quality::Dom7).pitch_classes(), vec![7, 11, 2, 5]);
        assert_eq!(c(2, Quality::Add9).pitch_classes(), vec![2, 6, 9, 4]);
    }

    #[test]
    fn every_chord_is_identified_from_its_own_notes() {
        for root in 0..12 {
            for q in Quality::ALL {
                let chord = c(root, q);
                let found = identify(&chord.pitch_classes(), Some(root));
                assert_eq!(found, Some(chord), "{}", chord.name());
            }
        }
        // The same four notes are told apart by the bass note.
        assert_eq!(
            identify(&[0, 4, 7, 9], Some(0)),
            Some(c(0, Quality::Major6))
        );
        assert_eq!(identify(&[0, 4, 7, 9], Some(9)), Some(c(9, Quality::Min7)));
        // A fifth left out, a single note, and nonsense.
        assert_eq!(identify(&[0, 4, 10], Some(0)), Some(c(0, Quality::Dom7)));
        assert_eq!(identify(&[0], None), None);
        assert_eq!(identify(&[], None), None);
    }

    #[test]
    fn the_chords_of_a_key_come_from_its_scale() {
        let names = |key: u8, id: &str, ext: Extension| -> Vec<String> {
            (0..7)
                .map(|d| diatonic_chord(key, scale(id), d, ext).name())
                .collect()
        };
        assert_eq!(
            names(0, "major", Extension::Triad),
            ["C", "Dm", "Em", "F", "G", "Am", "Bdim"]
        );
        assert_eq!(
            names(0, "major", Extension::Seventh),
            ["Cmaj7", "Dm7", "Em7", "Fmaj7", "G7", "Am7", "Bm7b5"]
        );
        assert_eq!(
            names(9, "minor", Extension::Triad),
            ["Am", "Bdim", "C", "Dm", "Em", "F", "G"]
        );
        assert_eq!(
            names(9, "harmonic_minor", Extension::Triad),
            ["Am", "Bdim", "Caug", "Dm", "E", "F", "G#dim"]
        );
        assert_eq!(
            diatonic_chord(0, scale("major"), 0, Extension::Ninth).name(),
            "Cmaj9"
        );
        assert_eq!(
            diatonic_chord(0, scale("major"), 4, Extension::Ninth).name(),
            "G9"
        );
        // Pentatonic and blues borrow their relative minor or major; a key shifts the roots.
        assert_eq!(
            diatonic_chord(2, scale("minor_pentatonic"), 0, Extension::Triad),
            diatonic_chord(2, scale("minor"), 0, Extension::Triad)
        );
        assert_eq!(
            diatonic_chord(2, scale("major_pentatonic"), 4, Extension::Triad).name(),
            "A"
        );
        assert_eq!(parent_scale(scale("chromatic")), scale("major"));
        // Every scale gives a chord on every degree, in every key.
        for s in SCALES {
            for key in 0..12 {
                for d in 0..9 {
                    for e in Extension::ALL {
                        let chord = diatonic_chord(key, s, d, e);
                        assert!(chord.root < 12);
                    }
                }
            }
        }
    }

    #[test]
    fn numerals_read_and_write() {
        let key_c = |t: &str| Numeral::parse(t).unwrap().resolve(0).name();
        assert_eq!(key_c("I"), "C");
        assert_eq!(key_c("vi"), "Am");
        assert_eq!(key_c("bVII"), "A#");
        assert_eq!(key_c("V7"), "G7");
        assert_eq!(key_c("ii7"), "Dm7");
        assert_eq!(key_c("Imaj7"), "Cmaj7");
        assert_eq!(key_c("viidim"), "Bdim");
        assert_eq!(key_c("vii7b5"), "Bm7b5");
        assert_eq!(key_c("IVsus4"), "Fsus4");
        assert_eq!(key_c("bIII"), "D#");
        assert_eq!(key_c("#ivdim"), "F#dim");
        assert_eq!(key_c("Imaj9"), "Cmaj9");
        assert_eq!(Numeral::parse("V").unwrap().resolve(2).name(), "A");
        for bad in ["", "X", "iI", "Vdim", "viiaug", "I7x", "b", "IIII"] {
            assert!(Numeral::parse(bad).is_err(), "{bad}");
        }
        // Writing a numeral and reading it back gives the same numeral, for every one.
        for offset in 0..12 {
            for q in Quality::ALL {
                let n = Numeral::new(offset, q);
                assert_eq!(Numeral::parse(&n.text()), Ok(n), "{} {:?}", n.text(), q);
            }
        }
    }

    fn pcs_of(notes: &[u8]) -> Vec<u8> {
        let mut v: Vec<u8> = notes.iter().map(|n| n % 12).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    #[test]
    fn voicings_keep_the_chord_and_the_register() {
        for q in Quality::ALL {
            for root in 0..12 {
                let chord = c(root, q);
                let mut want = chord.pitch_classes();
                want.sort_unstable();
                want.dedup();
                for style in Voicing::ALL {
                    let notes = voice(chord, style, None, 60, false);
                    assert_eq!(pcs_of(&notes), want, "{} {style:?}", chord.name());
                    assert!(notes.windows(2).all(|w| w[0] < w[1]), "ascending");
                    assert!(notes[0] >= 46 && *notes.last().unwrap() <= 84, "{notes:?}");
                }
            }
        }
        // With a bass note the root is added low down.
        let with = voice(c(9, Quality::Minor), Voicing::Root, None, 60, true);
        assert!((36..48).contains(&with[0]) && with[0] % 12 == 9, "{with:?}");
    }

    #[test]
    fn smooth_voicing_moves_less_than_root_position() {
        let progression = [
            (0, Quality::Major),
            (9, Quality::Minor),
            (5, Quality::Major),
            (7, Quality::Major),
        ];
        let movement = |style: Voicing| -> i32 {
            let mut prev: Vec<u8> = Vec::new();
            let mut total = 0;
            for (root, q) in progression.into_iter().cycle().take(16) {
                let v = voice(
                    c(root, q),
                    style,
                    Some(&prev).filter(|p| !p.is_empty()).map(Vec::as_slice),
                    60,
                    false,
                );
                total += v
                    .iter()
                    .zip(&prev)
                    .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
                    .sum::<i32>();
                prev = v;
            }
            total
        };
        assert!(
            movement(Voicing::Smooth) < movement(Voicing::Root),
            "{} vs {}",
            movement(Voicing::Smooth),
            movement(Voicing::Root)
        );
    }

    fn steps(texts: &[&str]) -> Vec<Step> {
        texts
            .iter()
            .map(|t| Step {
                chord: Numeral::parse(t).unwrap().resolve(0),
                beats: 4.0,
            })
            .collect()
    }

    #[test]
    fn every_pattern_makes_a_valid_clip() {
        let progression = steps(&["I", "V", "vi", "IV"]);
        for pattern in Pattern::ALL {
            for voicing in Voicing::ALL {
                let comp = Comp {
                    pattern,
                    voicing,
                    bass_note: true,
                    ..Comp::default()
                };
                let clip = comp_clip("Chords", &progression, 2, 8.0, 4, comp);
                assert_eq!(clip.start, 8.0);
                assert_eq!(clip.length, 32.0, "two passes of four bars");
                assert!(!clip.notes.is_empty(), "{pattern:?}");
                for n in &clip.notes {
                    assert!(n.pitch <= 127 && n.velocity > 0.0 && n.velocity <= 1.0);
                    assert!(n.start >= 0.0 && n.start < clip.length, "{pattern:?} {n:?}");
                    assert!(
                        n.length > 0.0 && n.start + n.length <= clip.length + 1e-9,
                        "{pattern:?} {n:?}"
                    );
                }
                // The same seed gives the same clip.
                assert_eq!(clip, comp_clip("Chords", &progression, 2, 8.0, 4, comp));
            }
        }
        // The length rounds up to whole bars.
        let short = [Step {
            chord: c(0, Quality::Major),
            beats: 6.0,
        }];
        assert_eq!(
            comp_clip("x", &short, 1, 0.0, 4, Comp::default()).length,
            8.0
        );
    }

    #[test]
    fn a_progression_reads_back_as_its_chords_in_every_pattern() {
        let progression = steps(&["I", "V", "vi", "IV"]);
        for pattern in Pattern::ALL {
            let comp = Comp {
                pattern,
                voicing: Voicing::Smooth,
                ..Comp::default()
            };
            let clip = comp_clip("Chords", &progression, 1, 0.0, 4, comp);
            let events = chord_events(&clip.notes, clip.length);
            let names: Vec<String> = events.iter().map(|e| e.chord.name()).collect();
            assert_eq!(names, ["C", "G", "Am", "F"], "{pattern:?}");
            assert_eq!(
                events.iter().map(|e| e.length).sum::<f64>(),
                16.0,
                "{pattern:?}"
            );
        }
    }

    #[test]
    fn random_progressions_stay_in_key() {
        let major: Vec<u8> = scale("major").steps.to_vec();
        // Natural minor, plus the raised seventh the major V chord needs.
        let mut minor: Vec<u8> = scale("minor").steps.to_vec();
        minor.push(11);
        for seed in 0..300 {
            for (mood, allowed) in [(Mood::Major, &major), (Mood::Minor, &minor)] {
                for chords in [2, 3, 4, 6, 8] {
                    let settings = RandomSettings {
                        mood,
                        chords,
                        bars: 4,
                        spice: 0,
                        beats_per_bar: 4,
                    };
                    let p = random_progression(seed, settings);
                    assert_eq!(p.len(), chords);
                    assert!((p.iter().map(|x| x.1).sum::<f64>() - 16.0).abs() < 1e-9);
                    for (n, beats) in &p {
                        assert!(*beats > 0.0);
                        for pc in n.resolve(0).pitch_classes() {
                            assert!(
                                allowed.contains(&pc),
                                "{mood:?} seed {seed}: {} has {pc}",
                                n.text()
                            );
                        }
                    }
                    assert_eq!(p, random_progression(seed, settings), "repeatable");
                    // No chord twice in a row.
                    assert!(
                        p.windows(2).all(|w| w[0].0 != w[1].0) || chords == 2,
                        "{p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn spice_adds_colour_and_different_seeds_differ() {
        let settings = |spice| RandomSettings {
            mood: Mood::Major,
            chords: 4,
            bars: 4,
            spice,
            beats_per_bar: 4,
        };
        let colourful = |spice| -> usize {
            (0..100)
                .flat_map(|s| random_progression(s, settings(spice)))
                .filter(|(n, _)| n.quality.intervals().len() > 3)
                .count()
        };
        assert_eq!(colourful(0), 0, "plain triads");
        assert!(colourful(1) > 50 && colourful(3) > colourful(1));
        let distinct: std::collections::HashSet<_> = (0..100)
            .map(|s| {
                random_progression(s, settings(0))
                    .iter()
                    .map(|x| x.0)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            distinct.len() > 25,
            "{} different progressions in 100",
            distinct.len()
        );
        // Every chord of the most colourful setting is a real chord, in any key.
        for s in 0..200 {
            for (n, _) in random_progression(s, settings(3)) {
                assert!(!n.resolve(5).name().is_empty());
            }
        }
    }

    #[test]
    fn bass_lines_follow_the_chords() {
        let progression = steps(&["I", "V", "vi", "IV"]);
        let clip = comp_clip("Chords", &progression, 1, 0.0, 4, Comp::default());
        let events = chord_events(&clip.notes, clip.length);
        let roots = [0u8, 7, 9, 5];
        for style in BassStyle::ALL {
            let line = bass_line(&events, style, 3);
            assert!(!line.is_empty(), "{style:?}");
            assert_eq!(line, bass_line(&events, style, 3), "repeatable");
            for n in &line {
                assert!((41..=75).contains(&n.pitch), "{style:?} {n:?}");
                assert!(
                    n.start >= 0.0 && n.start + n.length <= 16.0 + 1e-9,
                    "{style:?} {n:?}"
                );
                assert!(n.velocity > 0.0 && n.velocity <= 1.0);
            }
            // The first note of each chord is that chord's root (the off-beat style starts after
            // the beat).
            for (event, root) in events.iter().zip(roots) {
                let first = line
                    .iter()
                    .find(|n| n.start >= event.start - 1e-9 && n.start < event.start + event.length)
                    .unwrap();
                assert_eq!(first.pitch % 12, root, "{style:?} at {}", event.start);
            }
        }
    }
}
