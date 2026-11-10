//! Built-in track effects. Each processes a track's audio in place, after its instrument and
//! audio clips and before its volume and pan.
//!
//! Buffers are allocated in [`Effect::prepare`], on the UI thread while the engine is built, so
//! [`Effect::process`] never allocates. The effects have fixed settings for now; parameters come
//! with the device panel.

/// An effect on a track.
pub trait Effect: Send {
    fn prepare(&mut self, sample_rate: f64);
    /// Processes `left` and `right` (the same length) in place.
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);
}

/// Built-in effects: project id and display name.
pub const EFFECTS: [(&str, &str); 3] = [
    ("delay", "Delay"),
    ("reverb", "Reverb"),
    ("compressor", "Compressor"),
];

/// Creates a built-in effect by its project id.
pub fn create_effect(id: &str) -> Option<Box<dyn Effect>> {
    match id {
        "delay" => Some(Box::new(Delay::default())),
        "reverb" => Some(Box::new(Reverb::default())),
        "compressor" => Some(Box::new(Compressor::default())),
        _ => None,
    }
}

/// The display name of a built-in effect id.
pub fn effect_name(id: &str) -> &str {
    EFFECTS
        .iter()
        .find(|(i, _)| *i == id)
        .map_or(id, |(_, name)| name)
}

/// A stereo ping-pong delay: 3/8 of a second, with feedback.
#[derive(Default)]
pub struct Delay {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
}

impl Delay {
    const SECONDS: f64 = 0.375;
    const FEEDBACK: f32 = 0.45;
    const MIX: f32 = 0.35;
}

impl Effect for Delay {
    fn prepare(&mut self, sample_rate: f64) {
        let len = ((Self::SECONDS * sample_rate) as usize).max(1);
        self.left = vec![0.0; len];
        self.right = vec![0.0; len];
        self.pos = 0;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.left.is_empty() {
            return;
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (dl, dr) = (self.left[self.pos], self.right[self.pos]);
            // Ping-pong: the input goes into the left line, which feeds the right, and back.
            self.left[self.pos] = 0.5 * (*l + *r) + dr * Self::FEEDBACK;
            self.right[self.pos] = dl * Self::FEEDBACK;
            *l += dl * Self::MIX;
            *r += dr * Self::MIX;
            self.pos = (self.pos + 1) % self.left.len();
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
}

impl Reverb {
    const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
    const ALLPASSES: [usize; 2] = [556, 441];
    /// Extra delay on the right channel, for width.
    const SPREAD: usize = 23;
    const FEEDBACK: f32 = 0.84;
    const DAMP: f32 = 0.25;
    const MIX: f32 = 0.25;
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

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let input = (*l + *r) * Self::INPUT;
            let mut wet = [0.0f32; 2];
            for (side, out) in wet.iter_mut().enumerate() {
                let mut sum = 0.0;
                for comb in &mut self.combs[side] {
                    sum += comb.tick(input, Self::FEEDBACK, Self::DAMP);
                }
                for allpass in &mut self.allpasses[side] {
                    sum = allpass.tick(sum);
                }
                *out = sum;
            }
            *l += wet[0] * Self::MIX * 3.0;
            *r += wet[1] * Self::MIX * 3.0;
        }
    }
}

/// A feed-forward compressor that evens out a track: -18 dB threshold, 4:1, with makeup gain.
#[derive(Default)]
pub struct Compressor {
    envelope: f32,
    attack: f32,
    release: f32,
}

impl Compressor {
    const THRESHOLD_DB: f32 = -18.0;
    const RATIO: f32 = 4.0;
    const MAKEUP_DB: f32 = 6.0;
}

impl Effect for Compressor {
    fn prepare(&mut self, sample_rate: f64) {
        let coeff = |seconds: f64| (-1.0 / (seconds * sample_rate)).exp() as f32;
        self.attack = coeff(0.010);
        self.release = coeff(0.150);
        self.envelope = 0.0;
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
            let over = (level_db - Self::THRESHOLD_DB).max(0.0);
            let gain_db = Self::MAKEUP_DB - over * (1.0 - 1.0 / Self::RATIO);
            let gain = 10f32.powf(gain_db / 20.0);
            *l *= gain;
            *r *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_response(id: &str, frames: usize) -> Vec<f32> {
        let mut effect = create_effect(id).unwrap();
        effect.prepare(48_000.0);
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        l[0] = 1.0;
        r[0] = 1.0;
        effect.process(&mut l, &mut r);
        l
    }

    #[test]
    fn every_effect_is_creatable_and_named() {
        for (id, name) in EFFECTS {
            assert!(create_effect(id).is_some());
            assert_eq!(effect_name(id), name);
        }
        assert!(create_effect("nope").is_none());
    }

    #[test]
    fn delay_repeats_after_its_time() {
        let out = impulse_response("delay", 48_000);
        let at = (0.375 * 48_000.0) as usize;
        assert!(out[at].abs() > 0.1, "first echo");
        assert!(out[at / 2].abs() < 1e-6, "silence in between");
    }

    #[test]
    fn reverb_tails_off_and_stays_finite() {
        let out = impulse_response("reverb", 96_000);
        assert!(out.iter().all(|x| x.is_finite()));
        let energy = |r: std::ops::Range<usize>| out[r].iter().map(|x| x * x).sum::<f32>();
        assert!(energy(2_000..20_000) > 0.0, "there's a tail");
        assert!(
            energy(80_000..96_000) < energy(2_000..20_000),
            "and it decays"
        );
    }

    #[test]
    fn compressor_turns_loud_signals_down_more_than_quiet_ones() {
        let mut c = create_effect("compressor").unwrap();
        c.prepare(48_000.0);
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
    }
}
