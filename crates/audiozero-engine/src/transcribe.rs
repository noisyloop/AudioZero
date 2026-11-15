//! Audio to MIDI: turns a recording into notes, in three ways.
//!
//! - **Melody:** follows the strongest pitch over time (harmonic summation on a short-time
//!   spectrum, then smoothing) and writes it as single notes. Works best on one voice or
//!   instrument; on a full mix it follows whatever line is loudest.
//! - **Harmony:** works out the chord on each beat (a chroma profile matched against major,
//!   minor, seventh and so on) and writes the chords as held notes.
//! - **Drums:** finds the hits (spectral flux), tells kick, snare and hats apart by where their
//!   energy sits, and writes them on the General MIDI drum notes the drum rack uses (kick 36,
//!   snare 38, closed hat 42, open hat 46).
//!
//! A hi-hat that lands at the same instant as a snare is hidden by its noise, so it isn't
//! reported on its own.
//!
//! All three snap notes to a sixteenth-note grid so they're easy to edit. It's signal
//! processing, not a trained model, so treat the result as a starting point to tidy in the piano
//! roll, not a perfect transcription.

use crate::project::Note;
use crate::sample::Sample;
use crate::spectral::Stft;

/// What to pull out of the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Melody,
    Harmony,
    Drums,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Melody, Kind::Harmony, Kind::Drums];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Melody => "Melody",
            Kind::Harmony => "Harmony (chords)",
            Kind::Drums => "Drums",
        }
    }
}

/// General MIDI drum notes.
pub const KICK: u8 = 36;
pub const SNARE: u8 = 38;
pub const CLOSED_HAT: u8 = 42;
pub const OPEN_HAT: u8 = 46;

/// Notes snap to this many beats (a sixteenth).
const GRID: f64 = 0.25;

/// The part of `sample` from `offset` seconds, `seconds` long, as one mono signal.
fn mono(sample: &Sample, offset: f64, seconds: f64) -> (Vec<f32>, f64) {
    let part = sample.region(offset, seconds);
    let signal = part
        .left()
        .iter()
        .zip(part.right())
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    (signal, part.rate())
}

fn snap(beats: f64) -> f64 {
    (beats / GRID).round() * GRID
}

/// Writes the notes for `kind` found in the part of `sample` from `offset` seconds, `seconds`
/// long, at `bpm`. Note times are in beats from the start of that part. `progress` gets 0 to 1 and
/// returns `false` to cancel.
pub fn transcribe(
    sample: &Sample,
    offset: f64,
    seconds: f64,
    bpm: f64,
    kind: Kind,
    mut progress: impl FnMut(f32) -> bool,
) -> Result<Vec<Note>, String> {
    if !(20.0..=400.0).contains(&bpm) {
        return Err("the tempo has to be 20 to 400 BPM".into());
    }
    let (signal, rate) = mono(sample, offset, seconds);
    if signal.len() < 4096 {
        return Err("that's too short to find notes in".into());
    }
    let notes = match kind {
        Kind::Melody => melody(&signal, rate, bpm, &mut progress)?,
        Kind::Harmony => harmony(&signal, rate, bpm, &mut progress)?,
        Kind::Drums => drums(&signal, rate, bpm, &mut progress)?,
    };
    let length = seconds * bpm / 60.0;
    Ok(notes.into_iter().filter(|n| n.start < length).collect())
}

/// Time of the centre of frame `t` of a [`Stft`], in seconds.
fn frame_time(stft: &Stft, t: usize, rate: f64) -> f64 {
    ((t * stft.hop + stft.hop) as f64 - stft.size as f64 / 2.0) / rate
}

fn magnitudes(stft: &Stft, signal: &[f32], t: usize) -> Vec<f32> {
    let s = stft.analyze(signal, t);
    (0..stft.bins()).map(|k| s.re[k].hypot(s.im[k])).collect()
}

// ---- melody -----------------------------------------------------------------------------------

const MELODY_LOW: u8 = 28; // E1, the bottom string of a bass
const MELODY_HIGH: u8 = 90; // F#6
/// Notes below this are tracked with a longer transform, which tells low semitones apart.
const LOW_BAND_TOP: u8 = 57; // A3
const HARMONICS: usize = 8;

fn hz(pitch: f64) -> f64 {
    440.0 * 2f64.powf((pitch - 69.0) / 12.0)
}

/// Per frame: the best pitch and how strongly it stands out, and how much sound there is.
type PitchTrack = (Vec<(u8, f32)>, Vec<f32>);

/// Per frame of an analysis with transform size `size` at a hop of 512: the best pitch from `low`
/// to `high` and how strongly it stands out from the field of candidates, and how much sound
/// there is.
fn track_pitch(
    signal: &[f32],
    rate: f64,
    size: usize,
    (low, high): (u8, u8),
    progress: &mut impl FnMut(f32) -> bool,
    (from, to): (f32, f32),
) -> Result<PitchTrack, String> {
    let stft = Stft::new(size, 512);
    let frames = stft.frames(signal.len());
    let bin_of = |f: f64| f * stft.size as f64 / rate;
    let mut best = Vec::with_capacity(frames);
    let mut loud = Vec::with_capacity(frames);
    for t in 0..frames {
        if t % 256 == 0 && !progress(from + (to - from) * t as f32 / frames as f32) {
            return Err("cancelled".into());
        }
        let mag: Vec<f32> = magnitudes(&stft, signal, t)
            .iter()
            .map(|m| m.sqrt())
            .collect();
        let mut top = (low, 0.0f32);
        let mut sum = 0.0f32;
        for pitch in low..=high {
            let f0 = hz(f64::from(pitch));
            let mut salience = 0.0f32;
            for h in 1..=HARMONICS {
                let centre = bin_of(f0 * h as f64);
                if centre + 1.5 >= mag.len() as f64 {
                    break;
                }
                let (lo, hi) = (
                    (centre - 1.0).round().max(0.0) as usize,
                    (centre + 1.0).round() as usize,
                );
                let peak = mag[lo..=hi].iter().copied().fold(0.0, f32::max);
                salience += peak * 0.85f32.powi(h as i32 - 1);
            }
            sum += salience;
            if salience > top.1 {
                top = (pitch, salience);
            }
        }
        // How much the best pitch stands out from the field, so noise doesn't count as a note.
        best.push((top.0, top.1 - sum / f32::from(high - low + 1)));
        loud.push(mag.iter().sum::<f32>());
    }
    Ok((best, loud))
}

/// The value at the 95th percentile, for judging "strong" without one spike deciding it.
fn p95(values: &[f32]) -> f32 {
    let mut v = values.to_vec();
    v.sort_unstable_by(f32::total_cmp);
    v[(v.len() * 95 / 100).min(v.len() - 1)]
}

/// The loudness of `signal` in blocks of `ENV_HOP` samples.
const ENV_HOP: usize = 128;

fn envelope(signal: &[f32]) -> Vec<f32> {
    signal
        .chunks(ENV_HOP)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

/// Moves a note start to the sharpest rise in loudness close to it, if there's a clear one. The
/// pitch analysis looks through long windows and so notices a note a little early; the attack
/// itself says when it really began.
fn refine_onset(env: &[f32], seconds: f64, rate: f64) -> f64 {
    let block = |s: f64| ((s * rate) / ENV_HOP as f64) as isize;
    let (lo, hi) = (block((seconds - 0.06).max(0.0)), block(seconds + 0.22));
    let at = |i: isize| env[i.clamp(0, env.len() as isize - 1) as usize];
    let mut best = (1.0f32, None);
    for i in lo..hi.min(env.len() as isize) {
        // Loudness just after against just before.
        let growth = at(i + 3) / at(i - 2).max(1e-4);
        if growth > best.0 && at(i + 3) > 0.02 {
            best = (growth, Some(i));
        }
    }
    match best {
        (growth, Some(i)) if growth > 2.5 => (i as f64 * ENV_HOP as f64) / rate,
        _ => seconds,
    }
}

fn melody(
    signal: &[f32],
    rate: f64,
    bpm: f64,
    progress: &mut impl FnMut(f32) -> bool,
) -> Result<Vec<Note>, String> {
    // The same frames, found twice: a short transform for the notes of a voice or lead, a long
    // one for the low notes of a bass, where a semitone is only a few Hz apart.
    let stft = Stft::new(4096, 512);
    let frames = stft.frames(signal.len());
    let (mid, mid_loud) = track_pitch(
        signal,
        rate,
        4096,
        (LOW_BAND_TOP + 1, MELODY_HIGH),
        progress,
        (0.0, 0.4),
    )?;
    let (low, low_loud) = track_pitch(
        signal,
        rate,
        16384,
        (MELODY_LOW, LOW_BAND_TOP),
        progress,
        (0.4, 0.8),
    )?;
    // The long transform's frames are centred 6144 samples (12 hops) later than the short one's.
    let low_at = |t: usize| low.get(t + 12).copied().unwrap_or((MELODY_LOW, 0.0));
    // A steady tone's peak grows with the window (four times as long gives twice the square-rooted
    // magnitude), so the long transform is brought back to the short one's scale and the two are
    // compared directly: whichever finds the more convincing pitch wins the frame.
    let best: Vec<(u8, f32)> = (0..frames)
        .map(|t| {
            let (lp, ls) = low_at(t);
            let (mp, ms) = mid[t];
            if 0.5 * ls > ms {
                (lp, 0.5 * ls)
            } else {
                (mp, ms)
            }
        })
        .collect();
    let loud: Vec<f32> = (0..frames)
        .map(|t| mid_loud[t].max(low_loud.get(t + 12).copied().unwrap_or(0.0) * 0.5))
        .collect();

    let strong = p95(&best.iter().map(|b| b.1).collect::<Vec<_>>());
    let floor = 0.08 * p95(&loud);
    let voiced: Vec<Option<u8>> = best
        .iter()
        .zip(&loud)
        .map(|(&(p, s), &l)| (s > 0.3 * strong && l > floor).then_some(p))
        .collect();

    // Median-smooth the pitch so a stray frame doesn't make a note.
    let smooth: Vec<Option<u8>> = (0..frames)
        .map(|t| {
            let (lo, hi) = (t.saturating_sub(2), (t + 3).min(frames));
            let mut near: Vec<u8> = voiced[lo..hi].iter().flatten().copied().collect();
            if near.len() * 2 <= hi - lo || voiced[t].is_none() {
                return None;
            }
            near.sort_unstable();
            Some(near[near.len() / 2])
        })
        .collect();

    // Runs of the same pitch, short ones dropped, nearby same-pitch runs joined.
    let mut runs: Vec<(u8, usize, usize, f32)> = Vec::new(); // pitch, first, last, strength
    for (t, p) in smooth.iter().enumerate() {
        let Some(p) = *p else { continue };
        match runs.last_mut() {
            Some(r) if r.0 == p && t <= r.2 + 3 => {
                r.2 = t;
                r.3 = r.3.max(best[t].1);
            }
            _ => runs.push((p, t, t, best[t].1)),
        }
    }
    let seconds_per_beat = 60.0 / bpm;
    let env = envelope(signal);
    // A short run right beside a longer one an octave away is the smear of the long window
    // finding the lower octave as a note changes, not a note.
    let keep: Vec<bool> = (0..runs.len())
        .map(|i| {
            let (p, first, last, _) = runs[i];
            let len = last + 1 - first;
            let smear = |j: usize| {
                let (q, f, l, _) = runs[j];
                p.abs_diff(q) == 12
                    && l + 1 - f > len
                    && (f.abs_diff(last + 1) <= 3 || first.abs_diff(l + 1) <= 3)
            };
            len >= 4 && !(i > 0 && smear(i - 1)) && !(i + 1 < runs.len() && smear(i + 1))
        })
        .collect();
    let mut notes = Vec::new();
    for (i, (pitch, first, last, strength)) in runs.into_iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let began = refine_onset(&env, frame_time(&stft, first, rate).max(0.0), rate);
        let start = snap(began / seconds_per_beat);
        let end = snap(frame_time(&stft, last + 1, rate) / seconds_per_beat);
        notes.push(Note {
            pitch,
            start,
            length: (end - start).max(GRID),
            velocity: (0.5 + 0.5 * strength / strong).clamp(0.4, 1.0),
        });
    }
    // Snapping can make a note run into the next one; the next one wins.
    for i in 1..notes.len() {
        let next = notes[i].start;
        if notes[i - 1].start + notes[i - 1].length > next {
            notes[i - 1].length = (next - notes[i - 1].start).max(GRID / 2.0);
        }
    }
    progress(1.0);
    Ok(notes)
}

// ---- harmony ----------------------------------------------------------------------------------

/// Chord types: name, the notes above the root, and how much to favour it (sevenths have to earn
/// their place).
const CHORDS: [(&str, &[u8], f32); 5] = [
    ("maj", &[0, 4, 7], 0.0),
    ("min", &[0, 3, 7], 0.0),
    ("7", &[0, 4, 7, 10], 0.08),
    ("maj7", &[0, 4, 7, 11], 0.08),
    ("min7", &[0, 3, 7, 10], 0.08),
];

/// The best chord for a chroma vector: (root, chord index), if anything fits.
fn best_chord(chroma: &[f32; 12]) -> Option<(usize, usize)> {
    let norm = chroma.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-9 {
        return None;
    }
    let mut best = (0.0f32, 0usize, 0usize);
    for root in 0..12 {
        for (c, (_, steps, penalty)) in CHORDS.iter().enumerate() {
            let mut template = [0.0f32; 12];
            for (i, step) in steps.iter().enumerate() {
                // The root counts most, the fifth least.
                template[(root + usize::from(*step)) % 12] = [1.0, 0.85, 0.75, 0.6][i];
            }
            let t_norm = template.iter().map(|x| x * x).sum::<f32>().sqrt();
            let score = template.iter().zip(chroma).map(|(a, b)| a * b).sum::<f32>()
                / (t_norm * norm)
                - penalty;
            if score > best.0 {
                best = (score, root, c);
            }
        }
    }
    (best.0 > 0.6).then_some((best.1, best.2))
}

fn harmony(
    signal: &[f32],
    rate: f64,
    bpm: f64,
    progress: &mut impl FnMut(f32) -> bool,
) -> Result<Vec<Note>, String> {
    let stft = Stft::new(4096, 1024);
    let frames = stft.frames(signal.len());
    let seconds_per_beat = 60.0 / bpm;
    let beats = (signal.len() as f64 / rate / seconds_per_beat).ceil() as usize;
    if beats == 0 {
        return Ok(Vec::new());
    }
    let mut chroma = vec![[0.0f32; 12]; beats];
    let mut weight = vec![0.0f32; beats];
    // Which pitch class each bin belongs to (80 Hz to 3 kHz, where chords live).
    let class: Vec<Option<usize>> = (0..stft.bins())
        .map(|k| {
            let f = k as f64 * rate / stft.size as f64;
            (80.0..3000.0)
                .contains(&f)
                .then(|| (12.0 * (f / 440.0).log2() + 69.0).round().rem_euclid(12.0) as usize)
        })
        .collect();
    for t in 0..frames {
        if t % 128 == 0 && !progress(0.9 * t as f32 / frames as f32) {
            return Err("cancelled".into());
        }
        let time = frame_time(&stft, t, rate);
        if time < 0.0 {
            continue;
        }
        let beat = (time / seconds_per_beat) as usize;
        if beat >= beats {
            break;
        }
        let mag = magnitudes(&stft, signal, t);
        for (k, c) in class.iter().enumerate() {
            if let Some(c) = c {
                // Upper partials spell other notes (the third harmonic of an E is a B), so
                // the low end counts for more.
                let f = k as f32 * rate as f32 / stft.size as f32;
                chroma[beat][*c] += mag[k].sqrt() / (1.0 + (f / 400.0).powi(2));
            }
        }
        weight[beat] += 1.0;
    }
    let energy: Vec<f32> = chroma.iter().map(|c| c.iter().sum::<f32>()).collect();
    let loudest = energy.iter().copied().fold(0.0, f32::max);
    let mut labels: Vec<Option<(usize, usize)>> = (0..beats)
        .map(|b| {
            if weight[b] == 0.0 || energy[b] < 0.05 * loudest {
                None
            } else {
                best_chord(&chroma[b])
            }
        })
        .collect();
    // A single beat that disagrees with the same chord either side is a smudge, not a change.
    for b in 1..beats.saturating_sub(1) {
        if labels[b - 1] == labels[b + 1] && labels[b] != labels[b - 1] {
            labels[b] = labels[b - 1];
        }
    }

    let mut notes = Vec::new();
    let mut b = 0;
    while b < beats {
        let Some((root, chord)) = labels[b] else {
            b += 1;
            continue;
        };
        let mut end = b + 1;
        while end < beats && labels[end] == labels[b] {
            end += 1;
        }
        for step in CHORDS[chord].1 {
            notes.push(Note {
                pitch: (48 + root + usize::from(*step)) as u8,
                start: b as f64,
                length: (end - b) as f64,
                velocity: 0.7,
            });
        }
        b = end;
    }
    progress(1.0);
    Ok(notes)
}

// ---- drums ------------------------------------------------------------------------------------

/// The mean power per Hz in a band of a power spectrum.
fn density(power: &[f32], lo_hz: f64, hi_hz: f64, rate: f64, size: usize) -> f32 {
    let bin = |f: f64| ((f * size as f64 / rate).round() as usize).min(power.len() - 1);
    let (lo, hi) = (bin(lo_hz), bin(hi_hz).max(bin(lo_hz) + 1));
    power[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
}

/// The power spectrum of 2048 samples of `signal` starting at `at`.
fn power_at(signal: &[f32], at: usize) -> Vec<f32> {
    let stft = Stft::new(2048, 2048);
    // With the hop equal to the size, frame 0 is exactly `padded`.
    let end = (at + 2048).min(signal.len());
    let slice = &signal[at.min(signal.len())..end];
    let mut padded = vec![0.0f32; 2048];
    padded[..slice.len()].copy_from_slice(slice);
    let s = stft.analyze(&padded, 0);
    (0..stft.bins())
        .map(|k| s.re[k] * s.re[k] + s.im[k] * s.im[k])
        .collect()
}

fn drums(
    signal: &[f32],
    rate: f64,
    bpm: f64,
    progress: &mut impl FnMut(f32) -> bool,
) -> Result<Vec<Note>, String> {
    let stft = Stft::new(1024, 256);
    let frames = stft.frames(signal.len());
    // Spectral flux: how much new energy appears in each frame.
    let mut flux = vec![0.0f32; frames];
    let mut previous = vec![0.0f32; stft.bins()];
    for (t, slot) in flux.iter_mut().enumerate() {
        if t % 512 == 0 && !progress(0.5 * t as f32 / frames as f32) {
            return Err("cancelled".into());
        }
        let mag: Vec<f32> = magnitudes(&stft, signal, t)
            .iter()
            .map(|m| m.sqrt())
            .collect();
        *slot = mag
            .iter()
            .zip(&previous)
            .map(|(a, b)| (a - b).max(0.0))
            .sum();
        previous = mag;
    }
    let top = flux.iter().copied().fold(0.0, f32::max);
    if top <= 0.0 {
        return Ok(Vec::new());
    }
    // Pick peaks that stand out from their surroundings (about half a second either side).
    let around = (0.5 * rate / stft.hop as f64) as usize;
    let min_gap = (0.045 * rate / stft.hop as f64).ceil() as usize;
    let mut onsets: Vec<usize> = Vec::new();
    for t in 0..frames {
        let (lo, hi) = (t.saturating_sub(around), (t + around + 1).min(frames));
        let local = &flux[lo..hi];
        let mean = local.iter().sum::<f32>() / local.len() as f32;
        let var = local.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / local.len() as f32;
        // A hit right at the start has no frame before it; the end is the same.
        let is_peak =
            (t == 0 || flux[t] >= flux[t - 1]) && (t + 1 >= frames || flux[t] > flux[t + 1]);
        if is_peak && flux[t] > mean + 0.8 * var.sqrt() && flux[t] > 0.12 * top {
            match onsets.last() {
                Some(&last) if t - last < min_gap => {
                    if flux[t] > flux[last] {
                        *onsets.last_mut().expect("just matched") = t;
                    }
                }
                _ => onsets.push(t),
            }
        }
    }

    let seconds_per_beat = 60.0 / bpm;
    let mut hits: Vec<Note> = Vec::new();
    for (i, &t) in onsets.iter().enumerate() {
        if i % 32 == 0 && !progress(0.5 + 0.5 * i as f32 / onsets.len() as f32) {
            return Err("cancelled".into());
        }
        // The frame's flux peak is when the energy arrived; its window is centred a little
        // earlier than the sound, so look from just before the onset.
        let time = frame_time(&stft, t, rate);
        let at = ((time - 0.004) * rate).max(0.0) as usize;
        let power = power_at(signal, at);
        let (low, mid, high) = (
            density(&power, 30.0, 150.0, rate, 2048),
            density(&power, 150.0, 2500.0, rate, 2048),
            density(&power, 4500.0, 12000.0, rate, 2048),
        );
        let strength = (flux[t] / top).clamp(0.0, 1.0);
        let velocity = (0.45 + 0.55 * strength).clamp(0.45, 1.0);
        let start = snap(time.max(0.0) / seconds_per_beat);
        let mut hit = |pitch: u8| {
            hits.push(Note {
                pitch,
                start,
                length: GRID,
                velocity,
            });
        };
        let kick = low > 3.0 * mid;
        let hat = high > 3.0 * mid;
        if kick {
            hit(KICK);
        }
        if hat {
            // Open if the high end is still ringing 120 ms later.
            let later = power_at(signal, at + (0.12 * rate) as usize);
            let after = density(&later, 4500.0, 12000.0, rate, 2048);
            hit(if after > 0.35 * high {
                OPEN_HAT
            } else {
                CLOSED_HAT
            });
        }
        if !kick && !hat {
            hit(SNARE);
        }
    }
    // The same drum twice in one grid slot is one hit: keep the harder one.
    hits.sort_by(|a, b| a.pitch.cmp(&b.pitch).then(a.start.total_cmp(&b.start)));
    let mut kept: Vec<Note> = Vec::new();
    for h in hits {
        match kept.last_mut() {
            Some(last) if last.pitch == h.pitch && (last.start - h.start).abs() < 1e-6 => {
                last.velocity = last.velocity.max(h.velocity);
            }
            _ => kept.push(h),
        }
    }
    kept.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    progress(1.0);
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: f64 = 44_100.0;

    fn sample_of(signal: Vec<f32>) -> Sample {
        Sample::new(signal.clone(), signal, RATE)
    }

    /// A note with a few harmonics, so it has a pitch like an instrument.
    fn tone(out: &mut [f32], from: f64, to: f64, midi: f32, gain: f32) {
        let f = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
        let (a, b) = (
            (from * RATE) as usize,
            ((to * RATE) as usize).min(out.len()),
        );
        for (i, x) in out.iter_mut().enumerate().take(b).skip(a) {
            let t = i as f32 / RATE as f32;
            *x += gain
                * ((TAU * f * t).sin()
                    + 0.5 * (TAU * 2.0 * f * t).sin()
                    + 0.25 * (TAU * 3.0 * f * t).sin());
        }
    }

    #[test]
    fn a_melody_comes_back_as_its_notes() {
        // 120 BPM: a beat is half a second. C4 E4 G4 C5 for a beat each, then a rest, then E4.
        let mut s = vec![0.0f32; (5.0 * RATE) as usize];
        for (i, midi) in [60.0, 64.0, 67.0, 72.0].into_iter().enumerate() {
            tone(&mut s, i as f64 * 0.5, i as f64 * 0.5 + 0.45, midi, 0.2);
        }
        tone(&mut s, 3.0, 3.5, 64.0, 0.2);
        let notes = transcribe(&sample_of(s), 0.0, 5.0, 120.0, Kind::Melody, |_| true).unwrap();
        let got: Vec<(u8, f64)> = notes.iter().map(|n| (n.pitch, n.start)).collect();
        assert_eq!(
            got,
            vec![(60, 0.0), (64, 1.0), (67, 2.0), (72, 3.0), (64, 6.0)],
            "{notes:?}"
        );
        for n in &notes[..4] {
            assert!(
                (n.length - 0.75..=n.length + 0.5).contains(&1.0) || n.length >= 0.75,
                "{n:?}"
            );
        }
    }

    #[test]
    fn chords_come_back_as_chords() {
        // 120 BPM: C major for 2 beats, A minor for 2, G7 for 2.
        let mut s = vec![0.0f32; (3.0 * RATE) as usize];
        for (from, to, notes) in [
            (0.0, 1.0, [60.0, 64.0, 67.0, 0.0]),
            (1.0, 2.0, [57.0, 60.0, 64.0, 0.0]),
            (2.0, 3.0, [55.0, 59.0, 62.0, 65.0]),
        ] {
            for m in notes.into_iter().filter(|m| *m > 0.0) {
                tone(&mut s, from, to, m, 0.15);
            }
        }
        let notes = transcribe(&sample_of(s), 0.0, 3.0, 120.0, Kind::Harmony, |_| true).unwrap();
        let chord_at = |beat: f64| -> Vec<u8> {
            let mut p: Vec<u8> = notes
                .iter()
                .filter(|n| n.start <= beat && beat < n.start + n.length)
                .map(|n| n.pitch % 12)
                .collect();
            p.sort_unstable();
            p
        };
        assert_eq!(chord_at(0.5), vec![0, 4, 7], "C major: {notes:?}");
        assert_eq!(chord_at(2.5), vec![0, 4, 9], "A minor");
        assert_eq!(chord_at(4.5), vec![2, 5, 7, 11], "G7");
        // C major lasts two beats, as one held chord.
        assert!(notes.iter().any(|n| n.start == 0.0 && n.length == 2.0));
    }

    /// A kick, a snare and a hat, as simple stand-ins.
    fn drum_loop() -> Vec<f32> {
        let n = (4.0 * RATE) as usize;
        let mut s = vec![0.0f32; n];
        let mut seed = 12345u32;
        let mut noise = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        // 120 BPM, a beat = 0.5 s. Kick on 0 and 2, snare on 1 and 3, closed hat on every half beat.
        for beat in 0..8 {
            let t0 = beat as f64 * 0.5;
            let at = (t0 * RATE) as usize;
            if beat % 2 == 0 {
                for i in 0..(0.2 * RATE) as usize {
                    let t = i as f32 / RATE as f32;
                    s[at + i] += (TAU * 55.0 * t).sin() * (-t * 18.0).exp() * 0.9;
                }
            } else {
                for i in 0..(0.15 * RATE) as usize {
                    let t = i as f32 / RATE as f32;
                    s[at + i] +=
                        (noise() * 0.5 + (TAU * 190.0 * t).sin() * 0.4) * (-t * 22.0).exp();
                }
            }
            for half in 0..2 {
                let at = ((t0 + half as f64 * 0.25) * RATE) as usize;
                let mut last = 0.0;
                for i in 0..(0.03 * RATE) as usize {
                    // A crude high-pass: the difference of white noise.
                    let x = noise();
                    let t = i as f32 / RATE as f32;
                    s[at + i] += (x - last) * 0.25 * (-t * 120.0).exp();
                    last = x;
                }
            }
        }
        s
    }

    #[test]
    fn drums_are_found_and_told_apart() {
        let notes = transcribe(
            &sample_of(drum_loop()),
            0.0,
            4.0,
            120.0,
            Kind::Drums,
            |_| true,
        )
        .unwrap();
        let at = |pitch: u8| -> Vec<f64> {
            notes
                .iter()
                .filter(|n| n.pitch == pitch)
                .map(|n| n.start)
                .collect()
        };
        assert_eq!(
            at(KICK),
            vec![0.0, 2.0, 4.0, 6.0],
            "kicks on the even beats: {notes:?}"
        );
        assert_eq!(
            at(SNARE),
            vec![1.0, 3.0, 5.0, 7.0],
            "snares on the odd beats"
        );
        // The hats between the beats are all found. (One that lands exactly with a snare is
        // masked by the snare's noise, so it isn't reported on its own.)
        let hats = [at(CLOSED_HAT), at(OPEN_HAT)].concat();
        for beat in 0..8 {
            let off = f64::from(beat) + 0.5;
            assert!(hats.contains(&off), "the hat at {off} is found: {notes:?}");
        }
        assert!(
            notes.iter().all(|n| n.start % 0.25 == 0.0),
            "everything is on the grid"
        );
    }

    #[test]
    fn silence_and_bad_input_are_handled() {
        let silent = sample_of(vec![0.0; 100_000]);
        for kind in Kind::ALL {
            assert_eq!(
                transcribe(&silent, 0.0, 2.0, 120.0, kind, |_| true).unwrap(),
                vec![]
            );
        }
        let tiny = sample_of(vec![0.0; 100]);
        assert!(transcribe(&tiny, 0.0, 1.0, 120.0, Kind::Melody, |_| true).is_err());
        assert!(transcribe(&silent, 0.0, 2.0, 0.0, Kind::Melody, |_| true).is_err());
        let s = sample_of(drum_loop());
        for kind in Kind::ALL {
            assert_eq!(
                transcribe(&s, 0.0, 4.0, 120.0, kind, |_| false)
                    .err()
                    .as_deref(),
                Some("cancelled")
            );
        }
    }
}
