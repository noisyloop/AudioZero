//! The drum rack: 16 synthesized drum pads with a kit for each genre, and ready-made beats.
//!
//! Nothing is sampled: every drum is a few oscillators and a burst of filtered noise, so the
//! whole rack costs a few kilobytes and sounds the same everywhere. A pad plays one sound at a
//! time (hitting it again restarts it), like a hardware drum machine, and the closed and open
//! hi-hats choke each other.
//!
//! Pads sit on the General MIDI drum notes 36 to 51, so a MIDI keyboard, the computer keyboard
//! and the piano roll all reach them. A kit sets all 16 sounds; the rack's own settings (volume,
//! tune, decay, tone and a level per pad) apply on top of whichever kit is chosen.

use crate::effects::{Kind, Param};
use crate::project::{Clip, Note};
use crate::synth::Instrument;
use std::f32::consts::{PI, TAU};

/// The pad for the lowest drum note.
pub const FIRST_PAD: u8 = 36;
pub const PADS: usize = 16;

/// What each pad is called, from note 36.
pub const PAD_NAMES: [&str; PADS] = [
    "Kick",
    "Rim",
    "Snare",
    "Clap",
    "Snare 2",
    "Low tom",
    "Closed hat",
    "Mid tom",
    "Shaker",
    "High tom",
    "Open hat",
    "Perc",
    "808",
    "Crash",
    "FX",
    "Ride",
];

/// The pad's name for a note, if the note is one of the 16 pads.
pub fn pad_name(pitch: u8) -> Option<&'static str> {
    pitch
        .checked_sub(FIRST_PAD)
        .and_then(|i| PAD_NAMES.get(usize::from(i)))
        .copied()
}

// ---------------------------------------------------------------------------------- sounds

/// How a sound is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Voice {
    /// A sine that starts high and falls to its pitch: kicks, toms, 808s, congas, zaps.
    Sweep,
    /// A short tone plus high-passed noise.
    Snare,
    /// Band-passed noise in quick bursts, then a tail.
    Clap,
    /// Six detuned square waves (the classic 808 cymbal) plus noise, high-passed.
    Metal,
    /// Band-passed noise with a soft start: shakers.
    Noise,
}

/// One drum sound, in plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sound {
    voice: Voice,
    /// Pitch in Hz where a pitch sweep ends, or the metal bank's base frequency.
    freq: f32,
    /// How many times higher than `freq` a sweep starts.
    sweep: f32,
    /// How long the sweep takes to settle, in seconds (for `Noise`, the fade-in).
    sweep_time: f32,
    /// Seconds to fade 60 dB.
    decay: f32,
    /// How much noise there is, 0 to 1: a click on a kick, the body of a snare or hat.
    noise: f32,
    /// The noise filter's frequency in Hz.
    cutoff: f32,
    /// Saturation, 0 to 1.
    drive: f32,
    /// Loudness.
    gain: f32,
    /// Sounds in the same group (not 0) cut each other off.
    group: u8,
}

const fn sound(voice: Voice) -> Sound {
    Sound {
        voice,
        freq: 100.0,
        sweep: 1.0,
        sweep_time: 0.05,
        decay: 0.3,
        noise: 0.0,
        cutoff: 4000.0,
        drive: 0.0,
        gain: 1.0,
        group: 0,
    }
}

/// A kick: starts at `freq * sweep` and falls to `freq` in `sweep_time`.
const fn kick(freq: f32, sweep: f32, sweep_time: f32, decay: f32, click: f32, drive: f32) -> Sound {
    Sound {
        freq,
        sweep,
        sweep_time,
        decay,
        noise: click,
        cutoff: 5000.0,
        drive,
        ..sound(Voice::Sweep)
    }
}

/// A tom, conga, 808 or zap: the same voice as a kick, so just as tweakable.
const fn tone(freq: f32, sweep: f32, sweep_time: f32, decay: f32) -> Sound {
    kick(freq, sweep, sweep_time, decay, 0.0, 0.0)
}

const fn snare(freq: f32, decay: f32, noise: f32, cutoff: f32) -> Sound {
    Sound {
        freq,
        sweep: 1.6,
        sweep_time: 0.02,
        decay,
        noise,
        cutoff,
        ..sound(Voice::Snare)
    }
}

const fn clap(decay: f32, cutoff: f32) -> Sound {
    Sound {
        decay,
        noise: 1.0,
        cutoff,
        ..sound(Voice::Clap)
    }
}

const fn hat(freq: f32, decay: f32, cutoff: f32) -> Sound {
    Sound {
        freq,
        decay,
        noise: 0.25,
        cutoff,
        group: 1,
        gain: 0.7,
        ..sound(Voice::Metal)
    }
}

/// A cymbal: like a hat, but not choked.
const fn cymbal(freq: f32, decay: f32, cutoff: f32) -> Sound {
    Sound {
        group: 0,
        noise: 0.35,
        gain: 0.6,
        ..hat(freq, decay, cutoff)
    }
}

const fn shaker(decay: f32, cutoff: f32, fade_in: f32) -> Sound {
    Sound {
        decay,
        noise: 1.0,
        cutoff,
        sweep_time: fade_in,
        gain: 0.6,
        ..sound(Voice::Noise)
    }
}

/// A sound with its loudness changed.
const fn louder(sound: Sound, gain: f32) -> Sound {
    Sound {
        gain: sound.gain * gain,
        ..sound
    }
}

/// A set of sounds, one per pad.
pub struct Kit {
    pub id: &'static str,
    pub name: &'static str,
    sounds: [Sound; PADS],
}

/// Rim shot: a short, high knock.
const fn rim(freq: f32) -> Sound {
    Sound {
        noise: 0.5,
        cutoff: 6000.0,
        gain: 0.8,
        ..tone(freq, 1.4, 0.004, 0.05)
    }
}

pub static KITS: [Kit; 6] = [
    Kit {
        id: "techno",
        name: "Techno",
        sounds: [
            kick(48.0, 5.0, 0.045, 0.42, 0.25, 0.35),
            rim(1700.0),
            snare(190.0, 0.18, 0.7, 2500.0),
            louder(clap(0.22, 1500.0), 1.0),
            snare(230.0, 0.12, 0.5, 3000.0),
            tone(95.0, 2.0, 0.08, 0.35),
            hat(330.0, 0.05, 7500.0),
            tone(140.0, 2.0, 0.07, 0.3),
            shaker(0.09, 7000.0, 0.01),
            tone(210.0, 2.0, 0.06, 0.25),
            hat(330.0, 0.3, 7500.0),
            tone(330.0, 1.2, 0.03, 0.18),
            tone(45.0, 1.2, 0.05, 0.9),
            cymbal(380.0, 1.4, 6000.0),
            tone(900.0, 6.0, 0.12, 0.25),
            cymbal(520.0, 0.9, 8000.0),
        ],
    },
    Kit {
        id: "house",
        name: "House",
        sounds: [
            kick(52.0, 4.5, 0.04, 0.34, 0.2, 0.25),
            rim(1500.0),
            snare(200.0, 0.2, 0.75, 2200.0),
            clap(0.3, 1300.0),
            snare(240.0, 0.14, 0.55, 2800.0),
            tone(100.0, 2.0, 0.08, 0.35),
            hat(310.0, 0.045, 7000.0),
            tone(150.0, 2.0, 0.07, 0.3),
            shaker(0.12, 6500.0, 0.015),
            tone(220.0, 2.0, 0.06, 0.25),
            hat(310.0, 0.28, 7000.0),
            tone(310.0, 1.2, 0.03, 0.2),
            tone(50.0, 1.2, 0.05, 0.7),
            cymbal(360.0, 1.6, 5500.0),
            tone(800.0, 5.0, 0.1, 0.22),
            cymbal(480.0, 1.0, 7000.0),
        ],
    },
    Kit {
        id: "dubstep",
        name: "Dubstep",
        sounds: [
            kick(44.0, 6.5, 0.05, 0.5, 0.35, 0.6),
            rim(1400.0),
            snare(170.0, 0.3, 0.85, 1800.0),
            louder(clap(0.35, 1100.0), 0.9),
            snare(150.0, 0.45, 0.9, 1500.0),
            tone(85.0, 2.5, 0.1, 0.5),
            hat(340.0, 0.04, 8500.0),
            tone(125.0, 2.5, 0.09, 0.45),
            shaker(0.08, 8000.0, 0.008),
            tone(190.0, 2.5, 0.08, 0.4),
            hat(340.0, 0.35, 8500.0),
            tone(280.0, 1.3, 0.04, 0.22),
            Sound {
                drive: 0.55,
                ..tone(38.0, 1.5, 0.08, 1.4)
            },
            cymbal(400.0, 2.0, 6500.0),
            tone(1200.0, 8.0, 0.25, 0.5),
            cymbal(540.0, 1.1, 8500.0),
        ],
    },
    Kit {
        id: "hiphop",
        name: "Hip hop",
        sounds: [
            kick(55.0, 3.5, 0.035, 0.38, 0.3, 0.2),
            rim(1300.0),
            snare(185.0, 0.24, 0.65, 2000.0),
            clap(0.28, 1200.0),
            snare(210.0, 0.16, 0.45, 2400.0),
            tone(100.0, 1.8, 0.07, 0.35),
            hat(290.0, 0.05, 6500.0),
            tone(150.0, 1.8, 0.06, 0.3),
            shaker(0.1, 6000.0, 0.012),
            tone(215.0, 1.8, 0.05, 0.25),
            hat(290.0, 0.25, 6500.0),
            tone(260.0, 1.2, 0.03, 0.2),
            tone(46.0, 1.1, 0.04, 1.0),
            cymbal(340.0, 1.4, 5500.0),
            tone(700.0, 4.0, 0.1, 0.2),
            cymbal(450.0, 0.9, 6500.0),
        ],
    },
    Kit {
        id: "lofi",
        name: "Lo-fi",
        sounds: [
            // Rounder and softer: lower filter cutoffs, less snap, a little dirt.
            kick(58.0, 2.8, 0.03, 0.3, 0.1, 0.4),
            Sound {
                cutoff: 3500.0,
                ..rim(1100.0)
            },
            snare(175.0, 0.22, 0.55, 1400.0),
            louder(clap(0.3, 900.0), 0.8),
            snare(195.0, 0.14, 0.4, 1600.0),
            tone(105.0, 1.6, 0.06, 0.3),
            hat(260.0, 0.04, 4500.0),
            tone(155.0, 1.6, 0.05, 0.26),
            shaker(0.11, 4500.0, 0.015),
            tone(220.0, 1.6, 0.05, 0.22),
            hat(260.0, 0.22, 4500.0),
            tone(240.0, 1.2, 0.03, 0.2),
            tone(48.0, 1.1, 0.04, 0.9),
            cymbal(300.0, 1.2, 4000.0),
            tone(600.0, 3.0, 0.08, 0.2),
            cymbal(400.0, 0.8, 4500.0),
        ],
    },
    Kit {
        id: "trap",
        name: "Trap",
        sounds: [
            kick(50.0, 4.0, 0.03, 0.5, 0.15, 0.5),
            rim(1600.0),
            snare(210.0, 0.17, 0.8, 3000.0),
            clap(0.25, 1600.0),
            snare(260.0, 0.1, 0.6, 3500.0),
            tone(110.0, 2.0, 0.07, 0.3),
            hat(350.0, 0.03, 9000.0),
            tone(165.0, 2.0, 0.06, 0.28),
            shaker(0.07, 9000.0, 0.006),
            tone(240.0, 2.0, 0.05, 0.22),
            hat(350.0, 0.22, 9000.0),
            tone(380.0, 1.3, 0.03, 0.18),
            // The 808: a long, saturated sub; it's what a trap bass line is made of.
            Sound {
                drive: 0.45,
                ..tone(41.0, 1.8, 0.06, 1.8)
            },
            cymbal(420.0, 1.6, 7500.0),
            tone(1000.0, 7.0, 0.2, 0.4),
            cymbal(560.0, 1.0, 9000.0),
        ],
    },
];

/// The kit with this id, or the first one.
pub fn kit_by_id(id: &str) -> &'static Kit {
    KITS.iter().find(|k| k.id == id).unwrap_or(&KITS[0])
}

// ---------------------------------------------------------------------------------- parameters

const fn p(
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
    kind: Kind,
) -> Param {
    Param {
        name,
        label,
        min,
        max,
        default,
        unit,
        kind,
    }
}

const KIT_NAMES: &[&str] = &["Techno", "House", "Dubstep", "Hip hop", "Lo-fi", "Trap"];
const FIRST_LEVEL: usize = 5;
const PARAM_COUNT: usize = FIRST_LEVEL + PADS;

const fn level(name: &'static str, label: &'static str) -> Param {
    p(name, label, 0.0, 1.5, 1.0, "", Kind::Linear)
}

/// The rack's settings: the kit, volume, tune, decay and tone for all pads, then a level for
/// each pad.
pub static PARAMS: [Param; PARAM_COUNT] = [
    p("kit", "Kit", 0.0, 5.0, 0.0, "", Kind::Choice(KIT_NAMES)),
    p("volume", "Volume", 0.0, 1.0, 0.7, "", Kind::Linear),
    p("tune", "Tune", -12.0, 12.0, 0.0, "st", Kind::Linear),
    p("decay", "Decay", 0.4, 2.5, 1.0, "x", Kind::Log),
    p("tone", "Tone", -1.0, 1.0, 0.0, "", Kind::Linear),
    level("level36", "Kick"),
    level("level37", "Rim"),
    level("level38", "Snare"),
    level("level39", "Clap"),
    level("level40", "Snare 2"),
    level("level41", "Low tom"),
    level("level42", "Closed hat"),
    level("level43", "Mid tom"),
    level("level44", "Shaker"),
    level("level45", "High tom"),
    level("level46", "Open hat"),
    level("level47", "Perc"),
    level("level48", "808"),
    level("level49", "Crash"),
    level("level50", "FX"),
    level("level51", "Ride"),
];

/// How the parameters are grouped for people.
pub const SECTIONS: [(&str, std::ops::Range<usize>); 2] = [
    ("Kit", 0..FIRST_LEVEL),
    ("Pad levels", FIRST_LEVEL..PARAM_COUNT),
];

/// The presets are the kits: picking one switches the kit and puts the rest back to normal.
pub const PRESETS: [&str; 6] = ["Techno", "House", "Dubstep", "Hip hop", "Lo-fi", "Trap"];

pub fn preset(name: &str) -> Option<&'static [(&'static str, f32)]> {
    Some(match name {
        "Techno" => &[("kit", 0.0)],
        "House" => &[("kit", 1.0)],
        "Dubstep" => &[("kit", 2.0)],
        "Hip hop" => &[("kit", 3.0)],
        "Lo-fi" => &[("kit", 4.0)],
        "Trap" => &[("kit", 5.0)],
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------- the rack

/// One pad's voice: a sound, and where it is in playing it.
#[derive(Clone, Copy)]
struct Playing {
    sound: Sound,
    active: bool,
    velocity: f32,
    /// Seconds since the hit, as a sample count.
    t: u32,
    phase: f32,
    amp: f32,
    amp_coef: f32,
    pitch: f32,
    pitch_coef: f32,
    metal: [f32; 6],
    noise: u32,
    /// The noise filter.
    g: f32,
    k: f32,
    ic: [f32; 2],
    /// Fading out fast because another pad in its group was hit.
    choked: bool,
    fade: f32,
    /// How much the pad's sound is turned up.
    gain: f32,
}

impl Playing {
    const IDLE: Playing = Playing {
        sound: sound(Voice::Sweep),
        active: false,
        velocity: 0.0,
        t: 0,
        phase: 0.0,
        amp: 0.0,
        amp_coef: 0.0,
        pitch: 0.0,
        pitch_coef: 0.0,
        metal: [0.0; 6],
        noise: 1,
        g: 0.0,
        k: 1.0,
        ic: [0.0; 2],
        choked: false,
        fade: 1.0,
        gain: 1.0,
    };
}

/// The 808 cymbal's six square waves, as multiples of the base frequency.
const METAL_RATIOS: [f32; 6] = [1.0, 1.4471, 1.6170, 1.9265, 2.5028, 2.6637];

pub struct DrumRack {
    values: [f32; PARAM_COUNT],
    sample_rate: f32,
    voices: [Playing; PADS],
    seed: u32,
}

impl DrumRack {
    pub fn new() -> Self {
        let mut values = [0.0; PARAM_COUNT];
        for (value, param) in values.iter_mut().zip(&PARAMS) {
            *value = param.default;
        }
        Self {
            values,
            sample_rate: 48_000.0,
            voices: [Playing::IDLE; PADS],
            seed: 0x1234_5678,
        }
    }

    fn kit(&self) -> &'static Kit {
        &KITS[(self.values[0].round() as usize).min(KITS.len() - 1)]
    }

    /// The pads' voices currently sounding, for tests.
    #[cfg(test)]
    fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

impl Default for DrumRack {
    fn default() -> Self {
        Self::new()
    }
}

/// A filter's coefficients for `cutoff` Hz.
fn filter(cutoff: f32, rate: f32) -> (f32, f32) {
    let fc = cutoff.clamp(40.0, rate * 0.45);
    ((PI * fc / rate).tan(), 1.2)
}

impl Instrument for DrumRack {
    fn prepare(&mut self, sample_rate: f64) {
        self.sample_rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate as f32
        } else {
            48_000.0
        };
        self.all_notes_off();
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let (Some(slot), Some(param)) = (self.values.get_mut(index), PARAMS.get(index))
            && value.is_finite()
        {
            *slot = value.clamp(param.min, param.max);
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let Some(pad) = pitch
            .checked_sub(FIRST_PAD)
            .map(usize::from)
            .filter(|&p| p < PADS)
        else {
            return;
        };
        let rate = self.sample_rate;
        let sound = self.kit().sounds[pad];
        let tune = 2f32.powf(self.values[2] / 12.0);
        let decay = self.values[3];
        let brightness = 2f32.powf(self.values[4]);
        let (g, k) = filter(sound.cutoff * brightness, rate);
        let amp_time = (sound.decay * decay).max(0.005);
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let noise = self.seed | 1;

        // A hit cuts off the other pads in its group (the open hat when the closed one plays).
        if sound.group != 0 {
            for (i, other) in self.voices.iter_mut().enumerate() {
                if i != pad && other.active && other.sound.group == sound.group {
                    other.choked = true;
                }
            }
        }
        self.voices[pad] = Playing {
            sound: Sound {
                freq: sound.freq * tune,
                ..sound
            },
            active: true,
            velocity: velocity.clamp(0.0, 1.0),
            t: 0,
            phase: 0.0,
            amp: 1.0,
            amp_coef: (-6.9 / (amp_time * rate)).exp(),
            pitch: 1.0,
            pitch_coef: (-4.6 / (sound.sweep_time.max(0.001) * rate)).exp(),
            metal: [0.0; 6],
            noise,
            g,
            k,
            ic: [0.0; 2],
            choked: false,
            fade: 1.0,
            gain: self.values[FIRST_LEVEL + pad],
        };
    }

    fn note_off(&mut self, _pitch: u8) {
        // Drums play out; letting go of a pad changes nothing.
    }

    fn all_notes_off(&mut self) {
        self.voices = [Playing::IDLE; PADS];
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let rate = self.sample_rate;
        let volume = self.values[1] * self.values[1];
        let choke_coef = (-6.9 / (0.006 * rate)).exp();
        for voice in self.voices.iter_mut().filter(|v| v.active) {
            let s = voice.sound;
            let level = volume * voice.velocity * voice.gain * s.gain;
            for i in 0..frames {
                let t = voice.t as f32 / rate;
                let white = {
                    voice.noise ^= voice.noise << 13;
                    voice.noise ^= voice.noise >> 17;
                    voice.noise ^= voice.noise << 5;
                    voice.noise as f32 / 2_147_483_648.0 - 1.0
                };
                // A metal voice's six squares (mixed with some noise); everything else filters
                // plain noise.
                let mut bank = 0.0;
                if s.voice == Voice::Metal {
                    for (phase, ratio) in voice.metal.iter_mut().zip(METAL_RATIOS) {
                        *phase = (*phase + s.freq * ratio / rate).fract();
                        bank += if *phase < 0.5 { 1.0 } else { -1.0 };
                    }
                }
                let input = if s.voice == Voice::Metal {
                    bank / 6.0 * (1.0 - s.noise) + white * s.noise
                } else {
                    white
                };
                // Through the voice's filter, a band-pass and a high-pass.
                let v3 = input - voice.ic[1];
                let v1 = (voice.ic[0] + voice.g * v3) / (1.0 + voice.g * (voice.g + voice.k));
                let v2 = voice.ic[1] + voice.g * v1;
                voice.ic = [2.0 * v1 - voice.ic[0], 2.0 * v2 - voice.ic[1]];
                let band = v1;
                let high = input - voice.k * v1 - v2;

                let mut x = match s.voice {
                    Voice::Sweep => {
                        let f = s.freq * (1.0 + (s.sweep - 1.0) * voice.pitch);
                        voice.phase = (voice.phase + f / rate).fract();
                        let body = (voice.phase * TAU).sin() * voice.amp;
                        // The click: a few milliseconds of bright noise.
                        let click = high * s.noise * (-t / 0.004).exp();
                        saturate(body + click, s.drive)
                    }
                    Voice::Snare => {
                        let f = s.freq * (1.0 + (s.sweep - 1.0) * voice.pitch);
                        voice.phase = (voice.phase + f / rate).fract();
                        let body_env = (-t / (0.35 * s.decay.max(0.02))).exp();
                        let body = (voice.phase * TAU).sin() * body_env * (1.0 - 0.4 * s.noise);
                        body + high * voice.amp * s.noise * 0.9 + band * voice.amp * 0.2
                    }
                    Voice::Clap => {
                        // Four quick slaps 11 ms apart, then the room.
                        let slap = if t < 0.044 {
                            (-(t % 0.011) / 0.0025).exp()
                        } else {
                            0.0
                        };
                        band * 2.0 * slap.max(voice.amp * 0.55) + high * 0.25 * voice.amp
                    }
                    Voice::Metal => high * 2.0,
                    Voice::Noise => {
                        let fade_in = 1.0 - (-t / s.sweep_time.max(0.001)).exp();
                        band * 2.0 * fade_in
                    }
                };
                // Everything but a sweep's own sine rides the amplitude envelope.
                if s.voice != Voice::Sweep {
                    x *= voice.amp;
                }
                if voice.choked {
                    voice.fade *= choke_coef;
                    x *= voice.fade;
                }
                let out = x * level;
                left[i] += out;
                right[i] += out;

                voice.amp *= voice.amp_coef;
                voice.pitch *= voice.pitch_coef;
                voice.t += 1;
                if voice.amp < 1e-4 || voice.fade < 1e-3 {
                    voice.active = false;
                    break;
                }
            }
        }
    }
}

/// Soft clipping: more `drive` is more harmonics and a louder, thicker sound.
fn saturate(x: f32, drive: f32) -> f32 {
    if drive <= 0.0 {
        x
    } else {
        let amount = 1.0 + drive * 5.0;
        (x * amount).tanh() / amount.tanh().max(0.5)
    }
}

// ---------------------------------------------------------------------------------- beats

/// A ready-made beat: one line per pad, a character per sixteenth note. `.` is silence, `o` a
/// soft hit, `x` a normal one and `X` an accent. A line of 32 characters is two bars.
pub struct Beat {
    pub id: &'static str,
    pub name: &'static str,
    /// The kit it was written for.
    pub genre: &'static str,
    /// The tempo it feels right at.
    pub bpm: f64,
    /// How far the off-beat sixteenths are pushed late, as a fraction of a sixteenth.
    pub swing: f64,
    pub lines: &'static [(u8, &'static str)],
}

const K: u8 = 36;
const RIM: u8 = 37;
const SN: u8 = 38;
const CL: u8 = 39;
const SN2: u8 = 40;
const CH: u8 = 42;
const SH: u8 = 44;
const OH: u8 = 46;
const PERC: u8 = 47;
const SUB: u8 = 48;
const CRASH: u8 = 49;
const FX: u8 = 50;

macro_rules! beat {
    ($id:literal, $name:literal, $genre:literal, $bpm:literal, $swing:literal, [$(($pad:expr, $line:literal)),* $(,)?]) => {
        Beat {
            id: $id,
            name: $name,
            genre: $genre,
            bpm: $bpm,
            swing: $swing,
            lines: &[$(($pad, $line)),*],
        }
    };
}

pub static BEATS: [Beat; 18] = [
    // ---- techno: four on the floor, off-beat open hats
    beat!(
        "techno-basic",
        "Four on the floor",
        "techno",
        130.0,
        0.0,
        [
            (K, "X...X...X...X..."),
            (CL, "....X.......X..."),
            (OH, "..x...x...x...x."),
            (CH, "x.x.x.x.x.x.x.x."),
        ]
    ),
    beat!(
        "techno-driving",
        "Driving",
        "techno",
        132.0,
        0.0,
        [
            (K, "X...X...X...X..."),
            (CL, "....x.......x..x"),
            (OH, "..X...X...X...X."),
            (CH, "xoxoxoxoxoxoxoxo"),
            (RIM, ".o..o..o..o..o.."),
            (PERC, "...o.....o...o.."),
        ]
    ),
    beat!(
        "techno-minimal",
        "Minimal",
        "techno",
        126.0,
        0.0,
        [
            (K, "X...X...X...X..."),
            (RIM, "..o...o...o...o."),
            (CH, "..x...x...x...xo"),
            (SH, "o.o.o.o.o.o.o.o."),
            (PERC, "........o......."),
        ]
    ),
    // ---- house
    beat!(
        "house-classic",
        "Classic house",
        "house",
        124.0,
        0.0,
        [
            (K, "X...X...X...X..."),
            (CL, "....X.......X..."),
            (OH, "..x...x...x...x."),
            (CH, "x.x.x.x.x.x.x.x."),
        ]
    ),
    beat!(
        "house-groove",
        "Groove",
        "house",
        122.0,
        0.08,
        [
            (K, "X...X...X...X..."),
            (CL, "....X.......X..."),
            (OH, "..x...x...x...x."),
            (CH, "x.xoxxx.x.xoxxx."),
            (SH, ".o.o.o.o.o.o.o.o"),
            (PERC, ".....o.....o..o."),
        ]
    ),
    beat!(
        "house-deep",
        "Deep",
        "house",
        120.0,
        0.12,
        [
            (K, "X...X...X...X..."),
            (RIM, "....x.......x..."),
            (OH, "..o...o...o...o."),
            (CH, "x.o.x.o.x.o.x.o."),
            (SH, "o.o.o.o.o.o.o.o."),
        ]
    ),
    // ---- dubstep: half-time, the snare on beat three
    beat!(
        "dubstep-halftime",
        "Half-time",
        "dubstep",
        140.0,
        0.0,
        [
            (K, "X.......x.......X.......x..o...."),
            (SN, "........X...............X......."),
            (CH, "x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x."),
            (SUB, "X...............X..............."),
        ]
    ),
    beat!(
        "dubstep-brostep",
        "Brostep",
        "dubstep",
        140.0,
        0.0,
        [
            (K, "X.....x.........X.....x........."),
            (SN2, "........X...............X......."),
            (SN, "............................x.x."),
            (CH, "x.x.x.xxx.x.x.xxx.x.x.xxx.x.x.xx"),
            (FX, "............o..............x...."),
        ]
    ),
    beat!(
        "dubstep-roller",
        "Roller",
        "dubstep",
        140.0,
        0.0,
        [
            (K, "X.....x.....x...X.....x...x....."),
            (SN, "........X...............X......."),
            (CH, "x.xxx.xxx.xxx.xxx.xxx.xxx.xxx.xx"),
            (CL, "........x...............x......."),
            (CRASH, "X..............................."),
        ]
    ),
    // ---- hip hop: boom bap
    beat!(
        "hiphop-boombap",
        "Boom bap",
        "hiphop",
        90.0,
        0.12,
        [
            (K, "X.......o.X....."),
            (SN, "....X.......X..."),
            (CH, "x.x.x.x.x.x.x.x."),
            (OH, "..............x."),
        ]
    ),
    beat!(
        "hiphop-dusty",
        "Dusty",
        "hiphop",
        86.0,
        0.18,
        [
            (K, "X..o....o.X..o.."),
            (SN, "....X.......X..o"),
            (CH, "x.xox.xox.xox.xo"),
            (RIM, "...........o...."),
        ]
    ),
    beat!(
        "hiphop-bounce",
        "Bounce",
        "hiphop",
        94.0,
        0.1,
        [
            (K, "X.....x...X.x..."),
            (CL, "....X.......X..."),
            (SN, "....o.......o..."),
            (CH, "x.x.x.x.x.x.x.x."),
            (OH, "......x.......x."),
            (SUB, "X.........X....."),
        ]
    ),
    // ---- lo-fi: lazy, swung, soft
    beat!(
        "lofi-chill",
        "Chill",
        "lofi",
        75.0,
        0.25,
        [
            (K, "X.....x.....x..."),
            (SN, "....x.......x..."),
            (CH, "x.x.x.x.x.x.x.x."),
            (SH, "..o...o...o...o."),
        ]
    ),
    beat!(
        "lofi-rainy",
        "Rainy day",
        "lofi",
        70.0,
        0.28,
        [
            (K, "X.......x.o....."),
            (RIM, "....x.......x..."),
            (CH, "o.x.o.x.o.x.o.xo"),
            (SH, "o...o...o...o..."),
            (OH, "...............o"),
        ]
    ),
    beat!(
        "lofi-study",
        "Study session",
        "lofi",
        80.0,
        0.2,
        [
            (K, "X..o..x...x..o.."),
            (SN, "....x.......x..."),
            (CL, "....o.......o..."),
            (CH, "x.x.x.x.x.x.x.xo"),
            (PERC, "..........o....."),
        ]
    ),
    // ---- trap: rolling hats, a long 808, the snare on beat three
    beat!(
        "trap-classic",
        "Classic trap",
        "trap",
        140.0,
        0.0,
        [
            (K, "X.......x.x....."),
            (SUB, "X.......x.x....."),
            (CL, "........X......."),
            (SN2, "........o......."),
            (CH, "x.x.x.xxx.x.x.xx"),
            (OH, "..............x."),
        ]
    ),
    beat!(
        "trap-rolls",
        "Hi-hat rolls",
        "trap",
        142.0,
        0.0,
        [
            (K, "X.....x...x.....X.......x.x....."),
            (SUB, "X.....x...x.....X.......x.x....."),
            (CL, "........X...............X......."),
            (CH, "x.x.xxxxx.x.x.xxx.x.xxxxxxxxxxxx"),
            (OH, "..........x.............x......."),
            (FX, ".............o..............o..."),
        ]
    ),
    beat!(
        "trap-drill",
        "Drill",
        "trap",
        144.0,
        0.0,
        [
            (K, "X.........x.....X.....x...x....."),
            (SUB, "X.........x.....X.....x...x....."),
            (SN, "............X..............x...."),
            (CH, "x.xxx.x.xxx.x.xxx.xxx.x.xxx.x.xx"),
            (RIM, "......o.......o......o.......o.."),
        ]
    ),
];

/// The beat with this id.
pub fn beat_by_id(id: &str) -> Option<&'static Beat> {
    BEATS.iter().find(|b| b.id == id)
}

/// The beats for a genre (a kit id), in order.
pub fn beats_for(genre: &str) -> impl Iterator<Item = &'static Beat> {
    BEATS.iter().filter(move |b| b.genre == genre)
}

/// How hard a character in a beat hits, or `None` for silence (or a character that isn't one).
fn hit(c: char) -> Option<f32> {
    match c {
        'X' => Some(1.0),
        'x' => Some(0.8),
        'o' => Some(0.45),
        _ => None,
    }
}

impl Beat {
    /// How many bars long the pattern is (a line is a bar per 16 characters).
    pub fn bars(&self) -> usize {
        self.lines
            .iter()
            .map(|(_, line)| line.chars().count().div_ceil(16))
            .max()
            .unwrap_or(1)
    }

    /// The notes of the pattern played `repeats` times in a row, from beat 0, and how long that
    /// is in beats.
    pub fn notes(&self, repeats: usize) -> (Vec<Note>, f64) {
        let pattern_beats = self.bars() as f64 * 4.0;
        let mut notes = Vec::new();
        for repeat in 0..repeats.max(1) {
            let offset = repeat as f64 * pattern_beats;
            for &(pitch, line) in self.lines {
                for (step, c) in line.chars().enumerate() {
                    let Some(velocity) = hit(c) else { continue };
                    let late = if step % 2 == 1 { self.swing } else { 0.0 };
                    notes.push(Note {
                        pitch,
                        start: offset + (step as f64 + late) * 0.25,
                        length: 0.2,
                        velocity,
                    });
                }
            }
        }
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        (notes, pattern_beats * repeats.max(1) as f64)
    }

    /// A clip of the beat repeated `repeats` times, starting at `start` beats, as long as a
    /// whole number of bars of `beats_per_bar`.
    pub fn clip(&self, repeats: usize, start: f64, beats_per_bar: u32) -> Clip {
        let (notes, beats) = self.notes(repeats);
        let bar = f64::from(beats_per_bar.max(1));
        Clip {
            name: self.name.into(),
            start,
            length: (beats / bar).ceil() * bar,
            notes,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn rack(kit: usize) -> DrumRack {
        let mut rack = DrumRack::new();
        rack.prepare(RATE);
        rack.set_param(0, kit as f32);
        rack
    }

    fn hit_pad(rack: &mut DrumRack, pad: usize, seconds: f64) -> Vec<f32> {
        rack.note_on(FIRST_PAD + pad as u8, 1.0);
        let frames = (seconds * RATE) as usize;
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        for (l, r) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            rack.render(l, r);
        }
        assert_eq!(l, r, "the rack is mono");
        l
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    fn crossings(buf: &[f32]) -> usize {
        buf.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn the_parameter_table_matches_the_rack() {
        assert_eq!(PARAMS.len(), FIRST_LEVEL + PADS);
        assert_eq!(PARAMS[0].name, "kit");
        for (i, name) in PAD_NAMES.iter().enumerate() {
            assert_eq!(PARAMS[FIRST_LEVEL + i].label, *name);
            assert_eq!(PARAMS[FIRST_LEVEL + i].name, format!("level{}", 36 + i));
        }
        assert_eq!(KITS.len(), KIT_NAMES.len());
        for (kit, name) in KITS.iter().zip(KIT_NAMES) {
            assert_eq!(kit.name, *name);
            assert_eq!(
                preset(name).unwrap()[0].1 as usize,
                KITS.iter().position(|k| k.name == *name).unwrap()
            );
        }
        assert_eq!(PRESETS.len(), KITS.len());
        let mut covered: Vec<usize> = SECTIONS.iter().flat_map(|(_, r)| r.clone()).collect();
        covered.sort_unstable();
        assert_eq!(covered, (0..PARAMS.len()).collect::<Vec<_>>());
        assert_eq!(pad_name(36), Some("Kick"));
        assert_eq!(pad_name(51), Some("Ride"));
        assert_eq!((pad_name(35), pad_name(52)), (None, None));
        assert_eq!(kit_by_id("trap").name, "Trap");
        assert_eq!(kit_by_id("nope").name, "Techno");
    }

    #[test]
    fn every_pad_of_every_kit_makes_a_clean_sound_that_ends() {
        for (k, kit) in KITS.iter().enumerate() {
            for (pad, pad_name) in PAD_NAMES.iter().enumerate() {
                let mut rack = rack(k);
                let out = hit_pad(&mut rack, pad, 4.0);
                let name = format!("{} / {}", kit.name, pad_name);
                assert!(out.iter().all(|x| x.is_finite()), "{name} is finite");
                let level = peak(&out);
                assert!(level > 0.03, "{name} is heard ({level})");
                assert!(level < 1.2, "{name} isn't wildly loud ({level})");
                assert_eq!(rack.active(), 0, "{name} has finished after 4 s");
                assert_eq!(
                    peak(&out[out.len() - 2_000..]),
                    0.0,
                    "{name} ends in silence"
                );
            }
        }
    }

    #[test]
    fn drums_sound_like_what_they_are() {
        let rate = RATE as f32;
        // A kick is low: the first 150 ms are mostly a falling sine under 200 Hz.
        let kick = hit_pad(&mut rack(0), 0, 0.5);
        let hz = crossings(&kick[(0.1 * rate) as usize..(0.4 * rate) as usize]) as f32 / 0.3;
        assert!(
            (35.0..90.0).contains(&hz),
            "a kick settles low, got {hz} Hz"
        );
        let early = crossings(&kick[..(0.02 * rate) as usize]) as f32 / 0.02;
        assert!(early > hz * 1.5, "and starts higher: {early} then {hz}");

        // A hat is bright: lots of zero crossings. A closed one is shorter than an open one.
        let closed = hit_pad(&mut rack(0), 6, 1.0);
        let open = hit_pad(&mut rack(0), 10, 1.0);
        assert!(crossings(&closed[..4800]) > 100);
        let tail = |b: &[f32]| peak(&b[(0.15 * rate) as usize..]);
        assert!(
            tail(&open) > 6.0 * tail(&closed).max(1e-6),
            "the open hat rings on"
        );

        // The 808 is long and low, and outlasts a kick by a long way.
        let sub = hit_pad(&mut rack(5), 12, 2.0);
        let kick = hit_pad(&mut rack(5), 0, 2.0);
        let late = (0.7 * rate) as usize..;
        assert!(peak(&sub[late.clone()]) > 0.02, "still sounding at 0.7 s");
        assert!(peak(&sub[late.clone()]) > 5.0 * peak(&kick[late]));
        let hz = crossings(&sub[(0.3 * rate) as usize..(1.3 * rate) as usize]) as f32;
        assert!(
            (35.0..55.0).contains(&hz),
            "an 808 is a low note, got {hz} Hz"
        );
    }

    #[test]
    fn settings_change_the_sound() {
        let hit = |setup: &dyn Fn(&mut DrumRack)| {
            let mut r = rack(0);
            setup(&mut r);
            hit_pad(&mut r, 0, 0.6)
        };
        let normal = hit(&|_| {});
        let quiet = hit(&|r| r.set_param(1, 0.4));
        assert!(peak(&quiet) < 0.4 * peak(&normal), "volume");
        let short = hit(&|r| r.set_param(3, 0.4));
        let from = (0.3 * RATE) as usize;
        assert!(peak(&short[from..]) < 0.3 * peak(&normal[from..]), "decay");
        let high = hit(&|r| r.set_param(2, 12.0));
        let rate = |b: &[f32]| crossings(&b[4800..14_400]);
        assert!(
            rate(&high) > rate(&normal) * 3 / 2,
            "tune: an octave up is about twice as fast"
        );
        let muted = hit(&|r| r.set_param(FIRST_LEVEL, 0.0));
        assert_eq!(peak(&muted), 0.0, "a pad's level");
        let soft = {
            let mut r = rack(0);
            r.note_on(36, 0.3);
            let (mut l, mut rr) = (vec![0.0; 4800], vec![0.0; 4800]);
            r.render(&mut l, &mut rr);
            l
        };
        assert!(peak(&soft) < 0.5 * peak(&normal), "velocity");
        // Different kits are different sounds.
        let mut other = rack(4);
        assert_ne!(normal, hit_pad(&mut other, 0, 0.6));
    }

    #[test]
    fn the_closed_hat_chokes_the_open_hat_and_only_it() {
        let mut r = rack(0);
        r.note_on(FIRST_PAD + 10, 1.0); // open hat
        r.note_on(FIRST_PAD + 13, 1.0); // crash
        let (mut l, mut rr) = (vec![0.0; 2400], vec![0.0; 2400]);
        r.render(&mut l, &mut rr);
        r.note_on(FIRST_PAD + 6, 1.0); // closed hat
        let (mut l, mut rr) = (vec![0.0; 9600], vec![0.0; 9600]);
        r.render(&mut l, &mut rr);
        assert!(!r.voices[10].active, "the open hat was cut off");
        assert!(r.voices[13].active, "the crash rings on");
        // Hitting a pad again restarts it rather than stacking.
        r.note_on(FIRST_PAD, 1.0);
        r.note_on(FIRST_PAD, 1.0);
        assert!(r.voices[0].active && r.voices[0].t == 0);
    }

    #[test]
    fn notes_outside_the_pads_and_bad_values_are_ignored() {
        let mut r = rack(0);
        for pitch in [0, 35, 52, 127] {
            r.note_on(pitch, 1.0);
        }
        assert_eq!(r.active(), 0);
        r.set_param(0, f32::NAN);
        r.set_param(1, 50.0);
        r.set_param(999, 1.0);
        assert_eq!(r.values[1], 1.0, "clamped");
        r.note_on(36, f32::NAN.max(0.5));
        r.all_notes_off();
        assert_eq!(r.active(), 0);
        r.note_off(36);
        let (mut l, mut rr) = (vec![0.0; 100], vec![0.0; 100]);
        r.render(&mut l, &mut rr);
        assert_eq!(peak(&l), 0.0);
    }

    #[test]
    fn every_beat_is_well_formed() {
        let mut ids: Vec<_> = BEATS.iter().map(|b| b.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BEATS.len(), "unique ids");
        for kit in &KITS {
            assert_eq!(beats_for(kit.id).count(), 3, "three beats for {}", kit.name);
        }
        for beat in &BEATS {
            assert!(
                kit_by_id(beat.genre).id == beat.genre,
                "{} has a kit",
                beat.id
            );
            assert!((60.0..=180.0).contains(&beat.bpm) && (0.0..=0.33).contains(&beat.swing));
            let bars = beat.bars();
            for (pad, line) in beat.lines {
                assert!(pad_name(*pad).is_some(), "{}: pad {pad}", beat.id);
                assert_eq!(
                    line.chars().count(),
                    bars * 16,
                    "{} / {}: whole bars",
                    beat.id,
                    pad
                );
                assert!(
                    line.chars().all(|c| ".xXo".contains(c)),
                    "{}: {line}",
                    beat.id
                );
            }
            let (notes, beats) = beat.notes(1);
            assert_eq!(beats, bars as f64 * 4.0);
            assert!(notes.len() >= 8, "{} has notes", beat.id);
            assert!(
                notes
                    .iter()
                    .all(|n| n.start >= 0.0 && n.start + n.length <= beats + 0.3 * 0.25 + 1e-9)
            );
            assert!(
                notes.windows(2).all(|w| w[0].start <= w[1].start),
                "in order"
            );
        }
        assert_eq!(beat_by_id("trap-classic").unwrap().genre, "trap");
        assert!(beat_by_id("nope").is_none());
    }

    #[test]
    fn beats_repeat_swing_and_fit_whole_bars() {
        let basic = beat_by_id("techno-basic").unwrap();
        let (once, beats) = basic.notes(1);
        let (twice, double) = basic.notes(2);
        assert_eq!((beats, double), (4.0, 8.0));
        assert_eq!(twice.len(), 2 * once.len());
        let kicks: Vec<f64> = once
            .iter()
            .filter(|n| n.pitch == K)
            .map(|n| n.start)
            .collect();
        assert_eq!(kicks, vec![0.0, 1.0, 2.0, 3.0], "four on the floor");
        assert!(
            twice
                .iter()
                .filter(|n| n.pitch == K)
                .any(|n| n.start == 7.0)
        );

        // Swing pushes the off-beat sixteenths late and leaves the others alone.
        let lofi = beat_by_id("lofi-chill").unwrap();
        let (notes, _) = lofi.notes(1);
        let hats: Vec<f64> = notes
            .iter()
            .filter(|n| n.pitch == CH)
            .map(|n| n.start)
            .collect();
        assert_eq!(hats[0], 0.0);
        assert!(
            hats.iter().all(|s| (s * 4.0).fract() == 0.0),
            "hats are on the eighth grid"
        );
        let snare = notes.iter().find(|n| n.pitch == SN).unwrap();
        assert_eq!(snare.start, 1.0);

        let clip = basic.clip(3, 8.0, 4);
        assert_eq!((clip.start, clip.length), (8.0, 12.0));
        assert_eq!(
            basic.clip(1, 0.0, 3).length,
            6.0,
            "rounded up to whole 3-beat bars"
        );
        assert!(clip.notes.iter().all(|n| n.start < clip.length));
    }

    #[test]
    fn a_beat_played_through_the_rack_is_audible_and_in_time() {
        let beat = beat_by_id("hiphop-boombap").unwrap();
        let mut project = crate::Project {
            bpm: beat.bpm,
            ..Default::default()
        };
        project.tracks.push(crate::Track {
            name: "Drums".into(),
            instrument: Some(crate::Device::builtin("drums")),
            clips: vec![beat.clip(2, 0.0, 4)],
            ..Default::default()
        });
        let mut engine = crate::Engine::new(project, RATE, 512).unwrap();
        assert!(engine.warnings().is_empty(), "{:?}", engine.warnings());
        engine.play();
        let seconds = 8.0 * 60.0 / beat.bpm;
        let (left, _) = engine.render((seconds * RATE) as usize);
        assert!(peak(&left) > 0.1, "the beat is heard");
        // The first kick lands at the start; the first snare on beat 2.
        assert!(peak(&left[..2400]) > 0.1);
        let beat_frames = (60.0 / beat.bpm * RATE) as usize;
        assert!(peak(&left[beat_frames..beat_frames + 1200]) > 0.0);
    }
}
