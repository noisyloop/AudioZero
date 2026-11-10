//! The built-in looks. Each keeps its own state between frames (trails, particles) and is
//! deterministic: the same song gives the same frames.

use crate::analysis::{Features, SPECTRUM_BANDS};
use crate::canvas::{Canvas, Rgb};

/// A look: draws one frame from the features.
pub trait Look: Send {
    fn draw(&mut self, canvas: &mut Canvas, f: &Features);
}

/// Built-in looks: id and display name.
pub const LOOKS: [(&str, &str); 3] = [
    ("spectrum", "Spectrum"),
    ("rings", "Track rings"),
    ("particles", "Note particles"),
];

pub fn create_look(id: &str) -> Option<Box<dyn Look>> {
    match id {
        "spectrum" => Some(Box::new(Spectrum)),
        "rings" => Some(Box::new(Rings::default())),
        "particles" => Some(Box::new(Particles::default())),
        _ => None,
    }
}

/// A colour for a track, spread around the hue circle.
fn track_hue(track: usize) -> f32 {
    (track as f32 * 0.618_034).fract()
}

/// Mirrored spectrum bars across the middle, over a bass glow; colours drift with the bar.
pub struct Spectrum;

impl Look for Spectrum {
    fn draw(&mut self, c: &mut Canvas, f: &Features) {
        c.fade(0.6);
        let (w, h) = (c.width as f32, c.height as f32);
        let base_hue = 0.62 + 0.1 * f.bar_phase;
        c.glow(
            w / 2.0,
            h / 2.0,
            c.short_side() * (0.35 + 0.35 * f.low),
            Rgb::hsv(base_hue, 0.8, 1.0),
            0.12 + 0.5 * f.onset,
        );
        let bars = SPECTRUM_BANDS * 2;
        let slot = w * 0.9 / bars as f32;
        let left = w * 0.05;
        for i in 0..bars {
            // Low bands in the middle, high ones at the edges.
            let band = if i < SPECTRUM_BANDS {
                SPECTRUM_BANDS - 1 - i
            } else {
                i - SPECTRUM_BANDS
            };
            let v = f.spectrum[band];
            let height = h * 0.42 * v * v;
            let x = left + i as f32 * slot;
            let colour = Rgb::hsv(base_hue + band as f32 / 64.0, 0.75, 1.0);
            c.rect(
                x + slot * 0.15,
                h / 2.0 - height,
                x + slot * 0.85,
                h / 2.0 + height,
                colour,
                0.55,
            );
        }
        if f.new_bar {
            c.rect(
                0.0,
                h / 2.0 - 1.0,
                w,
                h / 2.0 + 1.0,
                Rgb(1.0, 1.0, 1.0),
                0.6,
            );
        }
    }
}

/// One ring per track, pulsing with that track's notes; the hue follows the pitch.
#[derive(Default)]
pub struct Rings {
    activity: Vec<f32>,
    hue: Vec<f32>,
}

impl Look for Rings {
    fn draw(&mut self, c: &mut Canvas, f: &Features) {
        c.fade(0.78);
        let tracks = f.tracks.clamp(1, 16);
        self.activity.resize(tracks, 0.0);
        self.hue.resize(tracks, 0.0);
        for (i, h) in self.hue.iter_mut().enumerate() {
            if *h == 0.0 {
                *h = track_hue(i);
            }
        }
        for a in &mut self.activity {
            *a *= 0.86;
        }
        for hit in &f.hits {
            if let Some(a) = self.activity.get_mut(hit.track) {
                *a = (*a + 0.6 * hit.velocity + 0.2).min(1.5);
                self.hue[hit.track] = track_hue(hit.track) * 0.5 + f32::from(hit.pitch % 12) / 24.0;
            }
        }
        let (cx, cy) = (c.width as f32 / 2.0, c.height as f32 / 2.0);
        let max_r = c.short_side() * 0.46;
        if f.onset > 0.05 {
            c.glow(cx, cy, max_r * 1.4, Rgb(1.0, 0.95, 0.9), 0.25 * f.onset);
        }
        for i in 0..tracks {
            let base = max_r * (i + 1) as f32 / tracks as f32;
            let a = self.activity[i];
            let r = base * (0.82 + 0.14 * a) + 0.02 * max_r * f.low;
            let colour = Rgb::hsv(self.hue[i], 0.7, 1.0);
            let width = 2.0 + c.short_side() * 0.012 * (0.3 + a);
            c.ring(cx, cy, r, width, colour, 0.25 + 0.75 * a.min(1.0));
        }
        // The beat as a dot running around the outer ring.
        let angle = f.beat_phase * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let r = max_r * 1.04;
        c.disc(
            cx + r * angle.cos(),
            cy + r * angle.sin(),
            c.short_side() * 0.008,
            Rgb(1.0, 1.0, 1.0),
            0.9,
        );
    }
}

#[derive(Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    size: f32,
    colour: Rgb,
}

/// Bursts of particles for every note: across by pitch, up and down by track.
#[derive(Default)]
pub struct Particles {
    particles: Vec<Particle>,
    seed: u64,
}

const MAX_PARTICLES: usize = 6000;

impl Particles {
    /// A deterministic random number in 0..1 (xorshift).
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
}

impl Look for Particles {
    fn draw(&mut self, c: &mut Canvas, f: &Features) {
        if self.seed == 0 {
            self.seed = 0x9e37_79b9_7f4a_7c15;
        }
        c.fade(0.82);
        let (w, h) = (c.width as f32, c.height as f32);
        let unit = c.short_side() / 1080.0;
        let lanes = f.tracks.max(1) as f32;
        for hit in &f.hits {
            let x = w * (0.08 + 0.84 * (f32::from(hit.pitch.saturating_sub(24)).min(84.0) / 84.0));
            let y = h * (0.15 + 0.7 * (hit.track as f32 + 0.5) / lanes);
            let count = 6 + (hit.velocity * 18.0) as usize;
            let colour = Rgb::hsv(
                track_hue(hit.track) + f32::from(hit.pitch % 12) / 48.0,
                0.65,
                1.0,
            );
            for _ in 0..count {
                if self.particles.len() >= MAX_PARTICLES {
                    break;
                }
                let angle = self.random() * std::f32::consts::TAU;
                let speed = (2.0 + 9.0 * self.random()) * unit * (0.6 + hit.velocity);
                let size = (2.0 + 6.0 * self.random()) * unit;
                self.particles.push(Particle {
                    x,
                    y,
                    vx: angle.cos() * speed,
                    vy: angle.sin() * speed,
                    life: 1.0,
                    size,
                    colour,
                });
            }
        }
        let gravity = 0.15 * unit;
        let push = 1.0 + 0.06 * f.onset;
        for p in &mut self.particles {
            p.vx *= 0.96 * push;
            p.vy = p.vy * 0.96 * push + gravity;
            p.x += p.vx;
            p.y += p.vy;
            p.life -= 0.022;
        }
        self.particles.retain(|p| p.life > 0.0);
        for p in &self.particles {
            c.disc(
                p.x,
                p.y,
                p.size * (0.4 + 0.6 * p.life),
                p.colour,
                0.8 * p.life,
            );
        }
        if f.onset > 0.05 {
            c.glow(
                w / 2.0,
                h,
                c.short_side() * 0.6,
                Rgb(0.9, 0.5, 1.0),
                0.2 * f.onset,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Hit;

    fn frame(hits: Vec<Hit>, onset: f32) -> Features {
        Features {
            spectrum: [0.6; SPECTRUM_BANDS],
            hits,
            onset,
            low: 0.5,
            tracks: 3,
            ..Default::default()
        }
    }

    #[test]
    fn every_look_draws_something_and_is_deterministic() {
        for (id, _) in LOOKS {
            let render = || {
                let mut look = create_look(id).unwrap();
                let mut canvas = Canvas::new(160, 90);
                let hit = Hit {
                    track: 1,
                    pitch: 60,
                    velocity: 1.0,
                };
                for i in 0..5 {
                    let hits = if i == 0 { vec![hit] } else { vec![] };
                    look.draw(&mut canvas, &frame(hits, 0.5));
                }
                let mut out = Vec::new();
                canvas.to_rgba(&mut out);
                out
            };
            let a = render();
            assert!(
                a.chunks(4).any(|p| p[0] > 20 || p[1] > 20 || p[2] > 20),
                "{id} drew nothing"
            );
            assert_eq!(a, render(), "{id} isn't deterministic");
        }
        assert!(create_look("nope").is_none());
    }

    #[test]
    fn particles_are_capped() {
        let mut look = Particles::default();
        let mut canvas = Canvas::new(32, 32);
        let hits = vec![
            Hit {
                track: 0,
                pitch: 40,
                velocity: 1.0
            };
            2000
        ];
        look.draw(&mut canvas, &frame(hits, 0.0));
        assert!(look.particles.len() <= MAX_PARTICLES);
    }
}
