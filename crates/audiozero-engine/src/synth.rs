//! Built-in instruments.

/// Anything the engine can play notes on: built-in instruments now, hosted plugins later.
///
/// `render` runs on the audio thread, so implementations must not allocate, lock or block.
pub trait Instrument: Send {
    fn prepare(&mut self, sample_rate: f64);
    fn note_on(&mut self, pitch: u8, velocity: f32);
    fn note_off(&mut self, pitch: u8);
    fn all_notes_off(&mut self);
    /// Adds the next `left.len()` samples into `left` and `right` (both the same length).
    fn render(&mut self, left: &mut [f32], right: &mut [f32]);
}

/// Creates a built-in instrument by its project id.
pub fn create_builtin(id: &str) -> Option<Box<dyn Instrument>> {
    match id {
        "sine" => Some(Box::new(SineSynth::new())),
        _ => None,
    }
}

const VOICES: usize = 16;

#[derive(Clone, Copy, Default)]
struct Voice {
    active: bool,
    releasing: bool,
    pitch: u8,
    phase: f64,
    step: f64,
    level: f32,
    target: f32,
    age: u64,
}

/// A small polyphonic synth: sine plus a touch of second harmonic, with a short attack and an
/// exponential release. It exists so the engine can be tested and heard end to end.
pub struct SineSynth {
    voices: [Voice; VOICES],
    sample_rate: f64,
    attack_step: f32,
    release_coef: f32,
    clock: u64,
}

impl SineSynth {
    pub fn new() -> Self {
        let mut synth = Self {
            voices: [Voice::default(); VOICES],
            sample_rate: 44_100.0,
            attack_step: 0.0,
            release_coef: 0.0,
            clock: 0,
        };
        synth.prepare(44_100.0);
        synth
    }
}

impl Default for SineSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl Instrument for SineSynth {
    fn prepare(&mut self, sample_rate: f64) {
        self.sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            44_100.0
        };
        self.attack_step = (1.0 / (0.005 * self.sample_rate)) as f32; // 5 ms
        self.release_coef = (-6.9 / (0.25 * self.sample_rate)).exp() as f32; // ~250 ms to -60 dB
        self.all_notes_off();
    }

    fn note_on(&mut self, pitch: u8, velocity: f32) {
        let pitch = pitch.min(127);
        self.clock += 1;
        // Reuse a free voice, else steal the oldest.
        let index = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        let frequency = 440.0 * 2f64.powf((f64::from(pitch) - 69.0) / 12.0);
        self.voices[index] = Voice {
            active: true,
            releasing: false,
            pitch,
            phase: 0.0,
            step: frequency / self.sample_rate,
            level: 0.0,
            target: 0.25 * velocity.clamp(0.0, 1.0),
            age: self.clock,
        };
    }

    fn note_off(&mut self, pitch: u8) {
        for voice in &mut self.voices {
            if voice.active && !voice.releasing && voice.pitch == pitch {
                voice.releasing = true;
            }
        }
    }

    fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
        }
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let attack = self.attack_step;
        let release = self.release_coef;
        for voice in self.voices.iter_mut().filter(|v| v.active) {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                if voice.releasing {
                    voice.level *= release;
                    if voice.level < 1e-4 {
                        voice.active = false;
                        break;
                    }
                } else if voice.level < voice.target {
                    voice.level = (voice.level + attack * voice.target).min(voice.target);
                }
                let x = voice.phase * std::f64::consts::TAU;
                let sample = (x.sin() + 0.3 * (2.0 * x).sin()) as f32 * voice.level;
                voice.phase = (voice.phase + voice.step).fract();
                *l += sample;
                *r += sample;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, x| m.max(x.abs()))
    }

    #[test]
    fn plays_and_releases() {
        let mut synth = SineSynth::new();
        synth.prepare(48_000.0);
        let (mut l, mut r) = (vec![0.0; 4800], vec![0.0; 4800]);
        synth.note_on(69, 1.0);
        synth.render(&mut l, &mut r);
        assert!(peak(&l) > 0.1);
        synth.note_off(69);
        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        synth.render(&mut l, &mut r);
        assert!(peak(&l[47_000..]) == 0.0, "voice has finished releasing");
    }

    #[test]
    fn steals_voices_instead_of_growing() {
        let mut synth = SineSynth::new();
        for pitch in 0..100 {
            synth.note_on(pitch, 1.0);
        }
        assert_eq!(synth.voices.iter().filter(|v| v.active).count(), VOICES);
    }
}
