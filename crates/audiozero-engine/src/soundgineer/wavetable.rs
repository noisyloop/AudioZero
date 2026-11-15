//! Wavetables: the built-in tables Soundgineer ships with, generated at start-up.
//!
//! A table is [`FRAMES`] single cycles of [`FRAME_SIZE`] samples. Each table is made from a few
//! key frames (additive recipes, FM, formulas) and the frames between them come from FFT-based
//! spectral morphing: magnitudes and phases are interpolated in the frequency domain, which is
//! what makes a morph sweep move smoothly instead of crossfading. Every frame also gets a
//! pyramid of band-limited copies ([`MIPS`] of them, one per octave) so that high notes never
//! alias.
//!
//! The tables are built once, the first time a synth is created, and shared by every instance.

use std::f64::consts::PI;
use std::sync::OnceLock;

pub const FRAME_SIZE: usize = 2048;
/// Frames per built-in table.
pub const FRAMES: usize = 32;
/// Band-limited copies of each frame: mip `m` keeps harmonics up to `1024 >> m`.
pub const MIPS: usize = 11;

/// The built-in tables' names, in the order the oscillators choose them.
pub const NAMES: [&str; 6] = [
    "Basic Shapes",
    "Harmonic Sweep",
    "PWM",
    "Vocal",
    "FM Bell",
    "Digital",
];

/// In-place complex FFT; the length has to be a power of two.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    assert!(
        n.is_power_of_two() && im.len() == n,
        "fft size must be a power of two"
    );
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        let (wr, wi) = (angle.cos(), angle.sin());
        let half = len >> 1;
        for start in (0..n).step_by(len) {
            let (mut cwr, mut cwi) = (1.0f64, 0.0f64);
            for k in 0..half {
                let (a, b) = (start + k, start + k + half);
                let (ur, ui) = (f64::from(re[a]), f64::from(im[a]));
                let vr = f64::from(re[b]) * cwr - f64::from(im[b]) * cwi;
                let vi = f64::from(re[b]) * cwi + f64::from(im[b]) * cwr;
                re[a] = (ur + vr) as f32;
                im[a] = (ui + vi) as f32;
                re[b] = (ur - vr) as f32;
                im[b] = (ui - vi) as f32;
                let next = cwr * wr - cwi * wi;
                cwi = cwr * wi + cwi * wr;
                cwr = next;
            }
        }
        len <<= 1;
    }
}

/// In-place inverse FFT, including the 1/n scaling.
pub fn ifft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len() as f32;
    im.iter_mut().for_each(|x| *x = -*x);
    fft(re, im);
    re.iter_mut().for_each(|x| *x /= n);
    im.iter_mut().for_each(|x| *x = -*x / n);
}

/// Keeps harmonics 1 to `max_harmonic` of a cycle and drops the rest (and DC).
pub fn bandlimit(cycle: &[f32], max_harmonic: usize) -> Vec<f32> {
    let n = cycle.len();
    let mut re = cycle.to_vec();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    re[0] = 0.0;
    im[0] = 0.0;
    let half = n >> 1;
    for k in (max_harmonic + 1)..=half {
        re[k] = 0.0;
        im[k] = 0.0;
        if k != half {
            re[n - k] = 0.0;
            im[n - k] = 0.0;
        }
    }
    ifft(&mut re, &mut im);
    re
}

fn normalize(data: &mut [f32]) {
    let peak = data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 1e-9 {
        let gain = 1.0 / peak;
        data.iter_mut().for_each(|x| *x *= gain);
    }
}

/// `frames` frames whose spectra move smoothly through the key frames.
fn spectral_morph(key_frames: &[Vec<f32>], frames: usize) -> Vec<f32> {
    let n = FRAME_SIZE;
    let half = n >> 1;
    let specs: Vec<(Vec<f32>, Vec<f32>)> = key_frames
        .iter()
        .map(|frame| {
            let mut re = frame.clone();
            let mut im = vec![0.0; n];
            fft(&mut re, &mut im);
            // Computed in 64 bits and stored in 32, as the original does: a bin whose phase
            // difference is almost exactly pi has to wrap the same way.
            let mag = (0..half)
                .map(|k| f64::from(re[k]).hypot(f64::from(im[k])) as f32)
                .collect();
            let phase = (0..half)
                .map(|k| f64::from(im[k]).atan2(f64::from(re[k])) as f32)
                .collect();
            (mag, phase)
        })
        .collect();

    let mut out = vec![0.0; frames * n];
    let (mut re, mut im) = (vec![0.0f32; n], vec![0.0f32; n]);
    for f in 0..frames {
        let pos = if frames == 1 {
            0.0
        } else {
            (f as f64 / (frames - 1) as f64 * (specs.len() - 1) as f64) as f32
        };
        let i0 = (pos.floor() as usize).min(specs.len() - 1);
        let i1 = (i0 + 1).min(specs.len() - 1);
        let t = f64::from(pos) - i0 as f64;
        re.fill(0.0);
        im.fill(0.0);
        for k in 1..half {
            let (m0, m1) = (f64::from(specs[i0].0[k]), f64::from(specs[i1].0[k]));
            let m = m0 + (m1 - m0) * t;
            let p0 = f64::from(specs[i0].1[k]);
            let mut dp = f64::from(specs[i1].1[k]) - p0;
            if dp > PI {
                dp -= 2.0 * PI;
            }
            if dp < -PI {
                dp += 2.0 * PI;
            }
            let phase = p0 + dp * t;
            re[k] = (m * phase.cos()) as f32;
            im[k] = (m * phase.sin()) as f32;
            re[n - k] = re[k];
            im[n - k] = -im[k];
        }
        ifft(&mut re, &mut im);
        out[f * n..(f + 1) * n].copy_from_slice(&re);
    }
    normalize(&mut out);
    out
}

// ---------------------------------------------------------------------------- key frames

/// A frame from its harmonics' amplitudes (sine phase), up to `count` of them.
fn additive(harmonics: impl Fn(usize) -> f64, count: usize) -> Vec<f32> {
    let n = FRAME_SIZE;
    let (mut re, mut im) = (vec![0.0f32; n], vec![0.0f32; n]);
    for k in 1..=count.min(n / 2 - 1) {
        let a = harmonics(k);
        if a == 0.0 {
            continue;
        }
        im[k] = (-a * n as f64 / 2.0) as f32;
        im[n - k] = (a * n as f64 / 2.0) as f32;
    }
    ifft(&mut re, &mut im);
    re
}

fn sine_frame() -> Vec<f32> {
    additive(|k| if k == 1 { 1.0 } else { 0.0 }, 512)
}

fn tri_frame() -> Vec<f32> {
    additive(
        |k| {
            if k % 2 == 1 {
                let sign = if k % 4 == 1 { 1.0 } else { -1.0 };
                sign * 8.0 / (PI * PI * (k * k) as f64)
            } else {
                0.0
            }
        },
        512,
    )
}

fn saw_frame() -> Vec<f32> {
    additive(|k| 2.0 / PI * (1.0 / k as f64), 800)
}

fn square_frame() -> Vec<f32> {
    additive(
        |k| {
            if k % 2 == 1 {
                4.0 / (PI * k as f64)
            } else {
                0.0
            }
        },
        800,
    )
}

fn pwm_frame(width: f64) -> Vec<f32> {
    additive(
        |k| {
            let k = k as f64;
            2.0 / (PI * k) * (PI * k * width).sin() * 2.0
        },
        600,
    )
}

fn fm_frame(index: f64, ratio: f64) -> Vec<f32> {
    (0..FRAME_SIZE)
        .map(|i| {
            let phase = i as f64 / FRAME_SIZE as f64 * 2.0 * PI;
            (phase + index * (ratio * phase).sin()).sin() as f32
        })
        .collect()
}

fn vocal_frame(f1: f64, f2: f64, f3: f64) -> Vec<f32> {
    additive(
        |k| {
            let k = k as f64;
            let peak = |c: f64, w: f64| (-((k - c) * (k - c)) / (2.0 * w * w)).exp();
            1.0 / k.sqrt() * (peak(f1, 2.0) + 0.7 * peak(f2, 3.0) + 0.4 * peak(f3, 4.0))
        },
        128,
    )
}

/// Hard-edged random steps, the same for the same seed.
fn digital_frame(seed: u32) -> Vec<f32> {
    let n = FRAME_SIZE;
    let mut state = seed;
    let mut rand = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state as f32 / 4_294_967_295.0 * 2.0 - 1.0
    };
    let steps = 8 + (seed % 24) as usize;
    let levels: Vec<f32> = (0..steps).map(|_| rand()).collect();
    let mut out: Vec<f32> = (0..n).map(|i| levels[i * steps / n]).collect();
    let mean = out.iter().sum::<f32>() / n as f32;
    out.iter_mut().for_each(|x| *x -= mean);
    out
}

/// The frames of built-in table `index` (see [`NAMES`]), concatenated.
fn generate(index: usize) -> Vec<f32> {
    match index {
        0 => spectral_morph(
            &[sine_frame(), tri_frame(), saw_frame(), square_frame()],
            FRAMES,
        ),
        1 => {
            let keys: Vec<Vec<f32>> = [1usize, 2, 4, 8, 16, 32]
                .iter()
                .map(|&h| additive(|k| if k <= h { 1.0 / (k as f64).sqrt() } else { 0.0 }, 64))
                .collect();
            spectral_morph(&keys, FRAMES)
        }
        2 => {
            let mut data = vec![0.0; FRAMES * FRAME_SIZE];
            for f in 0..FRAMES {
                let width = 0.5 - f as f64 / (FRAMES - 1) as f64 * 0.45;
                data[f * FRAME_SIZE..(f + 1) * FRAME_SIZE].copy_from_slice(&pwm_frame(width));
            }
            normalize(&mut data);
            data
        }
        3 => spectral_morph(
            &[
                vocal_frame(6.0, 9.0, 22.0),
                vocal_frame(4.0, 16.0, 24.0),
                vocal_frame(2.0, 20.0, 28.0),
                vocal_frame(3.0, 7.0, 21.0),
                vocal_frame(2.0, 6.0, 18.0),
            ],
            FRAMES,
        ),
        4 => spectral_morph(
            &[
                fm_frame(0.5, 2.0),
                fm_frame(2.0, 2.0),
                fm_frame(4.0, 3.01),
                fm_frame(7.0, 5.0),
            ],
            FRAMES,
        ),
        _ => spectral_morph(
            &[
                digital_frame(7),
                digital_frame(1234),
                digital_frame(9876),
                digital_frame(31415),
            ],
            FRAMES,
        ),
    }
}

/// One wavetable with all its band-limited copies.
pub struct Table {
    /// `[(frame * MIPS + mip) * FRAME_SIZE ..]`.
    mips: Vec<f32>,
}

impl Table {
    fn build(index: usize) -> Self {
        let data = generate(index);
        let mut mips = Vec::with_capacity(FRAMES * MIPS * FRAME_SIZE);
        for f in 0..FRAMES {
            let cycle = &data[f * FRAME_SIZE..(f + 1) * FRAME_SIZE];
            for m in 0..MIPS {
                if m == 0 {
                    mips.extend_from_slice(cycle);
                } else {
                    mips.extend(bandlimit(cycle, ((FRAME_SIZE >> 1) >> m).max(1)));
                }
            }
        }
        Self { mips }
    }

    fn cycle(&self, frame: usize, mip: usize) -> &[f32] {
        let at = (frame * MIPS + mip) * FRAME_SIZE;
        &self.mips[at..at + FRAME_SIZE]
    }

    /// The mip that keeps every harmonic below Nyquist at `phase_inc` cycles per sample.
    pub fn mip_for(&self, phase_inc: f32) -> usize {
        let audible = 0.5 / phase_inc.max(1e-9);
        let mut m = 0;
        while m < MIPS - 1 && (1024 >> m) as f32 > audible {
            m += 1;
        }
        m
    }

    /// The sample at `phase` (0 to 1) of the morph position `pos` (0 to 1), between two frames
    /// and two samples linearly.
    pub fn read(&self, pos: f32, phase: f32, mip: usize) -> f32 {
        let fpos = pos.clamp(0.0, 1.0) * (FRAMES - 1) as f32;
        let f0 = (fpos as usize).min(FRAMES - 2);
        let ft = fpos - f0 as f32;
        let idx = phase * FRAME_SIZE as f32;
        let i0 = (idx as usize).min(FRAME_SIZE - 1);
        let st = idx - i0 as f32;
        let i1 = if i0 + 1 >= FRAME_SIZE { 0 } else { i0 + 1 };
        let a = self.cycle(f0, mip);
        let s0 = a[i0] + (a[i1] - a[i0]) * st;
        if ft == 0.0 {
            return s0;
        }
        let b = self.cycle(f0 + 1, mip);
        let s1 = b[i0] + (b[i1] - b[i0]) * st;
        s0 + (s1 - s0) * ft
    }
}

/// All the built-in tables, built on first use and shared from then on.
pub fn tables() -> &'static [Table; 6] {
    static TABLES: OnceLock<[Table; 6]> = OnceLock::new();
    TABLES.get_or_init(|| std::array::from_fn(Table::build))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fft_round_trips_and_finds_a_sine() {
        let n = 256;
        let signal: Vec<f32> = (0..n)
            .map(|i| (i as f32 / n as f32 * std::f32::consts::TAU * 5.0).sin())
            .collect();
        let (mut re, mut im) = (signal.clone(), vec![0.0; n]);
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..n / 2).map(|k| re[k].hypot(im[k])).collect();
        let peak = (0..n / 2)
            .max_by(|&a, &b| mags[a].total_cmp(&mags[b]))
            .unwrap();
        assert_eq!(peak, 5);
        assert!((mags[5] - n as f32 / 2.0).abs() < 0.01);
        ifft(&mut re, &mut im);
        for (a, b) in re.iter().zip(&signal) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn bandlimiting_removes_the_high_harmonics() {
        let saw = saw_frame();
        let low = bandlimit(&saw, 8);
        let (mut re, mut im) = (low.clone(), vec![0.0; FRAME_SIZE]);
        fft(&mut re, &mut im);
        let mag = |k: usize| re[k].hypot(im[k]);
        assert!(mag(3) > 100.0, "harmonic 3 stays");
        assert!(
            mag(9) < 1e-2 * mag(3) && mag(40) < 1e-2 * mag(3),
            "the rest goes"
        );
        assert!(low.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn built_in_tables_are_normalised_and_move() {
        let tables = tables();
        for (i, table) in tables.iter().enumerate() {
            let mut peak = 0.0f32;
            for f in 0..FRAMES {
                for x in table.cycle(f, 0) {
                    assert!(x.is_finite(), "{} is finite", NAMES[i]);
                    peak = peak.max(x.abs());
                }
            }
            assert!((0.5..=1.001).contains(&peak), "{}: peak {peak}", NAMES[i]);
            let first = table.cycle(0, 0);
            let last = table.cycle(FRAMES - 1, 0);
            let diff: f32 = first.iter().zip(last).map(|(a, b)| (a - b).abs()).sum();
            assert!(
                diff > 50.0,
                "{} changes along its length ({diff})",
                NAMES[i]
            );
        }
        // The first Basic Shapes frame is a sine and the last a square-ish wave.
        let sine = tables[0].cycle(0, 0);
        // (The whole table is normalised together, so the sine peaks a little under 1.)
        let top = sine.iter().fold(0.0f32, |m, x| m.max(*x));
        assert!(top > 0.6 && (sine[512] - top).abs() < 0.01 && sine[0].abs() < 0.01);
        let square = tables[0].cycle(FRAMES - 1, 0);
        assert!(square[256] > 0.5 && square[1536] < -0.5);
    }

    #[test]
    fn reading_interpolates_and_picks_a_mip_that_avoids_aliasing() {
        let t = &tables()[0];
        assert!(t.read(0.0, 0.25, 0) > 0.6, "a sine's peak");
        let between = t.read(0.5, 0.1, 0);
        assert!(between.is_finite() && between.abs() <= 1.2);
        assert_eq!(t.mip_for(0.0002), 0, "a very low note uses the full table");
        assert!(
            t.mip_for(0.001) <= 2,
            "a low one barely needs band-limiting"
        );
        assert!(
            t.mip_for(0.2) >= 7,
            "a very high one a heavily band-limited copy"
        );
        assert!(t.mip_for(0.4) > t.mip_for(0.01));
        // The last mip keeps (almost) only the fundamental.
        let top = t.mip_for(0.45);
        assert!(top < MIPS);
        // Out-of-range positions are clamped, not a panic.
        let _ = t.read(5.0, 0.99999, 3);
        let _ = t.read(-1.0, 0.0, 0);
    }
}

#[cfg(test)]
mod compare_with_original {
    use super::*;

    /// Samples of the TypeScript original's tables (`generateWavetable` in
    /// `src/shared/wavetable-gen.ts`), at frame and sample positions [`PROBES`].
    const ORIGINAL: [[f32; 6]; 6] = [
        [0.18123, 0.60007, 0.46695, 0.19832, 0.59940, -0.59959],
        [0.05027, 0.16645, 0.21217, -0.02273, 0.17262, -0.11183],
        [0.04044, 1.00000, 0.19104, 0.00426, 0.05017, -0.02513],
        [0.77936, 0.00598, 0.00077, 0.00599, 0.48694, -0.22057],
        [0.39192, 0.69953, 0.45391, 0.15351, 0.59709, 0.59599],
        [-0.36608, 0.45823, 0.16257, 0.15073, -0.09377, -0.66292],
    ];
    const PROBES: [(usize, usize); 6] = [
        (0, 100),
        (0, 512),
        (5, 300),
        (16, 1000),
        (31, 256),
        (31, 1600),
    ];

    /// The port makes the same tables as the original, sample for sample (to the five decimals
    /// the reference values were printed with).
    #[test]
    fn the_tables_match_the_typescript_original() {
        for (i, name) in NAMES.iter().enumerate() {
            for (&(frame, sample), &expected) in PROBES.iter().zip(&ORIGINAL[i]) {
                let got = tables()[i].cycle(frame, 0)[sample];
                assert!(
                    (got - expected).abs() < 2e-5,
                    "{name} frame {frame} sample {sample}: {got} vs {expected}"
                );
            }
        }
    }
}
