//! Spectral tools shared by the audio analysis features (audio to MIDI, stem separation): a
//! radix-2 FFT and a short-time Fourier transform with perfect-reconstruction overlap-add.

use std::f32::consts::PI;

/// A radix-2 FFT of a fixed power-of-two size.
pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    reverse: Vec<u32>,
}

impl Fft {
    /// An FFT of `n` points. `n` has to be a power of two, at least 2.
    pub fn new(n: usize) -> Self {
        assert!(
            n >= 2 && n.is_power_of_two(),
            "FFT size must be a power of two"
        );
        let bits = n.trailing_zeros();
        Self {
            n,
            cos: (0..n / 2)
                .map(|k| (2.0 * PI * k as f32 / n as f32).cos())
                .collect(),
            sin: (0..n / 2)
                .map(|k| -(2.0 * PI * k as f32 / n as f32).sin())
                .collect(),
            reverse: (0..n as u32)
                .map(|i| i.reverse_bits() >> (32 - bits))
                .collect(),
        }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    /// The forward transform, in place.
    pub fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        self.run(re, im, false);
    }

    /// The inverse transform, in place, scaled so that `inverse(forward(x)) == x`.
    pub fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        self.run(re, im, true);
        let scale = 1.0 / self.n as f32;
        for x in re.iter_mut().chain(im.iter_mut()) {
            *x *= scale;
        }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.n;
        assert!(
            re.len() == n && im.len() == n,
            "buffers must be the FFT size"
        );
        for i in 0..n {
            let j = self.reverse[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let (half, step) = (len / 2, n / len);
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let s = if inverse { -s } else { s };
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * c - im[b] * s;
                    let ti = re[b] * s + im[b] * c;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

/// A short-time Fourier transform: square-root-Hann windows at analysis and synthesis, so that
/// analysing and then resynthesising (with the spectra untouched) gives the signal back.
pub struct Stft {
    fft: Fft,
    pub size: usize,
    pub hop: usize,
    window: Vec<f32>,
    /// What overlapping the squared windows adds up to at each position within a hop.
    norm: Vec<f32>,
}

/// One frame's spectrum: `size / 2 + 1` bins.
#[derive(Clone)]
pub struct Spectrum {
    pub re: Vec<f32>,
    pub im: Vec<f32>,
}

impl Stft {
    /// A transform of `size` points (a power of two) moving `hop` samples a frame.
    pub fn new(size: usize, hop: usize) -> Self {
        assert!(
            hop > 0 && size.is_multiple_of(hop),
            "the hop has to divide the size"
        );
        let window: Vec<f32> = (0..size)
            .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f32 / size as f32).cos()).sqrt())
            .collect();
        let mut norm = vec![0.0f32; hop];
        for (i, slot) in norm.iter_mut().enumerate() {
            *slot = (0..size / hop)
                .map(|k| window[i + k * hop].powi(2))
                .sum::<f32>();
        }
        Self {
            fft: Fft::new(size),
            size,
            hop,
            window,
            norm,
        }
    }

    pub fn bins(&self) -> usize {
        self.size / 2 + 1
    }

    /// How many frames cover `samples` samples (the signal is padded by one window each side).
    pub fn frames(&self, samples: usize) -> usize {
        (samples + self.size).div_ceil(self.hop) + 1
    }

    /// The spectrum of frame `t` of `signal`. Frame 0 starts one window before the signal, so
    /// every sample is covered by `size / hop` frames.
    pub fn analyze(&self, signal: &[f32], t: usize) -> Spectrum {
        let start = (t * self.hop) as isize - self.size as isize + self.hop as isize;
        let mut re = vec![0.0f32; self.size];
        let mut im = vec![0.0f32; self.size];
        for (i, (slot, w)) in re.iter_mut().zip(&self.window).enumerate() {
            let at = start + i as isize;
            if at >= 0 && (at as usize) < signal.len() {
                *slot = signal[at as usize] * w;
            }
        }
        self.fft.forward(&mut re, &mut im);
        re.truncate(self.bins());
        im.truncate(self.bins());
        Spectrum { re, im }
    }

    /// Adds the windowed inverse of `spectrum` (frame `t`) into `out`. After every frame has
    /// been added, call [`Stft::finish`].
    pub fn synthesize_into(&self, spectrum: &Spectrum, t: usize, out: &mut [f32]) {
        let n = self.size;
        let mut re = vec![0.0f32; n];
        let mut im = vec![0.0f32; n];
        re[..self.bins()].copy_from_slice(&spectrum.re);
        im[..self.bins()].copy_from_slice(&spectrum.im);
        // The upper half mirrors the lower, conjugated, so the signal is real.
        for k in 1..n / 2 {
            re[n - k] = re[k];
            im[n - k] = -im[k];
        }
        self.fft.inverse(&mut re, &mut im);
        let start = (t * self.hop) as isize - n as isize + self.hop as isize;
        for (i, (x, w)) in re.iter().zip(&self.window).enumerate() {
            let at = start + i as isize;
            if at >= 0 && (at as usize) < out.len() {
                out[at as usize] += x * w;
            }
        }
    }

    /// Divides out the overlap so the output is the right level. Call once, after the last
    /// frame is in.
    pub fn finish(&self, out: &mut [f32]) {
        // Frame `t` starts at `t * hop - size + hop`, so sample `s` sits at `s % hop` within a
        // hop in the same way for every frame that covers it.
        for (s, x) in out.iter_mut().enumerate() {
            let norm = self.norm[s % self.hop];
            if norm > 1e-6 {
                *x /= norm;
            }
        }
    }
}

/// The frequency of bin `k` of a transform of `size` points at `rate` Hz.
pub fn bin_hz(k: usize, size: usize, rate: f64) -> f64 {
    k as f64 * rate / size as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fft_finds_a_tone_and_inverts() {
        let n = 1024;
        let fft = Fft::new(n);
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * 50.0 * i as f32 / n as f32).sin())
            .collect();
        let original = re.clone();
        let mut im = vec![0.0f32; n];
        fft.forward(&mut re, &mut im);
        let peak = (0..n / 2)
            .max_by(|&a, &b| (re[a].hypot(im[a])).total_cmp(&re[b].hypot(im[b])))
            .unwrap();
        assert_eq!(peak, 50);
        assert!((re[50].hypot(im[50]) - n as f32 / 2.0).abs() < 0.5);
        fft.inverse(&mut re, &mut im);
        for (a, b) in re.iter().zip(&original) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn analysis_then_synthesis_gives_the_signal_back() {
        let stft = Stft::new(2048, 512);
        let signal: Vec<f32> = (0..10_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.7 + (i as f32 * 0.31).sin() * 0.2)
            .collect();
        let mut out = vec![0.0f32; signal.len()];
        for t in 0..stft.frames(signal.len()) {
            let spectrum = stft.analyze(&signal, t);
            stft.synthesize_into(&spectrum, t, &mut out);
        }
        stft.finish(&mut out);
        let worst = signal
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-3, "worst error {worst}");
    }
}
