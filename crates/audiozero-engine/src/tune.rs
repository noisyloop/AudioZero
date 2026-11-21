//! Vocal tuning for recorded audio: finds the pitch of a sung line, pulls it onto the notes of a
//! scale, and re-pitches the audio to match. It works offline on a whole clip, so there's no
//! latency to hide and the detector can look at the notes around each moment.
//!
//! Three steps:
//! 1. [`analyze`] tracks the pitch (YIN, on a copy of the audio at about 16 kHz), frame by frame.
//!    Breaths, consonants and silence come out as unvoiced and are never changed.
//! 2. [`correction`] decides, for each frame, how many semitones to move: towards the nearest
//!    note of the scale, as fast as [`Settings::speed_ms`] allows. 0 ms is the hard-tuned
//!    effect; 50 to 150 ms corrects drift and leaves vibrato alone.
//! 3. [`shift`] re-pitches with PSOLA: one grain per pitch period, laid down closer together or
//!    further apart. The grains themselves aren't resampled, so the voice's formants (its
//!    character) stay where they were. Where nothing is to be changed the output is the input.
//!
//! It expects one voice (or one monophonic instrument) per clip. Chords and busy mixes have no
//! single pitch to find.

use crate::music::{Scale, in_scale};
use crate::sample::Sample;

/// The lowest and highest pitch looked for, in Hz (about C2 to G5).
pub const MIN_HZ: f32 = 65.0;
pub const MAX_HZ: f32 = 800.0;
/// Time between pitch frames, in seconds (about).
const HOP_SECONDS: f64 = 0.005;
/// The rate the detector works at.
const DETECT_RATE: f64 = 16_000.0;
/// YIN's threshold: lower finds fewer, surer pitches.
const YIN_THRESHOLD: f32 = 0.15;
/// The most a frame is ever moved, in semitones.
const MAX_SHIFT: f32 = 12.0;
/// How much nearer another note has to be before the target changes, in semitones.
const HYSTERESIS: f32 = 0.25;
/// The grain spacing where there's no pitch, in seconds.
const UNVOICED_PERIOD: f64 = 0.005;

/// How to tune.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// The key's root, 0 = C to 11 = B.
    pub key: u8,
    pub scale: Scale,
    /// How long a correction takes to arrive, in milliseconds. 0 snaps at once.
    pub speed_ms: f32,
    /// How much of the correction to apply, 0 (none) to 1 (all).
    pub amount: f32,
}

/// The pitch of a recording over time.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchTrack {
    /// Frames of audio between pitch frames.
    pub hop: usize,
    /// The frame (in the audio) at the middle of pitch frame 0.
    pub first: usize,
    /// The pitch in Hz, or 0 where there's none.
    pub hz: Vec<f32>,
}

impl PitchTrack {
    /// The pitch frame nearest audio frame `at`.
    fn frame_at(&self, at: f64) -> Option<usize> {
        if self.hz.is_empty() {
            return None;
        }
        let i = ((at - self.first as f64) / self.hop as f64).round();
        Some((i.max(0.0) as usize).min(self.hz.len() - 1))
    }

    /// The share of frames with a pitch, 0 to 1.
    pub fn voiced_share(&self) -> f32 {
        if self.hz.is_empty() {
            return 0.0;
        }
        self.hz.iter().filter(|&&f| f > 0.0).count() as f32 / self.hz.len() as f32
    }
}

/// A frequency as a (fractional) MIDI note number.
pub fn hz_to_note(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Tracks the pitch of `mono` (audio at `rate` Hz).
pub fn analyze(mono: &[f32], rate: f64) -> PitchTrack {
    analyze_with(mono, rate, |_| true).expect("nothing asks it to stop")
}

/// [`analyze`], reporting progress (0 to 1). `None` if `progress` returned false.
pub fn analyze_with(
    mono: &[f32],
    rate: f64,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<PitchTrack> {
    // A cheaper copy: averaged down to about 16 kHz, which is plenty for the fundamental.
    let factor = ((rate / DETECT_RATE).round() as usize).max(1);
    let low: Vec<f32> = mono
        .chunks_exact(factor)
        .map(|c| c.iter().sum::<f32>() / factor as f32)
        .collect();
    let low_rate = rate / factor as f64;
    let tau_max = (low_rate / f64::from(MIN_HZ)).ceil() as usize;
    let tau_min = ((low_rate / f64::from(MAX_HZ)).floor() as usize).max(2);
    let window = tau_max * 2;
    let hop = ((low_rate * HOP_SECONDS).round() as usize).max(1);
    let mut track = PitchTrack {
        hop: hop * factor,
        first: (window / 2) * factor,
        hz: Vec::new(),
    };
    if low.len() < window + tau_max + 1 {
        return Some(track);
    }
    let frames = (low.len() - window - tau_max) / hop + 1;
    let mut level = Vec::with_capacity(frames);
    let mut diff = vec![0.0f32; tau_max + 1];
    for f in 0..frames {
        if f.is_multiple_of(32) && !progress(f as f32 / frames as f32) {
            return None;
        }
        let x = &low[f * hop..f * hop + window + tau_max];
        level.push((x[..window].iter().map(|v| v * v).sum::<f32>() / window as f32).sqrt());
        // YIN: the difference function, then its cumulative mean normalised form.
        let mut running = 0.0;
        diff[0] = 1.0;
        for tau in 1..=tau_max {
            let d: f32 = x[..window]
                .iter()
                .zip(&x[tau..tau + window])
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            running += d;
            diff[tau] = if running > 0.0 {
                d * tau as f32 / running
            } else {
                1.0
            };
        }
        // The first dip under the threshold, followed down to its bottom.
        let mut tau = tau_min;
        let mut found = None;
        while tau < tau_max {
            if diff[tau] < YIN_THRESHOLD {
                while tau + 1 < tau_max && diff[tau + 1] < diff[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        track.hz.push(found.map_or(0.0, |tau| {
            // A parabola through the dip and its neighbours gives the period between samples.
            let (a, b, c) = (diff[tau - 1], diff[tau], diff[tau + 1]);
            let bend = a - 2.0 * b + c;
            let fine = if bend > 0.0 {
                (0.5 * (a - c) / bend).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            (low_rate / (tau as f64 + f64::from(fine))) as f32
        }));
    }
    // Quiet frames have no pitch worth trusting.
    let loudest = level.iter().fold(0.0f32, |m, &l| m.max(l));
    let gate = (loudest * 0.02).max(1e-4);
    for (hz, &l) in track.hz.iter_mut().zip(&level) {
        if l < gate || *hz > MAX_HZ * 1.05 || *hz < MIN_HZ * 0.95 {
            *hz = 0.0;
        }
    }
    smooth(&mut track.hz);
    Some(track)
}

/// Removes single-frame jumps (octave slips) and pitched runs too short to be a note.
fn smooth(hz: &mut [f32]) {
    let raw = hz.to_vec();
    for i in 0..raw.len() {
        if raw[i] <= 0.0 {
            continue;
        }
        let mut near: Vec<f32> = raw[i.saturating_sub(2)..(i + 3).min(raw.len())]
            .iter()
            .copied()
            .filter(|&f| f > 0.0)
            .collect();
        near.sort_by(f32::total_cmp);
        hz[i] = near[near.len() / 2];
    }
    const SHORTEST: usize = 4; // frames: about 20 ms
    let mut start = 0;
    while start < hz.len() {
        if hz[start] <= 0.0 {
            start += 1;
            continue;
        }
        let end = (start..hz.len())
            .find(|&i| hz[i] <= 0.0)
            .unwrap_or(hz.len());
        if end - start < SHORTEST {
            hz[start..end].fill(0.0);
        }
        start = end;
    }
}

/// The note of the scale nearest `note` (a fractional MIDI note).
fn nearest_in_scale(note: f32, key: u8, scale: Scale) -> f32 {
    let base = note.round() as i32;
    (-6..=6)
        .map(|d| base + d)
        .filter(|&p| (0..=127).contains(&p) && in_scale(p as u8, key, scale))
        .map(|p| p as f32)
        .min_by(|a, b| (a - note).abs().total_cmp(&(b - note).abs()))
        .unwrap_or(note)
}

/// How far to move each pitch frame, in semitones (0 where there's no pitch).
pub fn correction(track: &PitchTrack, rate: f64, settings: Settings) -> Vec<f32> {
    let amount = settings.amount.clamp(0.0, 1.0);
    let dt = track.hop as f64 / rate;
    let glide = if settings.speed_ms > 0.0 {
        (1.0 - (-dt * 1000.0 / f64::from(settings.speed_ms)).exp()) as f32
    } else {
        1.0
    };
    // A gap this long between notes starts a new phrase: the target and the glide start afresh.
    let phrase_gap = (0.05 / dt).ceil() as usize;
    let mut out = vec![0.0; track.hz.len()];
    let mut target: Option<f32> = None;
    let mut offset: Option<f32> = None;
    let mut silent = usize::MAX / 2;
    for (i, &hz) in track.hz.iter().enumerate() {
        if hz <= 0.0 {
            silent += 1;
            if silent >= phrase_gap {
                (target, offset) = (None, None);
            }
            continue;
        }
        silent = 0;
        let note = hz_to_note(hz);
        let nearest = nearest_in_scale(note, settings.key, settings.scale);
        let held = match target {
            // Stay on the note being sung until another is clearly nearer, so vibrato around
            // the halfway point doesn't flip between two notes.
            Some(t) if (note - nearest).abs() + HYSTERESIS >= (note - t).abs() => t,
            _ => nearest,
        };
        target = Some(held);
        let wanted = ((held - note) * amount).clamp(-MAX_SHIFT, MAX_SHIFT);
        let now = offset.map_or(wanted, |o| o + (wanted - o) * glide);
        offset = Some(now);
        out[i] = now;
    }
    out
}

/// The pitch after correction, in Hz (0 where there's none), for drawing.
pub fn corrected_hz(track: &PitchTrack, semitones: &[f32]) -> Vec<f32> {
    track
        .hz
        .iter()
        .zip(semitones)
        .map(|(&hz, &s)| {
            if hz > 0.0 {
                hz * (s / 12.0).exp2()
            } else {
                0.0
            }
        })
        .collect()
}

/// Where each pitch period starts: a mark on the same peak of every cycle where there's a
/// pitch, and evenly spaced marks where there isn't.
fn pitch_marks(mono: &[f32], rate: f64, track: &PitchTrack) -> Vec<usize> {
    let n = mono.len();
    let period_at = |at: f64| -> Option<f64> {
        let hz = track.hz[track.frame_at(at)?];
        (hz > 0.0).then(|| rate / f64::from(hz))
    };
    let idle = (rate * UNVOICED_PERIOD).max(8.0);
    let mut marks = vec![0usize];
    let mut voiced_before = false;
    loop {
        let last = *marks.last().expect("there is a first mark") as f64;
        let guess = last + period_at(last).unwrap_or(idle);
        let next = match period_at(guess) {
            Some(period) => {
                // Look for the cycle's peak: close by when following a pitch, anywhere in the
                // cycle when the pitch has just begun.
                let centre = if voiced_before { last + period } else { guess };
                let reach = period * if voiced_before { 0.2 } else { 0.5 };
                let low = ((centre - reach).max(last + period * 0.4).ceil() as usize).min(n);
                let high = ((centre + reach).floor() as usize + 1).min(n);
                voiced_before = true;
                (low..high)
                    .max_by(|&a, &b| mono[a].total_cmp(&mono[b]))
                    .unwrap_or(guess.round() as usize)
            }
            None => {
                voiced_before = false;
                guess.round() as usize
            }
        };
        let next = next.max(last as usize + 4);
        if next >= n {
            return marks;
        }
        marks.push(next);
    }
}

/// Re-pitches `sample` by `semitones` (one value per frame of `track`), keeping its length.
/// `progress` is called with 0 to 1 as it goes; returning false stops early (with `None`).
pub fn shift(
    sample: &Sample,
    track: &PitchTrack,
    semitones: &[f32],
    mut progress: impl FnMut(f32) -> bool,
) -> Option<Sample> {
    let n = sample.frames();
    let rate = sample.rate();
    if n == 0 || track.hz.is_empty() {
        return Some(sample.clone());
    }
    let mono: Vec<f32> = sample
        .left()
        .iter()
        .zip(sample.right())
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    let marks = pitch_marks(&mono, rate, track);
    let ratio_at = |at: usize| -> f64 {
        track
            .frame_at(at as f64)
            .filter(|&f| track.hz[f] > 0.0)
            .map_or(1.0, |f| f64::from(semitones[f] / 12.0).exp2())
    };
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut weight = vec![0.0f32; n];
    let (src_l, src_r) = (sample.left(), sample.right());
    let mut j = 0;
    let mut t = 0.0f64;
    let mut steps = 0usize;
    while t < n as f64 {
        while j + 1 < marks.len() && (marks[j + 1] as f64 - t).abs() <= (marks[j] as f64 - t).abs()
        {
            j += 1;
        }
        let centre = marks[j];
        let after = marks.get(j + 1).map_or(n, |&m| m) - centre;
        let before = if j > 0 { centre - marks[j - 1] } else { after };
        let (before, after) = (before.max(1), after.max(1));
        let ratio = ratio_at(centre);
        // Where nothing changes, grains go back exactly where they came from.
        if (ratio - 1.0).abs() < 1e-6 {
            t = centre as f64;
        }
        let to = t.round() as isize;
        for k in -(before as isize)..after as isize {
            let (from, into) = (centre as isize + k, to + k);
            if from < 0 || from >= n as isize || into < 0 || into >= n as isize {
                continue;
            }
            let span = if k < 0 { before } else { after } as f32;
            let w = 0.5 * (1.0 + (std::f32::consts::PI * k as f32 / span).cos());
            left[into as usize] += src_l[from as usize] * w;
            right[into as usize] += src_r[from as usize] * w;
            weight[into as usize] += w;
        }
        t += (after as f64 / ratio).max(2.0);
        steps += 1;
        if steps.is_multiple_of(256) && !progress((t / n as f64).min(1.0) as f32) {
            return None;
        }
    }
    for ((l, r), &w) in left.iter_mut().zip(right.iter_mut()).zip(&weight) {
        if w > 1e-6 {
            *l /= w;
            *r /= w;
        }
    }
    // The few frames no grain reached (the very end) stay as recorded.
    for (i, &w) in weight.iter().enumerate() {
        if w <= 1e-6 {
            (left[i], right[i]) = (src_l[i], src_r[i]);
        }
    }
    Some(Sample::new(left, right, rate))
}

/// The whole job: analyse, decide, re-pitch. `None` if `progress` asked to stop.
pub fn tune(
    sample: &Sample,
    settings: Settings,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<Sample> {
    let mono: Vec<f32> = sample
        .left()
        .iter()
        .zip(sample.right())
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    // Finding the pitch is most of the work.
    let track = analyze_with(&mono, sample.rate(), |p| progress(p * 0.8))?;
    let semitones = correction(&track, sample.rate(), settings);
    shift(sample, &track, &semitones, |p| progress(0.8 + p * 0.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{Rng, scale_by_id};
    use std::f32::consts::TAU;

    const RATE: f64 = 48_000.0;

    fn note_hz(note: f32) -> f32 {
        440.0 * ((note - 69.0) / 12.0).exp2()
    }

    /// A voice-like tone: a harmonic series shaped by two formants, at the pitch `note_at`
    /// gives (a fractional MIDI note) for each moment in seconds.
    fn voice(seconds: f32, note_at: impl Fn(f32) -> f32) -> Vec<f32> {
        let frames = (seconds * RATE as f32) as usize;
        let mut phase = 0.0f32;
        (0..frames)
            .map(|i| {
                let time = i as f32 / RATE as f32;
                let hz = note_hz(note_at(time));
                phase = (phase + hz / RATE as f32).fract();
                let mut x = 0.0;
                for h in 1..=24 {
                    let f = hz * h as f32;
                    if f > 9000.0 {
                        break;
                    }
                    // Formants near 700 and 1200 Hz, like "ah".
                    let gain = 1.0 / h as f32
                        + 0.6 * (-((f - 700.0) / 150.0).powi(2)).exp()
                        + 0.4 * (-((f - 1200.0) / 200.0).powi(2)).exp();
                    x += gain * (TAU * phase * h as f32).sin();
                }
                let fade = (time * 50.0).min(1.0) * ((seconds - time) * 50.0).min(1.0);
                0.2 * x * fade
            })
            .collect()
    }

    fn noise(seconds: f32, level: f32) -> Vec<f32> {
        let mut rng = Rng::new(7);
        (0..(seconds * RATE as f32) as usize)
            .map(|_| (rng.unit() * 2.0 - 1.0) * level)
            .collect()
    }

    fn stereo(mono: &[f32]) -> Sample {
        Sample::new(mono.to_vec(), mono.to_vec(), RATE)
    }

    /// The pitched frames of `mono` as MIDI notes, skipping the first and last 60 ms.
    fn notes_of(mono: &[f32]) -> Vec<f32> {
        let track = analyze(mono, RATE);
        let edge = 12;
        let end = track.hz.len().saturating_sub(edge);
        track.hz[edge.min(end)..end]
            .iter()
            .filter(|&&f| f > 0.0)
            .map(|&f| hz_to_note(f))
            .collect()
    }

    fn mean(v: &[f32]) -> f32 {
        v.iter().sum::<f32>() / v.len() as f32
    }

    fn spread(v: &[f32]) -> f32 {
        let m = mean(v);
        (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn settings(scale: &str, speed_ms: f32) -> Settings {
        Settings {
            key: 0,
            scale: scale_by_id(scale),
            speed_ms,
            amount: 1.0,
        }
    }

    #[test]
    fn the_detector_finds_sung_pitches() {
        for note in [38.0, 45.3, 52.0, 57.5, 64.0, 69.0, 76.2] {
            let notes = notes_of(&voice(0.6, |_| note));
            assert!(notes.len() > 60, "{note} is heard as pitched");
            let error = (mean(&notes) - note).abs();
            assert!(
                error < 0.08,
                "{note} is found within 8 cents (off by {error})"
            );
            assert!(spread(&notes) < 0.05, "{note} is steady");
        }
    }

    #[test]
    fn silence_and_noise_have_no_pitch() {
        assert_eq!(analyze(&vec![0.0; 24_000], RATE).voiced_share(), 0.0);
        let hiss = analyze(&noise(0.5, 0.3), RATE);
        assert!(hiss.voiced_share() < 0.05, "{}", hiss.voiced_share());
        assert!(analyze(&[0.1; 100], RATE).hz.is_empty(), "too short to say");
    }

    #[test]
    fn a_flat_note_is_pulled_onto_the_scale() {
        // 35 cents under A3, in C major: hard tune puts it on A3.
        let sung = voice(1.0, |_| 56.65);
        let tuned = tune(&stereo(&sung), settings("major", 0.0), |_| true).unwrap();
        assert_eq!(tuned.frames(), sung.len(), "the length doesn't change");
        let notes = notes_of(tuned.left());
        assert!((mean(&notes) - 57.0).abs() < 0.05, "{}", mean(&notes));
        let level = rms(tuned.left()) / rms(&sung);
        assert!((0.8..1.25).contains(&level), "about as loud ({level})");
    }

    #[test]
    fn a_note_outside_the_scale_moves_to_the_nearest_one_in_it() {
        // C#4 isn't in C major; a slightly sharp one goes up to D4.
        let sung = voice(0.8, |_| 61.2);
        let tuned = tune(&stereo(&sung), settings("major", 0.0), |_| true).unwrap();
        assert!((mean(&notes_of(tuned.left())) - 62.0).abs() < 0.06);
        // In the chromatic scale it's a C#.
        let tuned = tune(&stereo(&sung), settings("chromatic", 0.0), |_| true).unwrap();
        assert!((mean(&notes_of(tuned.left())) - 61.0).abs() < 0.06);
    }

    #[test]
    fn hard_tune_flattens_vibrato_and_natural_keeps_it() {
        // A4, 20 cents sharp, with a 5.5 Hz vibrato of 40 cents each way.
        let sung = voice(1.5, |t| 69.2 + 0.4 * (TAU * 5.5 * t).sin());
        let before = notes_of(&sung);
        assert!(spread(&before) > 0.2);

        let hard = tune(&stereo(&sung), settings("major", 0.0), |_| true).unwrap();
        let hard = notes_of(hard.left());
        assert!(
            (mean(&hard) - 69.0).abs() < 0.05,
            "on the note: {}",
            mean(&hard)
        );
        assert!(spread(&hard) < 0.06, "no vibrato left: {}", spread(&hard));

        let natural = tune(&stereo(&sung), settings("major", 150.0), |_| true).unwrap();
        let natural = notes_of(natural.left());
        // The glide starts on the note, so skip nothing: the average is the note.
        assert!(
            (mean(&natural) - 69.0).abs() < 0.08,
            "centred: {}",
            mean(&natural)
        );
        assert!(
            spread(&natural) > 0.15,
            "vibrato kept: {}",
            spread(&natural)
        );
    }

    #[test]
    fn a_melody_keeps_its_notes_and_timing() {
        // Three notes, each a bit off: C4 +30, E4 -25, G4 +20 cents.
        let sung = voice(1.5, |t| match t {
            t if t < 0.5 => 60.3,
            t if t < 1.0 => 63.75,
            _ => 67.2,
        });
        let tuned = tune(&stereo(&sung), settings("major", 0.0), |_| true).unwrap();
        let track = analyze(tuned.left(), RATE);
        let at = |seconds: f64| {
            let frame = track.frame_at(seconds * RATE).unwrap();
            hz_to_note(track.hz[frame])
        };
        for (seconds, note) in [(0.25, 60.0), (0.75, 64.0), (1.25, 67.0)] {
            assert!(
                (at(seconds) - note).abs() < 0.06,
                "{} at {seconds}s",
                at(seconds)
            );
        }
    }

    #[test]
    fn amount_zero_and_unpitched_audio_are_left_alone() {
        let sung = voice(0.5, |_| 56.65);
        let none = Settings {
            amount: 0.0,
            ..settings("major", 0.0)
        };
        let same = tune(&stereo(&sung), none, |_| true).unwrap();
        let worst = same
            .left()
            .iter()
            .zip(&sung)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-4, "amount 0 changes nothing ({worst})");

        let hiss = noise(0.5, 0.2);
        let out = tune(&stereo(&hiss), settings("major", 0.0), |_| true).unwrap();
        let changed = out
            .left()
            .iter()
            .zip(&hiss)
            .filter(|(a, b)| (**a - **b).abs() > 1e-4)
            .count();
        assert!(
            changed < hiss.len() / 50,
            "noise passes through ({changed})"
        );
    }

    #[test]
    fn breaths_between_notes_are_untouched_and_the_output_is_clean() {
        // A flat note, a breath of noise, another flat note.
        let mut sung = voice(0.5, |_| 59.7);
        let breath_at = sung.len();
        sung.extend(noise(0.2, 0.05));
        let breath_end = sung.len();
        sung.extend(voice(0.5, |_| 64.3));
        let tuned = tune(&stereo(&sung), settings("major", 0.0), |_| true).unwrap();
        assert!(tuned.left().iter().all(|x| x.is_finite() && x.abs() < 2.0));
        let middle = breath_at + 2400..breath_end - 2400;
        let worst = tuned.left()[middle.clone()]
            .iter()
            .zip(&sung[middle])
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-4, "the breath is as recorded ({worst})");
        // No clicks: nothing jumps further between samples than the input ever does (much).
        let jump = |v: &[f32]| v.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(jump(tuned.left()) < jump(&sung) * 2.0 + 0.05);
    }

    #[test]
    fn the_voice_keeps_its_character() {
        // Shifting up 3 semitones must not move the formants: energy near 700 Hz stays the
        // strongest band, as PSOLA promises (resampling would move it to 830 Hz).
        let sung = voice(0.8, |_| 50.0);
        let track = analyze(&sung, RATE);
        let up = vec![3.0; track.hz.len()];
        let shifted = shift(&stereo(&sung), &track, &up, |_| true).unwrap();
        assert!((mean(&notes_of(shifted.left())) - 53.0).abs() < 0.08);
        let band = |v: &[f32], hz: f32| {
            // The level of one frequency, by correlating with a sine and a cosine.
            let (mut s, mut c) = (0.0f32, 0.0f32);
            for (i, x) in v.iter().enumerate().skip(4800).take(24_000) {
                let p = TAU * hz * i as f32 / RATE as f32;
                s += x * p.sin();
                c += x * p.cos();
            }
            s.hypot(c)
        };
        let around = |v: &[f32], centre: f32| {
            (-6..=6)
                .map(|d| band(v, centre + d as f32 * 15.0))
                .sum::<f32>()
        };
        let (low, high) = (around(shifted.left(), 700.0), around(shifted.left(), 850.0));
        assert!(
            low > high * 1.3,
            "the 700 Hz formant stays put ({low} vs {high})"
        );
    }

    #[test]
    fn stopping_early_gives_nothing() {
        let sung = voice(2.0, |_| 56.65);
        assert!(tune(&stereo(&sung), settings("major", 0.0), |_| false).is_none());
    }
}
