//! More built-in track effects: a four-band parametric **EQ**, **chorus**, **phaser**, **bit
//! crusher** and **distortion**. (The glitch effect is in [`crate::glitch`].)
//!
//! They work the same way as the older effects in [`crate::effects`]: buffers are allocated in
//! `prepare` (while the engine is built), and `set_param` and `process` never allocate.

use crate::effects::{Effect, Kind, Param};
use crate::soundgineer::effects::{Chorus as ChorusCore, Phaser as PhaserCore};
use std::f32::consts::TAU;

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

// ---------------------------------------------------------------------------- parameters

/// The EQ's four bands: a low shelf, two bells and a high shelf. Each has a frequency and a gain;
/// the bells also have a Q (how narrow they are).
pub static EQ_PARAMS: [Param; 10] = [
    p("lowFreq", "Low freq", 20.0, 500.0, 100.0, "Hz", Kind::Log),
    p("lowGain", "Low gain", -18.0, 18.0, 0.0, "dB", Kind::Linear),
    p("m1Freq", "Mid 1 freq", 80.0, 8000.0, 500.0, "Hz", Kind::Log),
    p("m1Gain", "Mid 1 gain", -18.0, 18.0, 0.0, "dB", Kind::Linear),
    p("m1Q", "Mid 1 Q", 0.2, 10.0, 1.0, "", Kind::Log),
    p(
        "m2Freq",
        "Mid 2 freq",
        300.0,
        16000.0,
        3000.0,
        "Hz",
        Kind::Log,
    ),
    p("m2Gain", "Mid 2 gain", -18.0, 18.0, 0.0, "dB", Kind::Linear),
    p("m2Q", "Mid 2 Q", 0.2, 10.0, 1.0, "", Kind::Log),
    p(
        "highFreq",
        "High freq",
        1500.0,
        18000.0,
        8000.0,
        "Hz",
        Kind::Log,
    ),
    p(
        "highGain",
        "High gain",
        -18.0,
        18.0,
        0.0,
        "dB",
        Kind::Linear,
    ),
];

pub static CHORUS_PARAMS: [Param; 3] = [
    p("rate", "Rate", 0.05, 8.0, 0.4, "Hz", Kind::Log),
    p("depth", "Depth", 0.0, 1.0, 0.5, "", Kind::Linear),
    p("mix", "Mix", 0.0, 1.0, 0.5, "", Kind::Linear),
];

pub static PHASER_PARAMS: [Param; 4] = [
    p("rate", "Rate", 0.02, 10.0, 0.3, "Hz", Kind::Log),
    p("depth", "Depth", 0.0, 1.0, 0.7, "", Kind::Linear),
    p("feedback", "Feedback", 0.0, 0.95, 0.4, "", Kind::Linear),
    p("mix", "Mix", 0.0, 1.0, 0.5, "", Kind::Linear),
];

pub static BITCRUSHER_PARAMS: [Param; 3] = [
    p("bits", "Bits", 1.0, 16.0, 8.0, "bit", Kind::Linear),
    p("downsample", "Rate divide", 1.0, 64.0, 1.0, "x", Kind::Log),
    p("mix", "Mix", 0.0, 1.0, 1.0, "", Kind::Linear),
];

const DISTORTION_TYPES: &[&str] = &["Soft clip", "Hard clip", "Wavefold"];

pub static DISTORTION_PARAMS: [Param; 4] = [
    p(
        "type",
        "Type",
        0.0,
        2.0,
        0.0,
        "",
        Kind::Choice(DISTORTION_TYPES),
    ),
    p("drive", "Drive", 0.0, 1.0, 0.4, "", Kind::Linear),
    p("tone", "Tone", 200.0, 18000.0, 8000.0, "Hz", Kind::Log),
    p("mix", "Mix", 0.0, 1.0, 1.0, "", Kind::Linear),
];

// ---------------------------------------------------------------------------- EQ

/// One biquad filter's coefficients, divided through by `a0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
}

impl Biquad {
    /// Does nothing.
    pub const FLAT: Biquad = Biquad {
        b: [1.0, 0.0, 0.0],
        a: [0.0, 0.0],
    };

    fn new(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [a1 / a0, a2 / a0],
        }
    }

    /// A low shelf (RBJ cookbook, shelf slope 1).
    pub fn low_shelf(sr: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w = TAU * freq.clamp(10.0, sr * 0.45) / sr;
        let (cs, sn) = (w.cos(), w.sin());
        let beta = 2.0 * a.sqrt() * (sn / std::f32::consts::SQRT_2);
        Self::new(
            a * ((a + 1.0) - (a - 1.0) * cs + beta),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - beta),
            (a + 1.0) + (a - 1.0) * cs + beta,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - beta,
        )
    }

    /// A high shelf (RBJ cookbook, shelf slope 1).
    pub fn high_shelf(sr: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w = TAU * freq.clamp(10.0, sr * 0.45) / sr;
        let (cs, sn) = (w.cos(), w.sin());
        let beta = 2.0 * a.sqrt() * (sn / std::f32::consts::SQRT_2);
        Self::new(
            a * ((a + 1.0) + (a - 1.0) * cs + beta),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - beta),
            (a + 1.0) - (a - 1.0) * cs + beta,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - beta,
        )
    }

    /// A bell (peaking filter) with quality factor `q`.
    pub fn bell(sr: f32, freq: f32, gain_db: f32, q: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w = TAU * freq.clamp(10.0, sr * 0.45) / sr;
        let alpha = w.sin() / (2.0 * q.max(0.05));
        let cs = w.cos();
        Self::new(
            1.0 + alpha * a,
            -2.0 * cs,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cs,
            1.0 - alpha / a,
        )
    }

    /// The gain in dB at `freq`, for a sample rate of `sr`. Worked out in 64 bits: near the
    /// bottom of the range the terms nearly cancel, and 32 bits isn't precise enough.
    pub fn gain_db(&self, sr: f32, freq: f32) -> f32 {
        let w = std::f64::consts::TAU * f64::from(freq) / f64::from(sr);
        let (c1, c2) = (w.cos(), (2.0 * w).cos());
        let [b0, b1, b2] = self.b.map(f64::from);
        let [a1, a2] = self.a.map(f64::from);
        let num = b0 * b0 + b1 * b1 + b2 * b2 + 2.0 * (b0 * b1 + b1 * b2) * c1 + 2.0 * b0 * b2 * c2;
        let den = 1.0 + a1 * a1 + a2 * a2 + 2.0 * (a1 + a1 * a2) * c1 + 2.0 * a2 * c2;
        (10.0 * (num / den).max(1e-12).log10()) as f32
    }
}

/// The four filters an EQ's settings make, in order: low shelf, bell, bell, high shelf.
pub fn eq_filters(values: &[f32], sr: f32) -> [Biquad; 4] {
    let v = |i: usize| values.get(i).copied().unwrap_or(EQ_PARAMS[i].default);
    [
        Biquad::low_shelf(sr, v(0), v(1)),
        Biquad::bell(sr, v(2), v(3), v(4)),
        Biquad::bell(sr, v(5), v(6), v(7)),
        Biquad::high_shelf(sr, v(8), v(9)),
    ]
}

/// The EQ's overall gain in dB at `freq`: what the curve in the editor draws.
pub fn eq_response_db(values: &[f32], sr: f32, freq: f32) -> f32 {
    eq_filters(values, sr)
        .iter()
        .map(|f| f.gain_db(sr, freq))
        .sum()
}

/// A four-band parametric EQ.
pub struct Eq {
    values: [f32; 10],
    filters: [Biquad; 4],
    /// Per channel, per filter: the last two inputs and outputs.
    state: [[[f32; 4]; 4]; 2],
    sr: f32,
}

impl Default for Eq {
    fn default() -> Self {
        let mut values = [0.0; 10];
        for (v, param) in values.iter_mut().zip(&EQ_PARAMS) {
            *v = param.default;
        }
        let mut eq = Self {
            values,
            filters: [Biquad::FLAT; 4],
            state: [[[0.0; 4]; 4]; 2],
            sr: 48_000.0,
        };
        eq.update();
        eq
    }
}

impl Eq {
    fn update(&mut self) {
        self.filters = eq_filters(&self.values, self.sr);
    }
}

impl Effect for Eq {
    fn prepare(&mut self, sample_rate: f64) {
        self.sr = sample_rate as f32;
        self.state = [[[0.0; 4]; 4]; 2];
        self.update();
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value;
            self.update();
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (ch, buf) in [left, right].into_iter().enumerate() {
            for sample in buf.iter_mut() {
                let mut x = *sample;
                for (f, state) in self.filters.iter().zip(&mut self.state[ch]) {
                    // state: x1, x2, y1, y2
                    let y = f.b[0] * x + f.b[1] * state[0] + f.b[2] * state[1]
                        - f.a[0] * state[2]
                        - f.a[1] * state[3];
                    let y = if y.abs() < 1e-20 { 0.0 } else { y };
                    *state = [x, state[0], y, state[2]];
                    x = y;
                }
                *sample = x;
            }
        }
    }
}

// ---------------------------------------------------------------------------- chorus, phaser

/// Chorus: a short delay swept by a slow LFO, mixed with the dry sound.
pub struct Chorus {
    core: ChorusCore,
    rate: f32,
    depth: f32,
    mix: f32,
}

impl Default for Chorus {
    fn default() -> Self {
        Self {
            core: ChorusCore::new(48_000.0),
            rate: CHORUS_PARAMS[0].default,
            depth: CHORUS_PARAMS[1].default,
            mix: CHORUS_PARAMS[2].default,
        }
    }
}

impl Effect for Chorus {
    fn prepare(&mut self, sample_rate: f64) {
        self.core = ChorusCore::new(sample_rate as f32);
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.rate = value,
            1 => self.depth = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.core
            .process(left, right, self.rate, self.depth, self.mix);
    }
}

/// Phaser: six all-pass stages swept by an LFO, with feedback.
pub struct Phaser {
    core: PhaserCore,
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
}

impl Default for Phaser {
    fn default() -> Self {
        Self {
            core: PhaserCore::new(48_000.0),
            rate: PHASER_PARAMS[0].default,
            depth: PHASER_PARAMS[1].default,
            feedback: PHASER_PARAMS[2].default,
            mix: PHASER_PARAMS[3].default,
        }
    }
}

impl Effect for Phaser {
    fn prepare(&mut self, sample_rate: f64) {
        self.core = PhaserCore::new(sample_rate as f32);
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.rate = value,
            1 => self.depth = value,
            2 => self.feedback = value,
            3 => self.mix = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.core
            .process(left, right, self.rate, self.depth, self.feedback, self.mix);
    }
}

// ---------------------------------------------------------------------------- bit crusher

/// Fewer bits and a lower sample rate: lo-fi grit.
pub struct BitCrusher {
    bits: f32,
    divide: f32,
    mix: f32,
    hold: [f32; 2],
    count: f32,
}

impl Default for BitCrusher {
    fn default() -> Self {
        Self {
            bits: BITCRUSHER_PARAMS[0].default,
            divide: BITCRUSHER_PARAMS[1].default,
            mix: BITCRUSHER_PARAMS[2].default,
            hold: [0.0; 2],
            count: 0.0,
        }
    }
}

impl Effect for BitCrusher {
    fn prepare(&mut self, _sample_rate: f64) {
        self.hold = [0.0; 2];
        self.count = 0.0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.bits = value,
            1 => self.divide = value.max(1.0),
            2 => self.mix = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let levels = 2f32.powf(self.bits.round().clamp(1.0, 16.0) - 1.0);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // Take a new sample every `divide` samples and hold it in between.
            self.count -= 1.0;
            if self.count <= 0.0 {
                self.hold = [
                    (*l * levels).round() / levels,
                    (*r * levels).round() / levels,
                ];
                self.count += self.divide;
            }
            *l = *l * (1.0 - self.mix) + self.hold[0] * self.mix;
            *r = *r * (1.0 - self.mix) + self.hold[1] * self.mix;
        }
    }
}

// ---------------------------------------------------------------------------- distortion

/// Soft clip, hard clip or wavefold, then a tone filter.
pub struct Distortion {
    kind: usize,
    drive: f32,
    tone: f32,
    mix: f32,
    lp: [f32; 2],
    sr: f32,
}

impl Default for Distortion {
    fn default() -> Self {
        Self {
            kind: 0,
            drive: DISTORTION_PARAMS[1].default,
            tone: DISTORTION_PARAMS[2].default,
            mix: DISTORTION_PARAMS[3].default,
            lp: [0.0; 2],
            sr: 48_000.0,
        }
    }
}

impl Effect for Distortion {
    fn prepare(&mut self, sample_rate: f64) {
        self.sr = sample_rate as f32;
        self.lp = [0.0; 2];
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.kind = value.round().max(0.0) as usize,
            1 => self.drive = value,
            2 => self.tone = value,
            3 => self.mix = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let gain = 1.0 + self.drive * 20.0;
        // Louder input means more distortion, not a louder output.
        let comp = 1.0 / gain.sqrt();
        let k = 1.0 - (-TAU * self.tone / self.sr).exp();
        let shape = |x: f32| match self.kind {
            1 => (x * gain).clamp(-0.8, 0.8) * comp,
            2 => (x * gain * 1.5).sin() * comp,
            _ => (x * gain).tanh() * comp,
        };
        for (ch, buf) in [left, right].into_iter().enumerate() {
            for sample in buf.iter_mut() {
                self.lp[ch] += (shape(*sample) - self.lp[ch]) * k;
                *sample = *sample * (1.0 - self.mix) + self.lp[ch] * self.mix;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (i as f32 / SR * freq * TAU).sin() * 0.5)
            .collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        let tail = &buf[buf.len() / 2..];
        (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt()
    }

    fn run(effect: &mut dyn Effect, input: &[f32]) -> Vec<f32> {
        let (mut l, mut r) = (input.to_vec(), input.to_vec());
        effect.prepare(f64::from(SR));
        effect.process(&mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
        l
    }

    fn eq_with(settings: &[(usize, f32)]) -> Eq {
        let mut eq = Eq::default();
        eq.prepare(f64::from(SR));
        for &(i, v) in settings {
            eq.set_param(i, v);
        }
        eq
    }

    #[test]
    fn the_eq_boosts_and_cuts_where_you_put_the_bands() {
        let gain = |freq: f32, settings: &[(usize, f32)]| {
            let input = sine(freq, 19_200);
            let out = run(&mut eq_with(settings), &input);
            20.0 * (rms(&out) / rms(&input)).log10()
        };
        // A flat EQ changes nothing.
        assert!(gain(1000.0, &[]).abs() < 0.05);
        // Bell: +9 dB at 1 kHz, little effect two octaves away.
        let bell = [(2, 1000.0), (3, 9.0)];
        assert!((gain(1000.0, &bell) - 9.0).abs() < 0.5);
        assert!(gain(100.0, &bell).abs() < 1.0);
        // Narrower Q affects less of the neighbourhood.
        let narrow = [(2, 1000.0), (3, 9.0), (4, 8.0)];
        assert!(gain(1500.0, &narrow) < gain(1500.0, &bell) - 2.0);
        // Shelves.
        assert!(
            gain(40.0, &[(0, 150.0), (1, 12.0)]) > 9.0,
            "low shelf boost"
        );
        assert!(
            gain(12_000.0, &[(8, 6000.0), (9, -12.0)]) < -9.0,
            "high shelf cut"
        );
        assert!(
            gain(1000.0, &[(0, 150.0), (1, 12.0)]).abs() < 1.0,
            "and not in the middle"
        );
    }

    #[test]
    fn the_curve_the_editor_draws_is_what_the_filter_does() {
        let settings = [
            (0, 120.0),
            (1, 5.0),
            (2, 800.0),
            (3, -7.0),
            (4, 2.0),
            (5, 4000.0),
            (6, 6.0),
            (7, 0.7),
            (8, 9000.0),
            (9, -4.0),
        ];
        let mut values = [0.0; 10];
        for (v, p) in values.iter_mut().zip(&EQ_PARAMS) {
            *v = p.default;
        }
        for &(i, v) in &settings {
            values[i] = v;
        }
        for freq in [60.0, 120.0, 400.0, 800.0, 2500.0, 4000.0, 9000.0, 15_000.0] {
            let input = sine(freq, 19_200);
            let out = run(&mut eq_with(&settings), &input);
            let measured = 20.0 * (rms(&out) / rms(&input)).log10();
            let drawn = eq_response_db(&values, SR, freq);
            assert!(
                (measured - drawn).abs() < 0.3,
                "{freq} Hz: {measured} vs {drawn}"
            );
        }
        assert!(
            eq_response_db(&[], SR, 1000.0).abs() < 1e-3,
            "no settings is flat"
        );
    }

    #[test]
    fn chorus_and_phaser_change_the_sound_and_a_zero_mix_doesnt() {
        let input = sine(440.0, 19_200);
        for (name, mix_index) in [("chorus", 2usize), ("phaser", 3)] {
            let make = |mix: f32| -> Box<dyn Effect> {
                let mut e: Box<dyn Effect> = if name == "chorus" {
                    Box::new(Chorus::default())
                } else {
                    Box::new(Phaser::default())
                };
                e.set_param(mix_index, mix);
                e
            };
            assert_ne!(run(make(0.5).as_mut(), &input), input, "{name}");
            assert_eq!(run(make(0.0).as_mut(), &input), input, "{name} at zero mix");
        }
    }

    #[test]
    fn the_bit_crusher_quantises_and_holds() {
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin() * 0.9).collect();
        let mut crusher = BitCrusher::default();
        crusher.set_param(0, 3.0);
        let out = run(&mut crusher, &input);
        let mut levels: Vec<i32> = out.iter().map(|x| (x * 1000.0).round() as i32).collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(
            levels.len() <= 9,
            "3 bits is a handful of levels, got {}",
            levels.len()
        );
        // Dividing the rate holds each value for several samples.
        let mut slow = BitCrusher::default();
        slow.set_param(0, 16.0);
        slow.set_param(1, 8.0);
        let out = run(&mut slow, &input);
        let changes = out.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes <= input.len() / 8 + 2, "{changes} changes");
        // No mix is the dry sound.
        let mut dry = BitCrusher::default();
        dry.set_param(2, 0.0);
        assert_eq!(run(&mut dry, &input), input);
    }

    #[test]
    fn distortion_adds_harmonics_and_its_types_differ() {
        let input = sine(220.0, 19_200);
        let out = |kind: f32, drive: f32| {
            let mut d = Distortion::default();
            d.set_param(0, kind);
            d.set_param(1, drive);
            d.set_param(2, 18_000.0);
            run(&mut d, &input)
        };
        // A clean sine is smooth; distortion makes the waveform change faster.
        // The energy at three times the frequency, as a share of the energy at the frequency:
        // a clean sine has almost none, a distorted one a lot.
        let third = |b: &[f32]| {
            let power = |freq: f32| {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, x) in b.iter().enumerate() {
                    let phase = TAU * freq * i as f32 / SR;
                    re += x * phase.cos();
                    im += x * phase.sin();
                }
                re * re + im * im
            };
            power(660.0) / power(220.0).max(1e-9)
        };
        assert!(third(&input) < 1e-4);
        assert!(third(&out(0.0, 0.9)) > 0.01, "{}", third(&out(0.0, 0.9)));
        assert_ne!(out(0.0, 0.6), out(1.0, 0.6));
        assert_ne!(out(1.0, 0.6), out(2.0, 0.6));
        let loud = out(0.0, 1.0);
        assert!(
            loud.iter().all(|x| x.abs() < 1.0),
            "never louder than full scale"
        );
        let mut tone = Distortion::default();
        tone.set_param(1, 0.9);
        tone.set_param(2, 300.0);
        let dark = run(&mut tone, &input);
        assert!(
            third(&dark) < third(&out(0.0, 0.9)),
            "a low tone smooths it"
        );
    }
}
