//! A small CPU drawing surface: RGBA pixels with fades for trails, additive blending for glow,
//! and soft-edged shapes. CPU drawing works everywhere (headless, CI) and gives the same frames
//! every time; a GPU path can come with custom shaders.

/// An RGB colour, 0 to 1 per channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    /// From hue (0 to 1, wrapping), saturation and value.
    pub fn hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(1.0) * 6.0;
        let i = h.floor();
        let f = h - i;
        let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
        match i as i32 {
            0 => Rgb(v, t, p),
            1 => Rgb(q, v, p),
            2 => Rgb(p, v, t),
            3 => Rgb(p, q, v),
            4 => Rgb(t, p, v),
            _ => Rgb(v, p, q),
        }
    }

    pub fn scale(self, k: f32) -> Self {
        Rgb(self.0 * k, self.1 * k, self.2 * k)
    }
}

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    /// Linear-ish RGB, 0 to 1, three floats per pixel.
    pixels: Vec<f32>,
    /// The tone curve as a table, so converting a frame needs no exp or sqrt per pixel.
    tone: Vec<u8>,
}

/// Entries per unit of brightness in the tone table, and how bright it goes.
const TONE_STEPS: f32 = 512.0;
const TONE_MAX: f32 = 8.0;

/// How many threads to split full-frame work across.
fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 16)
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        let tone = (0..=(TONE_STEPS * TONE_MAX) as usize)
            .map(|i| {
                let v = i as f32 / TONE_STEPS;
                let toned = 1.0 - (-v * 1.4).exp();
                (toned.sqrt() * 255.0).round() as u8
            })
            .collect();
        Self {
            width,
            height,
            pixels: vec![0.0; width * height * 3],
            tone,
        }
    }

    /// The shorter side, for sizing things the same way in every aspect.
    pub fn short_side(&self) -> f32 {
        self.width.min(self.height) as f32
    }

    /// Multiplies everything by `k` (below 1 leaves fading trails).
    pub fn fade(&mut self, k: f32) {
        let chunk = self.pixels.len().div_ceil(threads()).max(3 * 1024);
        std::thread::scope(|scope| {
            for part in self.pixels.chunks_mut(chunk) {
                scope.spawn(move || {
                    for p in part {
                        *p *= k;
                    }
                });
            }
        });
    }

    pub fn fill(&mut self, c: Rgb) {
        for px in self.pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&[c.0, c.1, c.2]);
        }
    }

    fn add(&mut self, x: usize, y: usize, c: Rgb, a: f32) {
        let i = (y * self.width + x) * 3;
        self.pixels[i] += c.0 * a;
        self.pixels[i + 1] += c.1 * a;
        self.pixels[i + 2] += c.2 * a;
    }

    /// Adds `c` over the rectangle (clipped to the canvas).
    pub fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb, a: f32) {
        let (xa, xb) = self.span(x0.min(x1), x0.max(x1), self.width);
        let (ya, yb) = self.span(y0.min(y1), y0.max(y1), self.height);
        for y in ya..yb {
            for x in xa..xb {
                self.add(x, y, c, a);
            }
        }
    }

    /// Adds a soft-edged disc.
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        self.shape(cx, cy, r + 1.5, c, a, |d| (r + 1.0 - d).clamp(0.0, 1.0));
    }

    /// Adds a glow: bright in the middle, fading out to `r`.
    pub fn glow(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        self.shape(cx, cy, r, c, a, |d| {
            let t = (1.0 - d / r).max(0.0);
            t * t
        });
    }

    /// Adds a ring of the given width.
    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, width: f32, c: Rgb, a: f32) {
        let half = width / 2.0;
        self.shape(cx, cy, r + half + 1.5, c, a, |d| {
            (half + 1.0 - (d - r).abs()).clamp(0.0, 1.0)
        });
    }

    fn span(&self, lo: f32, hi: f32, max: usize) -> (usize, usize) {
        let lo = lo.floor().clamp(0.0, max as f32) as usize;
        let hi = hi.ceil().clamp(0.0, max as f32) as usize;
        (lo, hi)
    }

    fn shape(
        &mut self,
        cx: f32,
        cy: f32,
        reach: f32,
        c: Rgb,
        a: f32,
        coverage: impl Fn(f32) -> f32,
    ) {
        if !(cx.is_finite() && cy.is_finite() && reach.is_finite()) || reach <= 0.0 {
            return;
        }
        let (xa, xb) = self.span(cx - reach, cx + reach, self.width);
        let (ya, yb) = self.span(cy - reach, cy + reach, self.height);
        for y in ya..yb {
            let dy = y as f32 + 0.5 - cy;
            for x in xa..xb {
                let dx = x as f32 + 0.5 - cx;
                let d = (dx * dx + dy * dy).sqrt();
                if d <= reach {
                    let k = coverage(d);
                    if k > 0.0 {
                        self.add(x, y, c, a * k);
                    }
                }
            }
        }
    }

    /// The frame as 8-bit RGBA, with a soft roll-off so bright overlaps don't clip harshly.
    pub fn to_rgba(&self, out: &mut Vec<u8>) {
        let pixels = self.width * self.height;
        out.resize(pixels * 4, 255);
        let per = pixels.div_ceil(threads()).max(1024);
        let tone = &self.tone;
        let last = tone.len() - 1;
        std::thread::scope(|scope| {
            for (dst, src) in out.chunks_mut(per * 4).zip(self.pixels.chunks(per * 3)) {
                scope.spawn(move || {
                    for (o, px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                        for (c, &v) in o.iter_mut().zip(px) {
                            let i = if v.is_finite() {
                                (v.max(0.0) * TONE_STEPS) as usize
                            } else {
                                0
                            };
                            *c = tone[i.min(last)];
                        }
                        o[3] = 255;
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_clips_and_fades() {
        let mut c = Canvas::new(64, 32);
        c.disc(32.0, 16.0, 6.0, Rgb(1.0, 0.0, 0.0), 1.0);
        c.ring(-100.0, -100.0, 5.0, 2.0, Rgb(1.0, 1.0, 1.0), 1.0); // off the canvas: nothing
        c.rect(60.0, 30.0, 200.0, 200.0, Rgb(0.0, 1.0, 0.0), 1.0); // clipped, no panic
        let mut rgba = Vec::new();
        c.to_rgba(&mut rgba);
        assert_eq!(rgba.len(), 64 * 32 * 4);
        let at = |x: usize, y: usize| &rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert!(at(32, 16)[0] > 200 && at(32, 16)[1] == 0);
        assert_eq!(at(0, 0), &[0, 0, 0, 255]);
        assert!(at(63, 31)[1] > 200);
        c.fade(0.0);
        c.to_rgba(&mut rgba);
        assert!(rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));
    }

    #[test]
    fn hsv_wraps() {
        assert_eq!(Rgb::hsv(0.0, 1.0, 1.0), Rgb(1.0, 0.0, 0.0));
        assert_eq!(Rgb::hsv(1.0, 1.0, 1.0), Rgb::hsv(0.0, 1.0, 1.0));
    }
}
