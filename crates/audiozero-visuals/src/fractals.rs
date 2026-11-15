//! Raymarched 3D fractals, drawn on the CPU. Each formula is a distance estimator (how far a
//! point is from the surface); a sphere-tracing march finds the surface and shades it. The
//! camera, folding, colour and glow follow the song. Deterministic, like every look.

use std::ops::{Add, Mul, Sub};

use crate::analysis::Features;
use crate::canvas::{Canvas, Rgb};
use crate::looks::Look;

#[derive(Debug, Clone, Copy, PartialEq)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    fn splat(a: f32) -> Self {
        v3(a, a, a)
    }
    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    fn normalize(self) -> Self {
        self * (1.0 / self.length().max(1e-9))
    }
    fn abs(self) -> Self {
        v3(self.x.abs(), self.y.abs(), self.z.abs())
    }
    fn clamp(self, lo: f32, hi: f32) -> Self {
        v3(
            self.x.clamp(lo, hi),
            self.y.clamp(lo, hi),
            self.z.clamp(lo, hi),
        )
    }
    fn cross(self, o: Self) -> Self {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn max_component(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, k: f32) -> V3 {
        v3(self.x * k, self.y * k, self.z * k)
    }
}

/// GLSL's `mod`: the result has the sign of the divisor.
fn modulo(x: f32, y: f32) -> f32 {
    x - y * (x / y).floor()
}

fn mod3(p: V3, y: f32) -> V3 {
    v3(modulo(p.x, y), modulo(p.y, y), modulo(p.z, y))
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// A distance estimator: the formula's id, its name, and the estimator for a point, given the
/// scale (`sc`) and fold (`fo`) the song steers.
pub struct Formula {
    pub id: &'static str,
    pub name: &'static str,
    de: fn(V3, f32, f32) -> f32,
}

fn mandelbulb(p: V3, _sc: f32, _fo: f32) -> f32 {
    let mut z = p;
    let mut dr = 1.0;
    let mut r = 0.0;
    for _ in 0..7 {
        r = z.length();
        if r > 2.0 {
            break;
        }
        let theta = (z.z / r.max(1e-4)).clamp(-1.0, 1.0).acos() * 8.0;
        let phi = z.y.atan2(z.x) * 8.0;
        dr = r.powi(7) * 8.0 * dr + 1.0;
        let zr = r.powi(8);
        z = v3(
            theta.sin() * phi.cos(),
            phi.sin() * theta.sin(),
            theta.cos(),
        ) * zr
            + p;
    }
    0.5 * r.max(0.001).ln() * r / dr.max(0.001)
}

fn menger(p: V3, _sc: f32, _fo: f32) -> f32 {
    let mut d = p.abs().max_component() - 1.0;
    let mut s = 1.0;
    for _ in 0..6 {
        let a = mod3(p * s, 2.0) - V3::splat(1.0);
        s *= 3.0;
        let r = V3::splat(1.0) - a.abs() * 3.0;
        let r = r.abs();
        let da = r.x.max(r.y);
        let db = r.y.max(r.z);
        let dc = r.z.max(r.x);
        d = d.max((da.min(db.min(dc)) - 1.0) / s);
    }
    d
}

fn sierpinski(mut p: V3, sc: f32, fo: f32) -> f32 {
    let mut s = 1.0;
    for _ in 0..10 {
        if p.x + p.y < 0.0 {
            p = v3(-p.y, -p.x, p.z);
        }
        if p.x + p.z < 0.0 {
            p = v3(-p.z, p.y, -p.x);
        }
        if p.y + p.z < 0.0 {
            p = v3(p.x, -p.z, -p.y);
        }
        p = p * sc - V3::splat(sc - 1.0) * fo;
        s *= sc;
    }
    p.length() / s - 0.002
}

fn apollonian(mut p: V3, _sc: f32, _fo: f32) -> f32 {
    let mut s = 1.0;
    for _ in 0..8 {
        p = v3(
            -1.0 + 2.0 * fract(0.5 * p.x + 0.5),
            -1.0 + 2.0 * fract(0.5 * p.y + 0.5),
            -1.0 + 2.0 * fract(0.5 * p.z + 0.5),
        );
        let r2 = p.dot(p).max(1e-6);
        let k = 1.2 / r2;
        p = p * k;
        s *= k;
    }
    0.25 * p.y.abs() / s
}

fn kali(p: V3, sc: f32, fo: f32) -> f32 {
    let mut z = p;
    for _ in 0..8 {
        z = z.abs();
        if z.x < z.y {
            z = v3(z.y, z.x, z.z);
        }
        if z.x < z.z {
            z = v3(z.z, z.y, z.x);
        }
        if z.y < z.z {
            z = v3(z.x, z.z, z.y);
        }
        z = z - V3::splat(z.dot(V3::splat(1.0)));
        z = z.clamp(-fo, fo) * 2.0 - z;
        z = z * sc - p * (sc - 1.0);
    }
    z.length() / sc.powi(8)
}

fn tetra(p: V3, sc: f32, fo: f32) -> f32 {
    let mut z = p;
    let mut s = 1.0;
    for _ in 0..6 {
        z = z.abs();
        z = v3(0.866 * z.x + 0.5 * z.y, -0.5 * z.x + 0.866 * z.y, z.z);
        z = v3(0.866 * z.x + 0.5 * z.z, z.y, -0.5 * z.x + 0.866 * z.z);
        z = z - V3::splat(0.5 * fo);
        z = z * sc;
        s *= sc;
    }
    z.length() / s - 0.05
}

/// The fractals the visuals can draw.
pub const FORMULAS: [Formula; 6] = [
    Formula {
        id: "mandelbulb",
        name: "Mandelbulb",
        de: mandelbulb,
    },
    Formula {
        id: "menger",
        name: "Menger sponge",
        de: menger,
    },
    Formula {
        id: "sierpinski",
        name: "Sierpinski",
        de: sierpinski,
    },
    Formula {
        id: "apollonian",
        name: "Apollonian",
        de: apollonian,
    },
    Formula {
        id: "kali",
        name: "Kali",
        de: kali,
    },
    Formula {
        id: "tetra",
        name: "Tetra",
        de: tetra,
    },
];

/// Pixels the march is allowed per frame: the picture is drawn at this size and scaled up.
const PIXEL_BUDGET: usize = 110_000;
const MAX_STEPS: usize = 72;

fn hsv(h: f32, s: f32, v: f32) -> V3 {
    let k = |o: f32| ((fract(h + o) * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
    v3(
        v * (1.0 + s * (k(0.0) - 1.0)),
        v * (1.0 + s * (k(0.666) - 1.0)),
        v * (1.0 + s * (k(0.333) - 1.0)),
    )
}

fn palette(t: f32) -> V3 {
    let c = |o: f32| 0.55 + 0.45 * (std::f32::consts::TAU * (t + o)).cos();
    v3(c(0.0), c(0.33), c(0.67))
}

/// What the song steers each frame.
#[derive(Clone, Copy)]
struct Scene {
    sc: f32,
    fo: f32,
    hue: f32,
    iris: f32,
    glow: f32,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

/// A fractal look: the camera orbits with the song's time, the fold breathes with the bass, kicks
/// pull the camera in and brighten the surface, and the hue turns once a bar.
pub struct Fractal {
    de: fn(V3, f32, f32) -> f32,
    pixels: Vec<f32>,
}

impl Fractal {
    pub fn new(id: &str) -> Option<Self> {
        FORMULAS.iter().find(|f| f.id == id).map(|f| Self {
            de: f.de,
            pixels: Vec::new(),
        })
    }

    fn scene(f: &Features) -> Scene {
        let time = f.time as f32;
        Scene {
            sc: 2.0 + 0.04 * f.mid,
            fo: 1.0 + 0.08 * f.low - 0.03 * f.high,
            hue: 0.08 + 0.5 * f.bar_phase + 0.02 * f.level,
            iris: 0.55,
            glow: 1.1 + 0.5 * f.onset + 0.2 * f.level,
            yaw: 0.22 * time,
            pitch: 0.35 + 0.12 * (0.17 * time).sin(),
            dist: 2.4 - 0.25 * f.onset - 0.1 * f.low,
        }
    }

    /// Marches one pixel: the colour of the fractal where the ray from the camera hits it.
    fn shade(&self, s: &Scene, st: (f32, f32), px: f32) -> V3 {
        let map = |p: V3| {
            let d = (self.de)(p, s.sc, s.fo);
            if d < 1e4 { d } else { p.length() - 1.05 }
        };
        let target = V3::splat(0.0);
        let dist = s.dist.max(0.6);
        let ro = target
            + v3(
                s.pitch.cos() * s.yaw.sin(),
                s.pitch.sin(),
                s.pitch.cos() * s.yaw.cos(),
            ) * dist;
        let ww = (target - ro).normalize();
        let uu = ww.cross(v3(0.0, 1.0, 0.0)).normalize();
        let vv = uu.cross(ww);
        let rd = (uu * st.0 + vv * st.1 + ww * 1.35).normalize();

        let far = dist * 4.5 + 6.0;
        let (mut t, mut steps, mut hit) = (0.02_f32, 0.0_f32, false);
        for _ in 0..MAX_STEPS {
            let h = map(ro + rd * t);
            let eps = (px * t).max(0.0005);
            if h < eps {
                hit = true;
                break;
            }
            if t > far {
                break;
            }
            t += h.clamp(eps, 1.2);
            steps += 1.0;
        }
        let trap = steps / MAX_STEPS as f32;

        let sky = {
            let k = (st.1 * 0.35 + 0.45).clamp(0.0, 1.0);
            let a = v3(0.07, 0.16, 0.13);
            let b = v3(0.16, 0.05, 0.12);
            a + (b - a) * k + palette(s.hue) * 0.05
        };
        if !hit {
            return sky;
        }
        let p = ro + rd * t;
        let e = (px * t).max(0.0015);
        let (k1, k2, k3, k4) = (
            v3(1.0, -1.0, -1.0),
            v3(-1.0, -1.0, 1.0),
            v3(-1.0, 1.0, -1.0),
            v3(1.0, 1.0, 1.0),
        );
        let n = (k1 * map(p + k1 * e)
            + k2 * map(p + k2 * e)
            + k3 * map(p + k3 * e)
            + k4 * map(p + k4 * e))
        .normalize();
        let light = v3(0.45, 0.85, 0.35).normalize();
        let diff = n.dot(light).clamp(0.0, 1.0);
        let amb = 0.38;
        let reflected = rd - n * (2.0 * n.dot(rd));
        let spec = reflected.dot(light).max(0.0).powi(16);
        let rim = n.dot(rd * -1.0).max(0.0).powi(2);
        let h2 = fract(trap * 2.2 + s.hue);
        let base = hsv(h2, 0.78, s.glow * (amb + diff * 0.85));
        let iris = palette(trap * 2.8) * (s.glow * (amb + diff));
        let iris_mix = s.iris.clamp(0.0, 1.0);
        let mut col = base * (1.0 - iris_mix) + iris * iris_mix;
        col = col + palette(h2 + 0.5) * (spec * 0.55) + palette(s.hue) * (rim * 0.12);
        col
    }
}

impl Look for Fractal {
    fn draw(&mut self, canvas: &mut Canvas, f: &Features) {
        let (cw, ch) = (canvas.width as f32, canvas.height as f32);
        let scale = (PIXEL_BUDGET as f32 / (cw * ch)).min(1.0).sqrt();
        let (w, h) = (
            ((cw * scale) as usize).max(8),
            ((ch * scale) as usize).max(8),
        );
        let scene = Self::scene(f);
        let px = 1.5 / h as f32;
        let aspect = w as f32 / h as f32;
        self.pixels.resize(w * h * 3, 0.0);

        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 16);
        let rows = h.div_ceil(threads).max(1);
        let mut pixels = std::mem::take(&mut self.pixels);
        let this = &*self;
        std::thread::scope(|scope| {
            for (part, band) in pixels.chunks_mut(rows * w * 3).enumerate() {
                scope.spawn(move || {
                    for (r, row) in band.chunks_mut(w * 3).enumerate() {
                        let y = part * rows + r;
                        for x in 0..w {
                            // Screen y points up, like the original's GL coordinates.
                            let st = (
                                ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * aspect,
                                1.0 - (y as f32 + 0.5) / h as f32 * 2.0,
                            );
                            let c = this.shade(&scene, st, px);
                            row[x * 3] = c.x.max(0.0).powf(0.92);
                            row[x * 3 + 1] = c.y.max(0.0).powf(0.92);
                            row[x * 3 + 2] = c.z.max(0.0).powf(0.92);
                        }
                    }
                });
            }
        });
        canvas.set_image(&pixels, w, h);
        self.pixels = pixels;
        if f.new_bar {
            canvas.glow(
                cw / 2.0,
                ch / 2.0,
                canvas.short_side() * 0.6,
                Rgb(1.0, 1.0, 1.0),
                0.12,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_formula_draws_the_fractal_not_just_sky() {
        for formula in &FORMULAS {
            let mut look = Fractal::new(formula.id).unwrap();
            let mut canvas = Canvas::new(96, 54);
            look.draw(&mut canvas, &Features::default());
            let mut rgba = Vec::new();
            canvas.to_rgba(&mut rgba);
            let lo = rgba.chunks(4).map(|p| p[1]).min().unwrap();
            let hi = rgba.chunks(4).map(|p| p[1]).max().unwrap();
            assert!(
                hi > 60 && hi - lo > 40,
                "{} looks empty ({lo}..{hi})",
                formula.id
            );
        }
        assert!(Fractal::new("nope").is_none());
    }
}
