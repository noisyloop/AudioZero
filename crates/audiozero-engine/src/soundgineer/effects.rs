//! Soundgineer's effects rack: chorus, phaser, flanger, delay, reverb, EQ, compressor and
//! distortion, run over the summed voices. Every buffer is allocated in `new` (while the
//! engine is built), so processing never allocates.

use std::f32::consts::{PI, TAU};

/// Kills denormals, which would otherwise slow feedback loops to a crawl.
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

// ---------------------------------------------------------------------------- delay line

struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0.0; size.max(4)],
            pos: 0,
        }
    }

    fn write(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }

    /// `delay` samples back, interpolated.
    fn read(&self, delay: f32) -> f32 {
        let size = self.buf.len();
        let p = (self.pos as f32 - delay).rem_euclid(size as f32);
        let i0 = (p as usize).min(size - 1);
        let frac = p - i0 as f32;
        let i1 = (i0 + 1) % size;
        self.buf[i0] + (self.buf[i1] - self.buf[i0]) * frac
    }
}

// ---------------------------------------------------------------------------- chorus

pub struct Chorus {
    lines: [DelayLine; 2],
    phase: f32,
    sr: f32,
}

impl Chorus {
    pub fn new(sr: f32) -> Self {
        let size = (sr * 0.2) as usize;
        Self {
            lines: [DelayLine::new(size), DelayLine::new(size)],
            phase: 0.0,
            sr,
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], rate: f32, depth: f32, mix: f32) {
        let base = 0.02 * self.sr;
        let dep = depth * 0.008 * self.sr;
        let inc = rate / self.sr;
        for (l, r) in l.iter_mut().zip(r.iter_mut()) {
            self.phase = (self.phase + inc).fract();
            let lfo_l = (self.phase * TAU).sin();
            let lfo_r = ((self.phase + 0.25) * TAU).sin();
            self.lines[0].write(*l);
            self.lines[1].write(*r);
            let wl = self.lines[0].read(base + dep * (1.0 + lfo_l) + 1.0);
            let wr = self.lines[1].read(base + dep * (1.0 + lfo_r) + 1.0);
            *l = *l * (1.0 - mix) + wl * mix;
            *r = *r * (1.0 - mix) + wr * mix;
        }
    }
}

// ---------------------------------------------------------------------------- flanger

pub struct Flanger {
    lines: [DelayLine; 2],
    feedback: [f32; 2],
    phase: f32,
    sr: f32,
}

impl Flanger {
    pub fn new(sr: f32) -> Self {
        let size = (sr * 0.1) as usize;
        Self {
            lines: [DelayLine::new(size), DelayLine::new(size)],
            feedback: [0.0; 2],
            phase: 0.0,
            sr,
        }
    }

    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        rate: f32,
        depth: f32,
        feedback: f32,
        mix: f32,
    ) {
        let min = 0.001 * self.sr;
        let dep = depth * 0.006 * self.sr;
        let inc = rate / self.sr;
        for i in 0..l.len().min(r.len()) {
            self.phase = (self.phase + inc).fract();
            let sweep = (1.0 - (self.phase * TAU).cos()) * 0.5;
            let delay = min + dep * sweep + 1.0;
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                self.lines[ch].write(buf[i] + self.feedback[ch] * feedback);
                let wet = self.lines[ch].read(delay);
                self.feedback[ch] = flush(wet);
                buf[i] = buf[i] * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---------------------------------------------------------------------------- phaser

pub struct Phaser {
    stages: [[f32; 6]; 2],
    feedback: [f32; 2],
    phase: f32,
    sr: f32,
}

impl Phaser {
    pub fn new(sr: f32) -> Self {
        Self {
            stages: [[0.0; 6]; 2],
            feedback: [0.0; 2],
            phase: 0.0,
            sr,
        }
    }

    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        rate: f32,
        depth: f32,
        feedback: f32,
        mix: f32,
    ) {
        let inc = rate / self.sr;
        for i in 0..l.len().min(r.len()) {
            self.phase = (self.phase + inc).fract();
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                let sweep = (1.0 - ((self.phase + ch as f32 * 0.25) * TAU).cos()) * 0.5;
                let freq = 300.0 * 2f32.powf(sweep * depth * 4.5);
                let w = (PI * freq / self.sr).min(1.5);
                let a = (1.0 - w.tan()) / (1.0 + w.tan());
                let mut x = buf[i] + self.feedback[ch] * feedback;
                for stage in &mut self.stages[ch] {
                    let y = a * x + *stage;
                    *stage = x - a * y;
                    x = y;
                }
                self.feedback[ch] = flush(x);
                buf[i] = buf[i] * (1.0 - mix) + x * mix;
            }
        }
    }
}

// ---------------------------------------------------------------------------- delay

pub struct StereoDelay {
    lines: [DelayLine; 2],
    smoothed: f32,
    sr: f32,
}

impl StereoDelay {
    pub fn new(sr: f32) -> Self {
        let size = (sr * 2.5).ceil() as usize;
        Self {
            lines: [DelayLine::new(size), DelayLine::new(size)],
            smoothed: sr * 0.35,
            sr,
        }
    }

    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        time: f32,
        feedback: f32,
        ping_pong: bool,
        mix: f32,
    ) {
        let target = (time * self.sr).clamp(32.0, self.sr * 2.4);
        for (l, r) in l.iter_mut().zip(r.iter_mut()) {
            // The time glides, so changing it doesn't click.
            self.smoothed += (target - self.smoothed) * 0.0005;
            let d = self.smoothed;
            let (wl, wr) = (self.lines[0].read(d), self.lines[1].read(d));
            if ping_pong {
                self.lines[0].write(flush(*l * 0.5 + *r * 0.5 + wr * feedback));
                self.lines[1].write(flush(wl * feedback));
            } else {
                self.lines[0].write(flush(*l + wl * feedback));
                self.lines[1].write(flush(*r + wr * feedback));
            }
            *l = *l * (1.0 - mix * 0.5) + wl * mix;
            *r = *r * (1.0 - mix * 0.5) + wr * mix;
        }
    }
}

// ---------------------------------------------------------------------------- reverb

/// A Freeverb-style reverb: eight parallel combs and four allpasses per side.
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0.0; size.max(1)],
            pos: 0,
            store: 0.0,
        }
    }

    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = flush(out * (1.0 - damp) + self.store * damp);
        self.buf[self.pos] = flush(x + self.store * feedback);
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0.0; size.max(1)],
            pos: 0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let held = self.buf[self.pos];
        let out = -x + held;
        self.buf[self.pos] = flush(x + held * 0.5);
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44_100.0;
        let size = |t: usize, extra: usize| ((t + extra) as f32 * scale).round() as usize;
        Self {
            combs: [0, STEREO_SPREAD].map(|extra| {
                COMB_TUNINGS
                    .iter()
                    .map(|&t| Comb::new(size(t, extra)))
                    .collect()
            }),
            allpasses: [0, STEREO_SPREAD].map(|extra| {
                ALLPASS_TUNINGS
                    .iter()
                    .map(|&t| Allpass::new(size(t, extra)))
                    .collect()
            }),
        }
    }

    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        size: f32,
        damp: f32,
        width: f32,
        mix: f32,
    ) {
        let feedback = 0.7 + size * 0.28;
        let damp = damp * 0.4;
        let wet1 = mix * (1.0 + width) / 2.0;
        let wet2 = mix * (1.0 - width) / 2.0;
        for (l, r) in l.iter_mut().zip(r.iter_mut()) {
            let input = (*l + *r) * 0.015;
            let (mut out_l, mut out_r) = (0.0, 0.0);
            for c in 0..8 {
                out_l += self.combs[0][c].process(input, feedback, damp);
                out_r += self.combs[1][c].process(input, feedback, damp);
            }
            for a in 0..4 {
                out_l = self.allpasses[0][a].process(out_l);
                out_r = self.allpasses[1][a].process(out_r);
            }
            *l = *l * (1.0 - mix) + out_l * wet1 + out_r * wet2;
            *r = *r * (1.0 - mix) + out_r * wet1 + out_l * wet2;
        }
    }
}

// ---------------------------------------------------------------------------- EQ

/// A biquad filter (RBJ cookbook), two channels.
#[derive(Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x1: [f32; 2],
    x2: [f32; 2],
    y1: [f32; 2],
    y2: [f32; 2],
}

impl Biquad {
    fn identity() -> Self {
        Self {
            b: [1.0, 0.0, 0.0],
            ..Default::default()
        }
    }

    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    fn low_shelf(&mut self, sr: f32, f: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = TAU * f / sr;
        let (cs, sn) = (w.cos(), w.sin());
        let beta = a.sqrt() / 0.9;
        self.set(
            a * (a + 1.0 - (a - 1.0) * cs + beta * sn),
            2.0 * a * (a - 1.0 - (a + 1.0) * cs),
            a * (a + 1.0 - (a - 1.0) * cs - beta * sn),
            a + 1.0 + (a - 1.0) * cs + beta * sn,
            -2.0 * (a - 1.0 + (a + 1.0) * cs),
            a + 1.0 + (a - 1.0) * cs - beta * sn,
        );
    }

    fn high_shelf(&mut self, sr: f32, f: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = TAU * f / sr;
        let (cs, sn) = (w.cos(), w.sin());
        let beta = a.sqrt() / 0.9;
        self.set(
            a * (a + 1.0 + (a - 1.0) * cs + beta * sn),
            -2.0 * a * (a - 1.0 + (a + 1.0) * cs),
            a * (a + 1.0 + (a - 1.0) * cs - beta * sn),
            a + 1.0 - (a - 1.0) * cs + beta * sn,
            2.0 * (a - 1.0 - (a + 1.0) * cs),
            a + 1.0 - (a - 1.0) * cs - beta * sn,
        );
    }

    fn peak(&mut self, sr: f32, f: f32, gain_db: f32, q: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = TAU * f / sr;
        let alpha = w.sin() / (2.0 * q);
        let cs = w.cos();
        self.set(
            1.0 + alpha * a,
            -2.0 * cs,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cs,
            1.0 - alpha / a,
        );
    }

    fn process(&mut self, x: f32, ch: usize) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x1[ch] + self.b[2] * self.x2[ch]
            - self.a[0] * self.y1[ch]
            - self.a[1] * self.y2[ch];
        self.x2[ch] = self.x1[ch];
        self.x1[ch] = x;
        self.y2[ch] = self.y1[ch];
        self.y1[ch] = flush(y);
        y
    }
}

/// Low shelf at 250 Hz, a peak you can move, and a high shelf at 4 kHz.
pub struct Eq3 {
    low: Biquad,
    mid: Biquad,
    high: Biquad,
    sr: f32,
}

impl Eq3 {
    pub fn new(sr: f32) -> Self {
        Self {
            low: Biquad::identity(),
            mid: Biquad::identity(),
            high: Biquad::identity(),
            sr,
        }
    }

    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        low_db: f32,
        mid_db: f32,
        mid_freq: f32,
        high_db: f32,
    ) {
        self.low.low_shelf(self.sr, 250.0, low_db);
        self.mid.peak(self.sr, mid_freq, mid_db, 0.7);
        self.high.high_shelf(self.sr, 4000.0, high_db);
        for (l, r) in l.iter_mut().zip(r.iter_mut()) {
            *l = self
                .high
                .process(self.mid.process(self.low.process(*l, 0), 0), 0);
            *r = self
                .high
                .process(self.mid.process(self.low.process(*r, 1), 1), 1);
        }
    }
}

// ---------------------------------------------------------------------------- compressor

pub struct Compressor {
    env: f32,
    sr: f32,
}

impl Compressor {
    pub fn new(sr: f32) -> Self {
        Self { env: 0.0, sr }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        threshold_db: f32,
        ratio: f32,
        attack: f32,
        release: f32,
        makeup_db: f32,
    ) {
        let attack_coef = (-1.0 / (attack * self.sr)).exp();
        let release_coef = (-1.0 / (release * self.sr)).exp();
        let makeup = 10f32.powf(makeup_db / 20.0);
        for (l, r) in l.iter_mut().zip(r.iter_mut()) {
            let peak = l.abs().max(r.abs());
            let coef = if peak > self.env {
                attack_coef
            } else {
                release_coef
            };
            self.env = flush(coef * self.env + (1.0 - coef) * peak);
            let env_db = 20.0 * self.env.max(1e-6).log10();
            let gain_db = if env_db > threshold_db {
                (threshold_db - env_db) * (1.0 - 1.0 / ratio)
            } else {
                0.0
            };
            let gain = 10f32.powf(gain_db / 20.0) * makeup;
            *l *= gain;
            *r *= gain;
        }
    }
}

// ---------------------------------------------------------------------------- distortion

pub struct Distortion {
    lp: [f32; 2],
    sr: f32,
}

impl Distortion {
    pub fn new(sr: f32) -> Self {
        Self { lp: [0.0; 2], sr }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], drive: f32, tone: f32, mix: f32) {
        let gain = 1.0 + drive * 30.0;
        let comp = 1.0 / gain.powf(0.6);
        let k = 1.0 - (-TAU * tone / self.sr).exp();
        for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
            for sample in buf.iter_mut() {
                let shaped = (*sample * gain).tanh() * comp;
                self.lp[ch] += (shaped - self.lp[ch]) * k;
                *sample = *sample * (1.0 - mix) + self.lp[ch] * mix;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn burst() -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0; 24_000];
        let mut r = vec![0.0; 24_000];
        l[0] = 1.0;
        r[0] = 1.0;
        (l, r)
    }

    fn energy(buf: &[f32]) -> f32 {
        buf.iter().map(|x| x * x).sum()
    }

    fn after(buf: &[f32], seconds: f32) -> &[f32] {
        &buf[(seconds * SR) as usize..]
    }

    fn all_finite(l: &[f32], r: &[f32]) -> bool {
        l.iter().chain(r).all(|x| x.is_finite() && x.abs() < 100.0)
    }

    #[test]
    fn the_delay_repeats_the_sound_after_its_time() {
        let mut delay = StereoDelay::new(SR);
        delay.smoothed = 0.25 * SR; // already there, no glide
        let (mut l, mut r) = burst();
        delay.process(&mut l, &mut r, 0.25, 0.5, false, 0.5);
        let echo = (0.25 * SR) as usize;
        assert!(
            l[echo - 2..echo + 2].iter().any(|x| x.abs() > 0.3),
            "the echo arrives"
        );
        assert!(
            l[echo + 100..echo + 1000].iter().all(|x| x.abs() < 0.01),
            "and only then"
        );
        assert!(all_finite(&l, &r));
        // Ping-pong sends the repeats to the other side.
        let mut pp = StereoDelay::new(SR);
        pp.smoothed = 0.1 * SR;
        let (mut l, mut r) = burst();
        r.fill(0.0);
        pp.process(&mut l, &mut r, 0.1, 0.5, true, 0.5);
        let first = (0.1 * SR) as usize;
        assert!(
            l[first - 2..first + 2].iter().any(|x| x.abs() > 0.1),
            "the first echo is left"
        );
        assert!(r[first - 2..first + 2].iter().all(|x| x.abs() < 0.01));
        assert!(
            r[2 * first - 2..2 * first + 2]
                .iter()
                .any(|x| x.abs() > 0.05),
            "the next is right"
        );
    }

    #[test]
    fn the_reverb_rings_on_and_dies_away() {
        let mut reverb = Reverb::new(SR);
        let (mut l, mut r) = burst();
        reverb.process(&mut l, &mut r, 0.7, 0.5, 1.0, 0.5);
        assert!(all_finite(&l, &r));
        assert!(energy(after(&l, 0.2)) > 1e-6, "a tail after 200 ms");
        assert!(
            energy(after(&l, 0.3)) > energy(after(&l, 0.45)),
            "that fades"
        );
        // A bigger room rings longer.
        let tail = |size| {
            let mut v = Reverb::new(SR);
            let (mut l, mut r) = burst();
            v.process(&mut l, &mut r, size, 0.5, 1.0, 0.5);
            energy(after(&l, 0.3))
        };
        assert!(tail(1.0) > 3.0 * tail(0.0));
        let mut dry = Reverb::new(SR);
        let (mut l, mut r) = burst();
        dry.process(&mut l, &mut r, 0.7, 0.5, 1.0, 0.0);
        assert!(l[0] == 1.0 && energy(&l[1..]) == 0.0, "no mix is untouched");
    }

    #[test]
    fn modulation_effects_stay_finite_and_change_the_sound() {
        let source: Vec<f32> = (0..24_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let run = |apply: &dyn Fn(&mut [f32], &mut [f32])| {
            let (mut l, mut r) = (source.clone(), source.clone());
            apply(&mut l, &mut r);
            assert!(all_finite(&l, &r));
            assert_ne!(l, source);
            l
        };
        run(&|l, r| Chorus::new(SR).process(l, r, 0.4, 0.5, 0.5));
        run(&|l, r| Phaser::new(SR).process(l, r, 0.3, 0.7, 0.9, 0.5));
        run(&|l, r| Flanger::new(SR).process(l, r, 0.25, 0.6, 0.9, 0.5));
        run(&|l, r| Distortion::new(SR).process(l, r, 0.8, 4000.0, 1.0));
        // A flat mix leaves the sound alone.
        let (mut l, mut r) = (source.clone(), source.clone());
        Chorus::new(SR).process(&mut l, &mut r, 0.4, 0.5, 0.0);
        assert_eq!(l, source);
    }

    #[test]
    fn the_eq_boosts_and_cuts_its_bands() {
        let tone = |f: f32| -> Vec<f32> {
            (0..9_600)
                .map(|i| (i as f32 / SR * f * TAU).sin())
                .collect()
        };
        let gain = |f: f32, low: f32, mid: f32, high: f32| {
            let mut eq = Eq3::new(SR);
            let mut l = tone(f);
            let mut r = l.clone();
            eq.process(&mut l, &mut r, low, mid, 1000.0, high);
            (energy(&l[4_800..]) / energy(&tone(f)[4_800..])).sqrt()
        };
        assert!(gain(80.0, 12.0, 0.0, 0.0) > 2.5, "low boost");
        assert!(gain(80.0, -12.0, 0.0, 0.0) < 0.4, "low cut");
        assert!(gain(1000.0, 0.0, 12.0, 0.0) > 2.5, "mid boost");
        assert!(gain(10_000.0, 0.0, 0.0, 12.0) > 2.5, "high boost");
        assert!(
            (gain(500.0, 0.0, 0.0, 0.0) - 1.0).abs() < 0.05,
            "flat is flat"
        );
    }

    #[test]
    fn the_compressor_turns_loud_sounds_down() {
        let mut comp = Compressor::new(SR);
        let mut l = vec![0.9; 24_000];
        let mut r = l.clone();
        comp.process(&mut l, &mut r, -20.0, 8.0, 0.001, 0.1, 0.0);
        assert!(l[23_000] < 0.5, "squashed, got {}", l[23_000]);
        let mut quiet = vec![0.01; 24_000];
        let mut r = quiet.clone();
        Compressor::new(SR).process(&mut quiet, &mut r, -20.0, 8.0, 0.001, 0.1, 0.0);
        assert!(
            (quiet[23_000] - 0.01).abs() < 1e-3,
            "quiet sounds are left alone"
        );
        let mut made_up = vec![0.01; 24_000];
        let mut r = made_up.clone();
        Compressor::new(SR).process(&mut made_up, &mut r, -20.0, 8.0, 0.001, 0.1, 12.0);
        assert!(made_up[23_000] > 0.03, "makeup gain");
    }
}
