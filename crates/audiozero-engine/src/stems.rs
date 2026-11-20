//! Quick stem separation: splits a recording into **Drums**, **Bass**, **Vocals** and **Other**
//! with no model to download and nothing leaving the machine.
//!
//! How it works, frame by frame over a short-time Fourier transform:
//!
//! 1. *Percussive or sustained.* Median filtering the spectrogram along time keeps what lasts
//!    (notes, vocals, pads); along frequency it keeps what's sudden and broadband (kicks, snares,
//!    hats). That gives a soft mask for the drums (harmonic/percussive separation, Fitzgerald
//!    2010).
//! 2. *Bass.* What's sustained and below about 180 Hz.
//! 3. *Vocals.* What's sustained, above the bass and centre-panned: where the left and right
//!    channels agree in size and phase, which is where lead vocals are mixed.
//! 4. *Other.* Everything else.
//!
//! The four masks always add up to one, so the stems add back up to the original exactly.
//!
//! This is a signal-processing separation, not a trained model: it's quick and private, but it
//! can't tell a voice from a centre-panned guitar or keyboard, and a low, tonal kick drum looks
//! like a bass note to it, so some of it lands in Bass. (Measured on a synthesized song with
//! known tracks: the bass line went 92% to Bass, centred chords 75% to Vocals, and the kick split
//! about 44% Drums, 55% Bass.) Expect rough stems that are useful for remixing, muting a part or
//! sampling, not studio-clean ones.

use crate::sample::Sample;
use crate::spectral::{Spectrum, Stft, bin_hz};
use std::collections::VecDeque;

/// The stems, in the order they're returned.
pub const STEM_NAMES: [&str; 4] = ["Drums", "Bass", "Vocals", "Other"];

const SIZE: usize = 4096;
const HOP: usize = 1024;
/// Frames either side that the sustained-sound median looks at (a window of 17 frames, ~0.36 s).
const TIME_RADIUS: usize = 8;
/// Bins either side that the percussive median looks at.
const FREQ_RADIUS: usize = 8;
/// Where "bass" ends, in Hz, and how soft the edge is.
const BASS_EDGE_HZ: f32 = 180.0;
const BASS_SOFTNESS_HZ: f32 = 35.0;

/// One analysed frame: both channels' spectra and the magnitude of their mid signal.
struct Frame {
    left: Spectrum,
    right: Spectrum,
    mid: Vec<f32>,
}

/// The median of up to 17 values.
fn median(values: &mut [f32]) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    values[values.len() / 2]
}

/// Separates `sample` into [`STEM_NAMES`]. `progress(done)` gets 0 to 1 and returns `false` to
/// cancel (the result is then an error saying so).
pub fn separate(
    sample: &Sample,
    mut progress: impl FnMut(f32) -> bool,
) -> Result<Vec<(&'static str, Sample)>, String> {
    let len = sample.frames();
    if len < SIZE {
        return Err("that's too short to separate into stems".into());
    }
    let rate = sample.rate();
    let stft = Stft::new(SIZE, HOP);
    let (bins, frames) = (stft.bins(), stft.frames(len));
    let (left, right) = (sample.left(), sample.right());

    let bass_gain: Vec<f32> = (0..bins)
        .map(|k| {
            let hz = bin_hz(k, SIZE, rate) as f32;
            1.0 / (1.0 + ((hz - BASS_EDGE_HZ) / BASS_SOFTNESS_HZ).exp())
        })
        .collect();

    // Output: per stem, a left and a right signal.
    let mut out: Vec<[Vec<f32>; 2]> = (0..STEM_NAMES.len())
        .map(|_| [vec![0.0f32; len], vec![0.0f32; len]])
        .collect();

    let analyze = |t: usize| {
        let (l, r) = (stft.analyze(left, t), stft.analyze(right, t));
        let mid = (0..bins)
            .map(|k| 0.5 * ((l.re[k] + r.re[k]).hypot(l.im[k] + r.im[k])))
            .collect();
        Frame {
            left: l,
            right: r,
            mid,
        }
    };

    let mut window: VecDeque<(usize, Frame)> = VecDeque::new();
    let mut next = 0;
    let mut scratch = [0.0f32; 2 * TIME_RADIUS + 1];
    let mut sustained = vec![0.0f32; bins];
    let mut sudden = vec![0.0f32; bins];

    for t in 0..frames {
        if t % 64 == 0 && !progress(t as f32 / frames as f32) {
            return Err("cancelled".into());
        }
        // Keep the frames from t - TIME_RADIUS to t + TIME_RADIUS in the window.
        while next < frames && next <= t + TIME_RADIUS {
            window.push_back((next, analyze(next)));
            next += 1;
        }
        while window.front().is_some_and(|(i, _)| i + TIME_RADIUS < t) {
            window.pop_front();
        }
        let current = window
            .iter()
            .position(|(i, _)| *i == t)
            .expect("the current frame is in the window");

        // Sustained: the median of each bin over time.
        for (k, slot) in sustained.iter_mut().enumerate() {
            for (n, (_, f)) in window.iter().enumerate() {
                scratch[n] = f.mid[k];
            }
            *slot = median(&mut scratch[..window.len()]);
        }
        // Sudden: the median across nearby bins of this frame.
        let mid = &window[current].1.mid;
        for (k, slot) in sudden.iter_mut().enumerate() {
            let (lo, hi) = (
                k.saturating_sub(FREQ_RADIUS),
                (k + FREQ_RADIUS + 1).min(bins),
            );
            let mut values = [0.0f32; 2 * FREQ_RADIUS + 1];
            values[..hi - lo].copy_from_slice(&mid[lo..hi]);
            *slot = median(&mut values[..hi - lo]);
        }

        let frame = &window[current].1;
        let mut parts: Vec<[Spectrum; 2]> = (0..STEM_NAMES.len())
            .map(|_| {
                [
                    Spectrum {
                        re: vec![0.0; bins],
                        im: vec![0.0; bins],
                    },
                    Spectrum {
                        re: vec![0.0; bins],
                        im: vec![0.0; bins],
                    },
                ]
            })
            .collect();
        for k in 0..bins {
            let (h, p) = (sustained[k] * sustained[k], sudden[k] * sudden[k]);
            let drums = p / (h + p + 1e-12);
            let held = 1.0 - drums;
            // Centre: the channels agree in size and phase.
            let (lr, li) = (frame.left.re[k], frame.left.im[k]);
            let (rr, ri) = (frame.right.re[k], frame.right.im[k]);
            let differ = (lr - rr).powi(2) + (li - ri).powi(2);
            let total = lr * lr + li * li + rr * rr + ri * ri + 1e-12;
            let centre = (1.0 - differ / total).clamp(0.0, 1.0).powi(2);
            let bass = held * bass_gain[k];
            let vocals = held * (1.0 - bass_gain[k]) * centre;
            let other = held * (1.0 - bass_gain[k]) * (1.0 - centre);
            for (s, gain) in [drums, bass, vocals, other].into_iter().enumerate() {
                parts[s][0].re[k] = lr * gain;
                parts[s][0].im[k] = li * gain;
                parts[s][1].re[k] = rr * gain;
                parts[s][1].im[k] = ri * gain;
            }
        }
        for (s, [l, r]) in parts.iter().enumerate() {
            stft.synthesize_into(l, t, &mut out[s][0]);
            stft.synthesize_into(r, t, &mut out[s][1]);
        }
    }
    progress(1.0);

    Ok(STEM_NAMES
        .into_iter()
        .zip(out)
        .map(|(name, [mut l, mut r])| {
            stft.finish(&mut l);
            stft.finish(&mut r);
            (name, Sample::new(l, r, rate))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: f64 = 44_100.0;

    /// Four seconds of: a kick (a 60 Hz burst) on every beat, a steady 90 Hz bass note, a
    /// centre-panned 1 kHz tone (the "vocal"), and a 2.2 kHz tone panned hard left.
    fn mix() -> Sample {
        let n = (4.0 * RATE) as usize;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let since_beat = t % 0.5;
            let kick = (TAU * 60.0 * since_beat).sin() * (-since_beat * 25.0).exp() * 0.8;
            let bass = (TAU * 90.0 * t).sin() * 0.3;
            let voice = (TAU * 1000.0 * t).sin() * 0.3;
            let guitar = (TAU * 2200.0 * t).sin() * 0.3;
            l[i] = kick + bass + voice + guitar;
            r[i] = kick + bass + voice;
        }
        Sample::new(l, r, RATE)
    }

    fn energy(s: &Sample) -> f64 {
        s.left()
            .iter()
            .chain(s.right())
            .map(|x| f64::from(*x).powi(2))
            .sum()
    }

    /// The energy of `s` near `hz`, by correlating with a tone there (both channels).
    fn at(s: &Sample, hz: f32) -> f64 {
        let (mut c, mut sn) = (0.0f64, 0.0f64);
        for (i, (a, b)) in s.left().iter().zip(s.right()).enumerate() {
            let ph = TAU * hz * i as f32 / RATE as f32;
            let x = f64::from(a + b);
            c += x * f64::from(ph.cos());
            sn += x * f64::from(ph.sin());
        }
        (c * c + sn * sn).sqrt()
    }

    #[test]
    fn the_stems_add_back_up_to_the_original() {
        let song = mix();
        let stems = separate(&song, |_| true).unwrap();
        assert_eq!(stems.len(), 4);
        let (mut error, mut total) = (0.0f64, 0.0f64);
        for i in 0..song.frames() {
            let sum: f32 = stems.iter().map(|(_, s)| s.left()[i]).sum();
            error += f64::from(sum - song.left()[i]).powi(2);
            total += f64::from(song.left()[i]).powi(2);
        }
        let snr = 10.0 * (total / error.max(1e-20)).log10();
        assert!(
            snr > 60.0,
            "the stems sum to the song to within {snr:.1} dB"
        );
    }

    #[test]
    fn each_part_lands_in_its_own_stem() {
        let stems = separate(&mix(), |_| true).unwrap();
        let by = |name: &str| &stems.iter().find(|(n, _)| *n == name).unwrap().1;
        let (drums, bass, vocals, other) = (by("Drums"), by("Bass"), by("Vocals"), by("Other"));
        // Each tone is mostly in the stem it belongs to.
        assert!(
            at(bass, 90.0) > 3.0 * at(drums, 90.0),
            "the steady low note is bass"
        );
        assert!(
            at(vocals, 1000.0) > 3.0 * at(other, 1000.0),
            "the centred tone is vocals"
        );
        assert!(
            at(other, 2200.0) > 3.0 * at(vocals, 2200.0),
            "the left-only tone is other"
        );
        // The kicks make the drums stem the one that has the 60 Hz bursts.
        assert!(at(drums, 60.0) > at(vocals, 60.0) && at(drums, 60.0) > at(other, 60.0));
        assert!(energy(drums) > 0.0);
    }

    #[test]
    fn it_can_be_cancelled_and_refuses_tiny_input() {
        let song = mix();
        assert_eq!(
            separate(&song, |_| false).err().as_deref(),
            Some("cancelled")
        );
        let tiny = Sample::new(vec![0.0; 100], vec![0.0; 100], RATE);
        assert!(separate(&tiny, |_| true).is_err());
    }
}
