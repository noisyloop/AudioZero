//! Built-in track effects. Each processes a track's audio in place, after its instrument and
//! audio clips and before its volume and pan.
//!
//! Buffers are allocated in [`Effect::prepare`], on the UI thread while the engine is built, so
//! [`Effect::process`] and [`Effect::set_param`] never allocate. Each effect describes its
//! parameters ([`params_for`]); projects store them by name, and [`crate::Project::sanitize`]
//! clamps them to these ranges.

/// Most parameters one effect has.
pub const MAX_PARAMS: usize = 4;

/// One of an effect's parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    /// The name projects store it under.
    pub name: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
}

const fn param(
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> Param {
    Param {
        name,
        label,
        min,
        max,
        default,
        unit,
    }
}

const DELAY_PARAMS: [Param; 3] = [
    param("time", "Time", 0.01, 2.0, 0.375, "s"),
    param("feedback", "Feedback", 0.0, 0.95, 0.45, ""),
    param("mix", "Mix", 0.0, 1.0, 0.35, ""),
];
const REVERB_PARAMS: [Param; 3] = [
    param("size", "Size", 0.0, 1.0, 0.6, ""),
    param("damping", "Damping", 0.0, 1.0, 0.25, ""),
    param("mix", "Mix", 0.0, 1.0, 0.25, ""),
];
const COMPRESSOR_PARAMS: [Param; 3] = [
    param("threshold", "Threshold", -60.0, 0.0, -18.0, "dB"),
    param("ratio", "Ratio", 1.0, 20.0, 4.0, ":1"),
    param("makeup", "Makeup", 0.0, 24.0, 6.0, "dB"),
];

/// Built-in effects: project id and display name.
pub const EFFECTS: [(&str, &str); 3] = [
    ("delay", "Delay"),
    ("reverb", "Reverb"),
    ("compressor", "Compressor"),
];

/// The parameters of a built-in effect, or `None` if `id` isn't one.
pub fn params_for(id: &str) -> Option<&'static [Param]> {
    match id {
        "delay" => Some(&DELAY_PARAMS),
        "reverb" => Some(&REVERB_PARAMS),
        "compressor" => Some(&COMPRESSOR_PARAMS),
        _ => None,
    }
}

/// An effect on a track.
pub trait Effect: Send {
    fn prepare(&mut self, sample_rate: f64);
    /// Sets parameter `index` (in [`params_for`] order) to `value`, already in range.
    fn set_param(&mut self, index: usize, value: f32);
    /// Processes `left` and `right` (the same length) in place.
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);
}

/// Creates a built-in effect by its project id, with its default settings.
pub fn create_effect(id: &str) -> Option<Box<dyn Effect>> {
    let mut effect: Box<dyn Effect> = match id {
        "delay" => Box::new(Delay::default()),
        "reverb" => Box::new(Reverb::default()),
        "compressor" => Box::new(Compressor::default()),
        _ => return None,
    };
    for (i, p) in params_for(id)?.iter().enumerate() {
        effect.set_param(i, p.default);
    }
    Some(effect)
}

/// The display name of a built-in effect id.
pub fn effect_name(id: &str) -> &str {
    EFFECTS
        .iter()
        .find(|(i, _)| *i == id)
        .map_or(id, |(_, name)| name)
}

/// A stereo ping-pong delay with feedback.
#[derive(Default)]
pub struct Delay {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
    sample_rate: f64,
    time: f32,
    feedback: f32,
    mix: f32,
}

impl Delay {
    fn length(&self) -> usize {
        ((f64::from(self.time) * self.sample_rate) as usize).clamp(1, self.left.len().max(1))
    }
}

impl Effect for Delay {
    fn prepare(&mut self, sample_rate: f64) {
        // Room for the longest time, so changing it never allocates.
        let max = (f64::from(DELAY_PARAMS[0].max) * sample_rate) as usize + 1;
        self.left = vec![0.0; max];
        self.right = vec![0.0; max];
        self.pos = 0;
        self.sample_rate = sample_rate;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.time = value,
            1 => self.feedback = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.left.is_empty() {
            return;
        }
        let size = self.left.len();
        let length = self.length();
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let read = (self.pos + size - length) % size;
            let (dl, dr) = (self.left[read], self.right[read]);
            // Ping-pong: the input goes into the left line, which feeds the right, and back.
            self.left[self.pos] = 0.5 * (*l + *r) + dr * self.feedback;
            self.right[self.pos] = dl * self.feedback;
            *l += dl * self.mix;
            *r += dr * self.mix;
            self.pos = (self.pos + 1) % size;
        }
    }
}

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            pos: 0,
            store: 0.0,
        }
    }
    fn tick(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = input + self.store * feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }
    fn tick(&mut self, input: f32) -> f32 {
        let delayed = self.buf[self.pos];
        self.buf[self.pos] = input + delayed * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        delayed - input
    }
}

/// A Freeverb-style room: parallel combs into series allpasses, per channel.
#[derive(Default)]
pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    feedback: f32,
    damping: f32,
    mix: f32,
}

impl Reverb {
    const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
    const ALLPASSES: [usize; 2] = [556, 441];
    /// Extra delay on the right channel, for width.
    const SPREAD: usize = 23;
    const INPUT: f32 = 0.015;
}

impl Effect for Reverb {
    fn prepare(&mut self, sample_rate: f64) {
        let scale = sample_rate / 44_100.0;
        let len = |n: usize, side: usize| ((n + side * Self::SPREAD) as f64 * scale) as usize;
        for side in 0..2 {
            self.combs[side] = Self::COMBS
                .iter()
                .map(|&n| Comb::new(len(n, side)))
                .collect();
            self.allpasses[side] = Self::ALLPASSES
                .iter()
                .map(|&n| Allpass::new(len(n, side)))
                .collect();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            // Size 0..1 maps onto Freeverb's useful feedback range.
            0 => self.feedback = 0.7 + 0.28 * value,
            1 => self.damping = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let input = (*l + *r) * Self::INPUT;
            let mut wet = [0.0f32; 2];
            for (side, out) in wet.iter_mut().enumerate() {
                let mut sum = 0.0;
                for comb in &mut self.combs[side] {
                    sum += comb.tick(input, self.feedback, self.damping);
                }
                for allpass in &mut self.allpasses[side] {
                    sum = allpass.tick(sum);
                }
                *out = sum;
            }
            *l += wet[0] * self.mix * 3.0;
            *r += wet[1] * self.mix * 3.0;
        }
    }
}

/// A feed-forward compressor.
#[derive(Default)]
pub struct Compressor {
    envelope: f32,
    attack: f32,
    release: f32,
    threshold_db: f32,
    ratio: f32,
    makeup_db: f32,
}

impl Effect for Compressor {
    fn prepare(&mut self, sample_rate: f64) {
        let coeff = |seconds: f64| (-1.0 / (seconds * sample_rate)).exp() as f32;
        self.attack = coeff(0.010);
        self.release = coeff(0.150);
        self.envelope = 0.0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.threshold_db = value,
            1 => self.ratio = value.max(1.0),
            2 => self.makeup_db = value,
            _ => {}
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let level = l.abs().max(r.abs());
            let coeff = if level > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = level + coeff * (self.envelope - level);
            let level_db = 20.0 * self.envelope.max(1e-6).log10();
            let over = (level_db - self.threshold_db).max(0.0);
            let gain_db = self.makeup_db - over * (1.0 - 1.0 / self.ratio);
            let gain = 10f32.powf(gain_db / 20.0);
            *l *= gain;
            *r *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_response(effect: &mut Box<dyn Effect>, frames: usize) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        l[0] = 1.0;
        r[0] = 1.0;
        effect.process(&mut l, &mut r);
        l
    }

    fn made(id: &str) -> Box<dyn Effect> {
        let mut effect = create_effect(id).unwrap();
        effect.prepare(48_000.0);
        effect
    }

    #[test]
    fn every_effect_is_creatable_named_and_described() {
        for (id, name) in EFFECTS {
            assert!(create_effect(id).is_some());
            assert_eq!(effect_name(id), name);
            let params = params_for(id).unwrap();
            assert!(params.len() <= MAX_PARAMS);
            for p in params {
                assert!(p.min <= p.default && p.default <= p.max, "{id}.{}", p.name);
            }
        }
        assert!(create_effect("nope").is_none() && params_for("nope").is_none());
    }

    #[test]
    fn delay_repeats_after_its_time_and_the_time_can_change() {
        let mut delay = made("delay");
        let out = impulse_response(&mut delay, 48_000);
        let at = (0.375 * 48_000.0) as usize;
        assert!(out[at].abs() > 0.1, "first echo at the default time");
        assert!(out[at / 2].abs() < 1e-6, "silence in between");

        let mut delay = made("delay");
        delay.set_param(0, 0.1);
        let out = impulse_response(&mut delay, 48_000);
        assert!(out[4_800].abs() > 0.1, "echo after 0.1 s");
    }

    #[test]
    fn reverb_tails_off_stays_finite_and_mix_zero_is_dry() {
        let mut reverb = made("reverb");
        let out = impulse_response(&mut reverb, 96_000);
        assert!(out.iter().all(|x| x.is_finite()));
        let energy = |r: std::ops::Range<usize>| out[r].iter().map(|x| x * x).sum::<f32>();
        assert!(energy(2_000..20_000) > 0.0, "there's a tail");
        assert!(
            energy(80_000..96_000) < energy(2_000..20_000),
            "and it decays"
        );

        let mut dry = made("reverb");
        dry.set_param(2, 0.0);
        let out = impulse_response(&mut dry, 4_000);
        assert!(out[1..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn compressor_turns_loud_signals_down_more_than_quiet_ones() {
        let mut c = made("compressor");
        let run = |c: &mut Box<dyn Effect>, amp: f32| {
            let (mut l, mut r) = (vec![amp; 48_000], vec![amp; 48_000]);
            c.process(&mut l, &mut r);
            l[47_999] / amp
        };
        let quiet = run(&mut c, 0.01);
        let loud = run(&mut c, 0.9);
        assert!(
            loud < quiet,
            "loud gain {loud} should be below quiet gain {quiet}"
        );
        c.set_param(1, 1.0); // a 1:1 ratio doesn't compress
        c.set_param(2, 0.0);
        assert!((run(&mut c, 0.9) - 1.0).abs() < 1e-3);
    }
}
