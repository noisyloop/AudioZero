//! The arpeggiator: turns the notes a track plays (from clips, a MIDI keyboard or the computer
//! keyboard) into a pattern, one note at a time, before they reach the instrument.
//!
//! [`ArpSettings`] belong to a track and are saved with the project. [`Arp`] is what the engine
//! runs: it keeps the notes that are held, and for each block turns the incoming note events
//! into the arpeggio's events, sample-accurately and the same whatever the block size.

use crate::engine::Event;
use serde::{Deserialize, Serialize};

/// The most notes one arpeggio steps through: 128 pitches across 4 octaves.
const POOL: usize = 512;
/// The most notes sounding at once (a chord step).
const SOUNDING: usize = 32;

/// The order the held notes are played in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Lowest to highest, then again.
    #[default]
    Up,
    /// Highest to lowest.
    Down,
    /// Up, then back down (the ends aren't played twice).
    UpDown,
    /// A random held note each step.
    Random,
    /// In the order they were pressed.
    AsPlayed,
    /// All the held notes at once, every step.
    Chord,
}

impl Mode {
    pub const ALL: [Mode; 6] = [
        Mode::Up,
        Mode::Down,
        Mode::UpDown,
        Mode::Random,
        Mode::AsPlayed,
        Mode::Chord,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Up => "Up",
            Mode::Down => "Down",
            Mode::UpDown => "Up and down",
            Mode::Random => "Random",
            Mode::AsPlayed => "As played",
            Mode::Chord => "Chord",
        }
    }
}

/// How long a step lasts, as a note length.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rate {
    #[serde(rename = "1/4")]
    Quarter,
    #[serde(rename = "1/8")]
    Eighth,
    #[serde(rename = "1/8T")]
    EighthTriplet,
    #[default]
    #[serde(rename = "1/16")]
    Sixteenth,
    #[serde(rename = "1/16T")]
    SixteenthTriplet,
    #[serde(rename = "1/32")]
    ThirtySecond,
}

impl Rate {
    pub const ALL: [Rate; 6] = [
        Rate::Quarter,
        Rate::Eighth,
        Rate::EighthTriplet,
        Rate::Sixteenth,
        Rate::SixteenthTriplet,
        Rate::ThirtySecond,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Rate::Quarter => "1/4",
            Rate::Eighth => "1/8",
            Rate::EighthTriplet => "1/8T",
            Rate::Sixteenth => "1/16",
            Rate::SixteenthTriplet => "1/16T",
            Rate::ThirtySecond => "1/32",
        }
    }

    /// A step's length in beats.
    pub fn beats(self) -> f64 {
        match self {
            Rate::Quarter => 1.0,
            Rate::Eighth => 0.5,
            Rate::EighthTriplet => 1.0 / 3.0,
            Rate::Sixteenth => 0.25,
            Rate::SixteenthTriplet => 1.0 / 6.0,
            Rate::ThirtySecond => 0.125,
        }
    }
}

/// A track's arpeggiator settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArpSettings {
    pub on: bool,
    pub mode: Mode,
    pub rate: Rate,
    /// How many octaves the pattern climbs through, 1 to 4.
    pub octaves: u8,
    /// How much of a step each note lasts, 0.05 to 1.
    pub gate: f32,
    /// How far the off-beat steps are pushed late, as a share of a step, 0 to 0.5.
    pub swing: f32,
}

impl Default for ArpSettings {
    fn default() -> Self {
        Self {
            on: false,
            mode: Mode::Up,
            rate: Rate::Sixteenth,
            octaves: 1,
            gate: 0.6,
            swing: 0.0,
        }
    }
}

impl ArpSettings {
    /// Puts every value in range (and NaN back to the default).
    pub fn sanitize(&mut self) {
        let default = Self::default();
        self.octaves = self.octaves.clamp(1, 4);
        let fit =
            |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.gate = fit(self.gate, 0.05, 1.0, default.gate);
        self.swing = fit(self.swing, 0.0, 0.5, default.swing);
    }
}

/// What the engine runs for one track.
pub struct Arp {
    settings: ArpSettings,
    /// The notes held down, oldest first, with their velocities.
    held: Vec<(u8, f32)>,
    /// Pitches currently sounding because of the arpeggio.
    sounding: [u8; SOUNDING],
    sounding_count: usize,
    /// Which step comes next, counted from when the first note was pressed.
    step: usize,
    /// Samples from the start of the block until the next step (may be past its end).
    step_in: f64,
    /// Samples until the sounding notes end, if any are sounding.
    off_in: Option<f64>,
    rng: u32,
}

impl Default for Arp {
    fn default() -> Self {
        Self {
            settings: ArpSettings::default(),
            held: Vec::with_capacity(128),
            sounding: [0; SOUNDING],
            sounding_count: 0,
            step: 0,
            step_in: 0.0,
            off_in: None,
            rng: 0x2545_f491,
        }
    }
}

impl Arp {
    pub fn settings(&self) -> ArpSettings {
        self.settings
    }

    pub fn is_on(&self) -> bool {
        self.settings.on
    }

    /// Changes the settings. Turning it off lets go of what it's playing.
    pub fn set_settings(&mut self, mut settings: ArpSettings) {
        settings.sanitize();
        self.settings = settings;
    }

    /// Forgets the held notes and returns the pitches that were sounding, so the engine can
    /// release them.
    pub fn reset(&mut self) -> &[u8] {
        self.held.clear();
        let count = std::mem::take(&mut self.sounding_count);
        self.off_in = None;
        self.step = 0;
        &self.sounding[..count]
    }

    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// The pitches and velocities of the pattern's notes, in the order the mode steps through
    /// them, for every octave.
    fn pool(&self, pitches: &mut [u8; POOL], velocities: &mut [f32; POOL]) -> usize {
        // Built on the stack, so the audio thread never allocates.
        let mut base = [(0u8, 0.0f32); 128];
        let count = self.held.len().min(128);
        base[..count].copy_from_slice(&self.held[..count]);
        if self.settings.mode != Mode::AsPlayed {
            base[..count].sort_unstable_by_key(|&(p, _)| p);
        }
        let mut len = 0;
        for octave in 0..self.settings.octaves {
            for &(pitch, velocity) in &base[..count] {
                let shifted = u16::from(pitch) + 12 * u16::from(octave);
                if shifted <= 127 && len < POOL {
                    pitches[len] = shifted as u8;
                    velocities[len] = velocity;
                    len += 1;
                }
            }
        }
        len
    }

    /// Plays one step at `at` samples into the block.
    fn play_step(&mut self, at: usize, out: &mut Vec<Event>) {
        let (mut pitches, mut velocities) = ([0u8; POOL], [0.0f32; POOL]);
        let len = self.pool(&mut pitches, &mut velocities);
        if len == 0 {
            return;
        }
        // Let go of the last step's notes first (a gate of 1 reaches the next step).
        self.release(at, out);
        let step = self.step;
        let play = |arp: &mut Arp, i: usize, out: &mut Vec<Event>| {
            if arp.sounding_count < SOUNDING && out.len() < out.capacity() {
                out.push(Event {
                    offset: at,
                    on: true,
                    pitch: pitches[i],
                    velocity: velocities[i],
                });
                arp.sounding[arp.sounding_count] = pitches[i];
                arp.sounding_count += 1;
            }
        };
        match self.settings.mode {
            Mode::Chord => {
                // All the held notes, up by an octave per step through the octave range.
                let base = self.held.len().min(len);
                let octave = step % usize::from(self.settings.octaves);
                for i in 0..base {
                    let at_octave = octave * base + i;
                    if at_octave < len {
                        play(self, at_octave, out);
                    }
                }
            }
            Mode::Up | Mode::AsPlayed => play(self, step % len, out),
            Mode::Down => play(self, len - 1 - step % len, out),
            Mode::UpDown => {
                let cycle = (2 * len).saturating_sub(2).max(1);
                let k = step % cycle;
                play(self, if k < len { k } else { cycle - k }, out);
            }
            Mode::Random => {
                let i = self.random() as usize % len;
                play(self, i, out);
            }
        }
        self.step = self.step.wrapping_add(1);
    }

    /// Ends the notes that are sounding, at `at` samples into the block.
    fn release(&mut self, at: usize, out: &mut Vec<Event>) {
        for &pitch in &self.sounding[..self.sounding_count] {
            if out.len() < out.capacity() {
                out.push(Event {
                    offset: at,
                    on: false,
                    pitch,
                    velocity: 0.0,
                });
            }
        }
        self.sounding_count = 0;
        self.off_in = None;
    }

    /// Runs the arpeggio up to `until` samples into the block, from `from`.
    fn advance(&mut self, from: f64, until: f64, step_len: f64, out: &mut Vec<Event>) {
        let mut now = from;
        loop {
            let next_off = self.off_in.filter(|&t| t < until);
            let next_step = (!self.held.is_empty() && self.step_in < until).then_some(self.step_in);
            match (next_off, next_step) {
                (Some(off), step) if step.is_none_or(|s| off <= s) => {
                    now = now.max(off);
                    self.release(now as usize, out);
                }
                (_, Some(at)) => {
                    now = now.max(at);
                    self.play_step(now as usize, out);
                    // Off-beat steps are late and on-beat ones short, by the swing amount.
                    let swing = f64::from(self.settings.swing);
                    let this_len = if self.step % 2 == 1 {
                        step_len * (1.0 + swing)
                    } else {
                        step_len * (1.0 - swing)
                    };
                    self.step_in = at + this_len;
                    self.off_in = Some(at + step_len * f64::from(self.settings.gate));
                }
                _ => break,
            }
        }
    }

    /// Turns `events` (this block's incoming note events, in time order) into the arpeggio's
    /// events, written to `out`. `samples_per_beat` is the tempo. `n` is the block's length.
    pub(crate) fn process(
        &mut self,
        events: &[Event],
        out: &mut Vec<Event>,
        n: usize,
        samples_per_beat: f64,
    ) {
        out.clear();
        let step_len = (self.settings.rate.beats() * samples_per_beat).max(1.0);
        let mut cursor = 0.0;
        for event in events {
            let at = event.offset.min(n) as f64;
            self.advance(cursor, at, step_len, out);
            cursor = at;
            if event.on {
                // A new note: add it (a repeated one replaces its old entry).
                self.held.retain(|&(p, _)| p != event.pitch);
                if self.held.is_empty() {
                    // The first note starts the pattern straight away.
                    self.step = 0;
                    self.step_in = at;
                    self.release(at as usize, out);
                }
                self.held.push((event.pitch, event.velocity));
            } else {
                self.held.retain(|&(p, _)| p != event.pitch);
                if self.held.is_empty() {
                    self.release(at as usize, out);
                }
            }
        }
        self.advance(cursor, n as f64, step_len, out);
        // The next block starts at 0.
        self.step_in -= n as f64;
        if let Some(t) = &mut self.off_in {
            *t -= n as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BPM_SAMPLES: f64 = 24_000.0; // 120 BPM at 48 kHz: half a second a beat

    fn on(offset: usize, pitch: u8) -> Event {
        Event {
            offset,
            on: true,
            pitch,
            velocity: 0.8,
        }
    }

    fn off(offset: usize, pitch: u8) -> Event {
        Event {
            offset,
            on: false,
            pitch,
            velocity: 0.0,
        }
    }

    fn arp_with(modify: impl FnOnce(&mut ArpSettings)) -> Arp {
        let mut settings = ArpSettings {
            on: true,
            rate: Rate::Eighth, // 12 000 samples
            gate: 0.5,
            ..Default::default()
        };
        modify(&mut settings);
        let mut arp = Arp::default();
        arp.set_settings(settings);
        arp
    }

    /// Runs `blocks` blocks of `n` frames, feeding `input` (absolute offsets) in. Returns every
    /// event with its absolute time.
    fn run(arp: &mut Arp, input: &[Event], n: usize, blocks: usize) -> Vec<(usize, bool, u8)> {
        let mut all = Vec::new();
        let mut out = Vec::with_capacity(1024);
        for b in 0..blocks {
            let (from, to) = (b * n, (b + 1) * n);
            let events: Vec<Event> = input
                .iter()
                .filter(|e| (from..to).contains(&e.offset))
                .map(|e| Event {
                    offset: e.offset - from,
                    ..*e
                })
                .collect();
            arp.process(&events, &mut out, n, BPM_SAMPLES);
            all.extend(out.iter().map(|e| (from + e.offset, e.on, e.pitch)));
        }
        all
    }

    fn notes_on(events: &[(usize, bool, u8)]) -> Vec<u8> {
        events.iter().filter(|e| e.1).map(|e| e.2).collect()
    }

    fn chord() -> Vec<Event> {
        vec![on(0, 64), on(0, 60), on(0, 67)]
    }

    #[test]
    fn up_steps_through_the_held_notes_on_the_grid() {
        let mut arp = arp_with(|_| {});
        let events = run(&mut arp, &chord(), 512, 200);
        assert_eq!(notes_on(&events)[..7], [60, 64, 67, 60, 64, 67, 60]);
        let times: Vec<usize> = events.iter().filter(|e| e.1).map(|e| e.0).collect();
        assert_eq!(
            &times[..4],
            &[0, 12_000, 24_000, 36_000],
            "an eighth note apart"
        );
        // Each note ends halfway through its step.
        assert!(events.contains(&(6_000, false, 60)));
        assert!(events.contains(&(18_000, false, 64)));
    }

    #[test]
    fn the_modes_order_the_notes() {
        let pattern = |mode: Mode, octaves: u8| {
            let mut arp = arp_with(|s| {
                s.mode = mode;
                s.octaves = octaves;
            });
            let events = run(&mut arp, &chord(), 512, 200);
            notes_on(&events).into_iter().take(8).collect::<Vec<u8>>()
        };
        assert_eq!(pattern(Mode::Down, 1)[..4], [67, 64, 60, 67]);
        assert_eq!(pattern(Mode::UpDown, 1)[..6], [60, 64, 67, 64, 60, 64]);
        assert_eq!(pattern(Mode::Up, 2)[..8], [60, 64, 67, 72, 76, 79, 60, 64]);
        // As played: the order they were pressed (64, 60, 67).
        assert_eq!(pattern(Mode::AsPlayed, 1)[..4], [64, 60, 67, 64]);
        // Random stays among the held notes.
        let random = pattern(Mode::Random, 1);
        assert!(random.iter().all(|p| [60, 64, 67].contains(p)));
        assert!(random.iter().any(|&p| p != random[0]), "and moves around");
    }

    #[test]
    fn chord_mode_plays_every_held_note_each_step() {
        let mut arp = arp_with(|s| s.mode = Mode::Chord);
        let events = run(&mut arp, &chord(), 512, 60);
        let first: Vec<u8> = events
            .iter()
            .filter(|e| e.0 == 0 && e.1)
            .map(|e| e.2)
            .collect();
        assert_eq!(first, vec![60, 64, 67]);
        assert_eq!(events.iter().filter(|e| e.0 == 12_000 && e.1).count(), 3);
        // Two octaves: the second step is the chord an octave up.
        let mut up = arp_with(|s| {
            s.mode = Mode::Chord;
            s.octaves = 2;
        });
        let events = run(&mut up, &chord(), 512, 60);
        let second: Vec<u8> = events
            .iter()
            .filter(|e| e.0 == 12_000 && e.1)
            .map(|e| e.2)
            .collect();
        assert_eq!(second, vec![72, 76, 79]);
    }

    #[test]
    fn the_result_doesnt_depend_on_the_block_size() {
        let input = vec![
            on(100, 60),
            on(5_000, 64),
            off(30_000, 60),
            on(41_000, 67),
            off(70_000, 64),
        ];
        let reference = run(&mut arp_with(|s| s.swing = 0.2), &input, 64, 1200);
        for n in [100, 512, 4096] {
            let blocks = 76_800 / n + 1;
            let mut other = run(&mut arp_with(|s| s.swing = 0.2), &input, n, blocks);
            other.retain(|e| e.0 < 76_800);
            let expected: Vec<_> = reference.iter().filter(|e| e.0 < 76_800).copied().collect();
            assert_eq!(other, expected, "block size {n}");
        }
    }

    #[test]
    fn releasing_every_note_stops_the_arpeggio_at_once() {
        let mut arp = arp_with(|s| s.gate = 1.0);
        let input = vec![on(0, 60), off(15_000, 60)];
        let events = run(&mut arp, &input, 512, 100);
        // The note that was sounding ends when the key goes up, and nothing starts after.
        assert!(events.contains(&(15_000, false, 60)));
        assert!(events.iter().all(|e| !e.1 || e.0 < 15_000));
        assert!(arp.reset().is_empty(), "nothing is left sounding");
    }

    #[test]
    fn a_held_note_added_later_joins_the_pattern() {
        let mut arp = arp_with(|_| {});
        let input = vec![on(0, 60), on(13_000, 64)];
        let notes = notes_on(&run(&mut arp, &input, 512, 100));
        // The pattern keeps its place: the new note is in from the next step it reaches.
        assert_eq!(notes[..4], [60, 60, 60, 64]);
    }

    #[test]
    fn swing_and_gate_change_the_timing() {
        let times = |swing: f32, gate: f32| {
            let mut arp = arp_with(|s| {
                s.swing = swing;
                s.gate = gate;
            });
            let events = run(&mut arp, &chord(), 512, 100);
            let ons: Vec<usize> = events.iter().filter(|e| e.1).map(|e| e.0).collect();
            let first_off = events.iter().find(|e| !e.1).map(|e| e.0).unwrap();
            (ons, first_off)
        };
        let (straight, _) = times(0.0, 0.5);
        let (swung, _) = times(0.25, 0.5);
        assert_eq!(swung[0], 0);
        assert!(swung[1] > straight[1], "the off-beat step is late");
        assert!(
            swung[2] == straight[2],
            "and the pair is as long as two steps"
        );
        let (_, short) = times(0.0, 0.1);
        let (_, long) = times(0.0, 0.9);
        assert!(short < long);
    }

    #[test]
    fn settings_are_kept_in_range_and_saved_by_name() {
        let mut s = ArpSettings {
            octaves: 9,
            gate: f32::NAN,
            swing: 7.0,
            ..Default::default()
        };
        s.sanitize();
        assert_eq!((s.octaves, s.gate, s.swing), (4, 0.6, 0.5));
        let json = serde_json::to_string(&ArpSettings {
            on: true,
            mode: Mode::UpDown,
            rate: Rate::EighthTriplet,
            ..Default::default()
        })
        .unwrap();
        assert!(json.contains("up_down") && json.contains("1/8T"), "{json}");
        let back: ArpSettings = serde_json::from_str(&json).unwrap();
        assert_eq!((back.mode, back.rate), (Mode::UpDown, Rate::EighthTriplet));
        let defaults: ArpSettings = serde_json::from_str("{}").unwrap();
        assert!(!defaults.on);
        for rate in Rate::ALL {
            assert!(rate.beats() > 0.0 && !rate.name().is_empty());
        }
        for mode in Mode::ALL {
            assert!(!mode.name().is_empty());
        }
    }

    #[test]
    fn it_never_allocates_or_overflows_with_many_notes() {
        let mut arp = arp_with(|s| {
            s.octaves = 4;
            s.mode = Mode::Chord;
        });
        let input: Vec<Event> = (0..128).map(|p| on(0, p)).collect();
        let mut out = Vec::with_capacity(256);
        arp.process(&input, &mut out, 512, BPM_SAMPLES);
        assert!(out.len() <= 256, "output stays inside its capacity");
        assert!(arp.held.len() <= 128);
        assert!(arp.sounding_count <= SOUNDING);
    }
}
