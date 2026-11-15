//! Per-voice building blocks: the DAHDSR envelope, the LFO and the filters.

use std::f32::consts::PI;

// ---------------------------------------------------------------------------- envelope

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    /// A very fast fade, when a voice is stolen or the song jumps.
    Kill,
}

/// The settings of one envelope stage set, in seconds and 0 to 1.
#[derive(Clone, Copy, Default)]
pub struct EnvParams {
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub atk_curve: f32,
    pub dec_curve: f32,
    pub rel_curve: f32,
}

/// A bend for a 0-to-1 ramp: positive is a slow start, negative a fast one.
fn curve(t: f32, c: f32) -> f32 {
    t.powf(2f32.powf(c * 3.0))
}

/// Delay, attack, hold, decay, sustain, release, each stage with its own curve.
#[derive(Clone, Copy, Default)]
pub struct Envelope {
    stage: Stage,
    /// Seconds into the current stage.
    t: f32,
    level: f32,
    release_from: f32,
}

impl Envelope {
    pub fn trigger(&mut self) {
        self.stage = Stage::Delay;
        self.t = 0.0;
    }

    pub fn gate_off(&mut self) {
        if !matches!(self.stage, Stage::Idle | Stage::Kill | Stage::Release) {
            self.release_from = self.level;
            self.stage = Stage::Release;
            self.t = 0.0;
        }
    }

    /// Fades out fast (4 ms).
    pub fn kill(&mut self) {
        if self.stage != Stage::Idle {
            self.release_from = self.level;
            self.stage = Stage::Kill;
            self.t = 0.0;
        }
    }

    pub fn idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn releasing(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Kill)
    }

    pub fn value(&self) -> f32 {
        self.level
    }

    /// Moves on by `dt` seconds and returns the level at the end of it.
    pub fn process(&mut self, p: &EnvParams, dt: f32) -> f32 {
        self.t += dt;
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Delay => {
                self.level = 0.0;
                if self.t >= p.delay {
                    self.stage = Stage::Attack;
                    self.t = 0.0;
                }
            }
            Stage::Attack => {
                if self.t >= p.attack {
                    self.level = 1.0;
                    self.stage = Stage::Hold;
                    self.t = 0.0;
                } else {
                    self.level = curve(self.t / p.attack, p.atk_curve);
                }
            }
            Stage::Hold => {
                self.level = 1.0;
                if self.t >= p.hold {
                    self.stage = Stage::Decay;
                    self.t = 0.0;
                }
            }
            Stage::Decay => {
                if self.t >= p.decay {
                    self.level = p.sustain;
                    self.stage = Stage::Sustain;
                    self.t = 0.0;
                } else {
                    self.level = p.sustain
                        + (1.0 - p.sustain) * (1.0 - curve(self.t / p.decay, -p.dec_curve));
                }
            }
            Stage::Sustain => self.level = p.sustain,
            Stage::Release => {
                if self.t >= p.release {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                } else {
                    self.level =
                        self.release_from * (1.0 - curve(self.t / p.release, -p.rel_curve));
                }
            }
            Stage::Kill => {
                const KILL_TIME: f32 = 0.004;
                if self.t >= KILL_TIME {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                } else {
                    self.level = self.release_from * (1.0 - self.t / KILL_TIME);
                }
            }
        }
        self.level
    }
}

// ---------------------------------------------------------------------------- LFO

/// How an LFO's phase is found.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LfoMode {
    /// Its own phase, restarted by each note.
    Trigger,
    /// One phase shared by all notes.
    Free,
    /// Locked to the song's beat.
    Sync,
}

/// A triangle LFO from 0 to 1, with optional smoothing of its output.
#[derive(Clone, Copy, Default)]
pub struct Lfo {
    phase: f32,
    smoothed: f32,
}

impl Lfo {
    pub fn trigger(&mut self, start_phase: f32) {
        self.phase = start_phase.rem_euclid(1.0);
        self.smoothed = 0.0;
    }

    pub fn process(
        &mut self,
        freq: f32,
        mode: LfoMode,
        global_phase: f32,
        beat_phase: f32,
        smooth: f32,
        dt: f32,
    ) -> f32 {
        let phase = match mode {
            LfoMode::Free => global_phase,
            LfoMode::Sync => beat_phase,
            LfoMode::Trigger => {
                self.phase = (self.phase + freq * dt).rem_euclid(1.0);
                self.phase
            }
        };
        let raw = if phase < 0.5 {
            phase * 2.0
        } else {
            2.0 - phase * 2.0
        };
        if smooth <= 0.001 {
            self.smoothed = raw;
            return raw;
        }
        // Smoothing with a time constant of up to a quarter of a second.
        let k = 1.0 - (-dt / (smooth * 0.25)).exp();
        self.smoothed += (raw - self.smoothed) * k;
        self.smoothed
    }
}

// ---------------------------------------------------------------------------- filters

/// A state-variable filter (Zavalishin's TPT form), one channel.
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Low,
    High,
    Band,
    Notch,
}

impl Svf {
    fn process(&mut self, x: f32, g: f32, k: f32, mode: Mode) -> f32 {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match mode {
            Mode::Low => v2,
            Mode::High => x - k * v1 - v2,
            Mode::Band => v1 * k,
            Mode::Notch => x - k * v1,
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

const COMB_SIZE: usize = 4096;

/// The filter types, in the order the filter's Type setting lists them.
pub const FILTER_TYPES: &[&str] = &[
    "LP 12", "LP 24", "HP 12", "HP 24", "BP 12", "BP 24", "Notch", "Comb", "Formant",
];

/// One filter slot of a voice: a state-variable filter (12 or 24 dB low, high, band pass or
/// notch), a feedback comb, or a three-band vowel formant. Stereo.
#[derive(Clone)]
pub struct VoiceFilter {
    s1: [Svf; 2],
    s2: [Svf; 2],
    formant: [[Svf; 3]; 2],
    comb: [Vec<f32>; 2],
    comb_pos: usize,
    comb_lp: [f32; 2],
    sr: f32,
}

/// The vowels' first three formants, in Hz: A, E, I, O, U.
const VOWELS: [[f32; 3]; 5] = [
    [800.0, 1150.0, 2900.0],
    [400.0, 2000.0, 2800.0],
    [250.0, 2300.0, 3000.0],
    [400.0, 800.0, 2600.0],
    [350.0, 600.0, 2700.0],
];

impl VoiceFilter {
    pub fn new(sr: f32) -> Self {
        Self {
            s1: [Svf::default(); 2],
            s2: [Svf::default(); 2],
            formant: [[Svf::default(); 3]; 2],
            comb: [vec![0.0; COMB_SIZE], vec![0.0; COMB_SIZE]],
            comb_pos: 0,
            comb_lp: [0.0; 2],
            sr,
        }
    }

    pub fn reset(&mut self) {
        self.s1.iter_mut().for_each(Svf::reset);
        self.s2.iter_mut().for_each(Svf::reset);
        self.formant.iter_mut().flatten().for_each(Svf::reset);
        self.comb.iter_mut().for_each(|c| c.fill(0.0));
        self.comb_lp = [0.0; 2];
        self.comb_pos = 0;
    }

    /// Filters `l` and `r` in place. `kind` indexes [`FILTER_TYPES`].
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        kind: usize,
        cutoff: f32,
        res: f32,
        drive: f32,
        mix: f32,
    ) {
        let n = l.len().min(r.len());
        let drive_gain = 1.0 + drive * 9.0;
        let drive_comp = 1.0 / drive_gain.sqrt();
        let (wet, dry) = (mix, 1.0 - mix);
        let drive_in = |x: f32| {
            if drive > 0.001 {
                (x * drive_gain).tanh() * drive_comp
            } else {
                x
            }
        };

        if kind <= 6 {
            // 0 LP12, 1 LP24, 2 HP12, 3 HP24, 4 BP12, 5 BP24, 6 notch.
            let mode = match kind {
                0 | 1 => Mode::Low,
                2 | 3 => Mode::High,
                4 | 5 => Mode::Band,
                _ => Mode::Notch,
            };
            let two_pole = matches!(kind, 1 | 3 | 5);
            let fc = cutoff.clamp(10.0, self.sr * 0.49);
            let g = (PI * fc / self.sr).tan();
            let k = 2.0 - 1.98 * res.min(0.99);
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                for sample in buf.iter_mut().take(n) {
                    let x = drive_in(*sample);
                    let mut y = self.s1[ch].process(x, g, k, mode);
                    if two_pole {
                        y = self.s2[ch].process(y, g, k.max(1.0), mode);
                    }
                    *sample = dry * *sample + wet * y;
                }
            }
            return;
        }

        if kind == 7 {
            // Feedback comb: the cutoff sets the delay, the resonance the feedback.
            let delay = (self.sr / cutoff.max(20.0)).clamp(2.0, (COMB_SIZE - 2) as f32);
            let feedback = 0.5 + res * 0.48;
            for i in 0..n {
                let read = (self.comb_pos as f32 - delay).rem_euclid(COMB_SIZE as f32);
                let ri = read as usize;
                let rf = read - ri as f32;
                for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                    let line = &mut self.comb[ch];
                    let delayed = line[ri] + (line[(ri + 1) % COMB_SIZE] - line[ri]) * rf;
                    self.comb_lp[ch] += (delayed - self.comb_lp[ch]) * 0.6;
                    let x = drive_in(buf[i]);
                    let y = x + self.comb_lp[ch] * feedback;
                    line[self.comb_pos] = y;
                    buf[i] = dry * buf[i] + wet * y * 0.5;
                }
                self.comb_pos = (self.comb_pos + 1) % COMB_SIZE;
            }
            return;
        }

        // Formant: the cutoff morphs between vowels, the resonance narrows the bands.
        let position = ((cutoff / 20.0).max(1e-3).ln() / 1000f32.ln()).clamp(0.0, 1.0)
            * (VOWELS.len() - 1) as f32;
        let v0 = (position.floor() as usize).min(VOWELS.len() - 2);
        let vt = position - v0 as f32;
        let k = 2.0 - 1.9 * (0.3 + res * 0.7).min(0.99);
        for band in 0..3 {
            let f = VOWELS[v0][band] + (VOWELS[v0 + 1][band] - VOWELS[v0][band]) * vt;
            let g = (PI * f.min(self.sr * 0.45) / self.sr).tan();
            let gain = [1.0, 0.6, 0.35][band];
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                for sample in buf.iter_mut().take(n) {
                    let x = if band == 0 {
                        drive_in(*sample)
                    } else {
                        *sample
                    };
                    let y = self.formant[ch][band].process(x, g, k, Mode::Band) * gain * 2.5;
                    if band == 0 {
                        *sample = dry * *sample + wet * y;
                    } else {
                        *sample += wet * y;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> EnvParams {
        EnvParams {
            delay: 0.0,
            attack: 0.1,
            hold: 0.0,
            decay: 0.2,
            sustain: 0.5,
            release: 0.3,
            atk_curve: 0.0,
            dec_curve: 0.0,
            rel_curve: 0.0,
        }
    }

    #[test]
    fn the_envelope_goes_through_its_stages() {
        let mut env = Envelope::default();
        assert!(env.idle());
        env.trigger();
        let p = params();
        let level = |env: &mut Envelope, seconds: f32| {
            let mut v = 0.0;
            for _ in 0..(seconds / 0.01).round() as usize {
                v = env.process(&p, 0.01);
            }
            v
        };
        assert!(level(&mut env, 0.06) < 1.0, "attacking");
        assert_eq!(level(&mut env, 0.06), 1.0, "peaks (then decays)");
        let decayed = level(&mut env, 0.5);
        assert!(
            (decayed - 0.5).abs() < 1e-3,
            "settles on the sustain, got {decayed}"
        );
        env.gate_off();
        assert!(env.releasing());
        let mid = level(&mut env, 0.15);
        assert!(mid > 0.0 && mid < 0.5);
        level(&mut env, 0.3);
        assert!(env.idle() && env.value() == 0.0);

        // Killing fades in about 4 ms.
        env.trigger();
        level(&mut env, 0.5);
        env.kill();
        assert!(env.releasing());
        env.process(&p, 0.002);
        assert!(env.value() > 0.0 && env.value() < 0.5);
        env.process(&p, 0.01);
        assert!(env.idle());
    }

    #[test]
    fn curves_bend_the_attack() {
        let ramp = |c: f32| {
            let mut env = Envelope::default();
            env.trigger();
            let p = EnvParams {
                atk_curve: c,
                ..params()
            };
            env.process(&p, 0.0); // delay stage ends
            env.process(&p, 0.05)
        };
        assert!(ramp(0.8) < ramp(0.0) && ramp(0.0) < ramp(-0.8));
    }

    #[test]
    fn the_lfo_is_a_triangle_in_each_mode() {
        let mut lfo = Lfo::default();
        lfo.trigger(0.0);
        let at = |lfo: &mut Lfo, mode, g, b| lfo.process(1.0, mode, g, b, 0.0, 0.25);
        assert_eq!(
            at(&mut lfo, LfoMode::Trigger, 0.0, 0.0),
            0.5,
            "a quarter of the way up"
        );
        assert_eq!(at(&mut lfo, LfoMode::Trigger, 0.0, 0.0), 1.0);
        assert_eq!(
            at(&mut lfo, LfoMode::Free, 0.25, 0.9),
            0.5,
            "follows the shared phase"
        );
        assert_eq!(
            at(&mut lfo, LfoMode::Sync, 0.25, 0.75),
            0.5,
            "follows the beat"
        );
        // Smoothing lags behind.
        let mut slow = Lfo::default();
        let jumpy = slow.process(1.0, LfoMode::Free, 0.5, 0.0, 1.0, 0.01);
        assert!(jumpy > 0.0 && jumpy < 0.1, "got {jumpy}");
    }

    fn tone(freq: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (i as f32 / 48_000.0 * freq * std::f32::consts::TAU).sin())
            .collect()
    }

    fn level(buf: &[f32]) -> f32 {
        let tail = &buf[buf.len() / 2..];
        (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn filters_shape_the_sound_the_way_their_names_say() {
        let run = |kind: usize, cutoff: f32, freq: f32| {
            let mut filter = VoiceFilter::new(48_000.0);
            let mut l = tone(freq, 9_600);
            let mut r = l.clone();
            filter.process(&mut l, &mut r, kind, cutoff, 0.1, 0.0, 1.0);
            assert_eq!(l, r);
            assert!(l.iter().all(|x| x.is_finite()));
            level(&l)
        };
        let reference = level(&tone(1000.0, 9_600));
        assert!(
            run(0, 500.0, 5000.0) < 0.1 * reference,
            "low pass cuts highs"
        );
        assert!(run(0, 5000.0, 200.0) > 0.9 * reference, "and keeps lows");
        assert!(
            run(2, 2000.0, 100.0) < 0.1 * reference,
            "high pass cuts lows"
        );
        assert!(run(2, 500.0, 5000.0) > 0.9 * reference, "and keeps highs");
        assert!(
            run(1, 500.0, 2000.0) < run(0, 500.0, 2000.0),
            "24 dB cuts more than 12"
        );
        assert!(
            run(4, 1000.0, 1000.0) > run(4, 1000.0, 5000.0) * 2.0,
            "band pass"
        );
        assert!(
            run(6, 1000.0, 1000.0) < 0.2 * reference,
            "notch removes its frequency"
        );
        for kind in [7, 8] {
            for cutoff in [30.0, 400.0, 5000.0] {
                let mut filter = VoiceFilter::new(48_000.0);
                let mut l = tone(300.0, 9_600);
                let mut r = l.clone();
                filter.process(&mut l, &mut r, kind, cutoff, 0.9, 0.5, 1.0);
                assert!(
                    l.iter().all(|x| x.is_finite() && x.abs() < 50.0),
                    "kind {kind} at {cutoff}"
                );
            }
        }
    }

    #[test]
    fn the_mix_blends_dry_and_wet() {
        let mut filter = VoiceFilter::new(48_000.0);
        let source = tone(5000.0, 4_800);
        let (mut l, mut r) = (source.clone(), source.clone());
        filter.process(&mut l, &mut r, 0, 100.0, 0.0, 0.0, 0.0);
        assert_eq!(l, source, "no mix leaves the sound alone");
    }
}
