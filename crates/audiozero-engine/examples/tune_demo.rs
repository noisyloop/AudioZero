//! Writes a made-up, out-of-tune "sung" phrase and two tuned versions of it, to listen to:
//!
//!     cargo run --release -p audiozero-engine --example tune_demo -- some/folder
//!
//! `sung.wav` is the phrase as "sung" (each note up to 45 cents off, with vibrato and a slide),
//! `natural.wav` is it tuned at 80 ms, and `hard.wav` is it hard-tuned (0 ms), all in C major.

use audiozero_engine::music::scale_by_id;
use audiozero_engine::sample::Sample;
use audiozero_engine::tune::{Settings, tune};
use std::f32::consts::TAU;

const RATE: f32 = 48_000.0;

fn main() -> Result<(), String> {
    let folder = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let folder = std::path::Path::new(&folder);
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;

    // (note, how far off in semitones, seconds): "do re mi fa sol", sung carelessly.
    let phrase = [
        (60.0, 0.30, 0.6),
        (62.0, -0.35, 0.6),
        (64.0, 0.25, 0.6),
        (65.0, -0.45, 0.6),
        (67.0, 0.35, 1.2),
    ];
    let mut mono = Vec::new();
    let mut phase = 0.0f32;
    let mut last = 59.0;
    for (note, off, seconds) in phrase {
        let frames = (seconds * RATE) as usize;
        for i in 0..frames {
            let time = i as f32 / RATE;
            // Slide in from the last note, then vibrato that grows through the note.
            let slide = (last - note) * (-time * 25.0).exp();
            let vibrato = 0.3 * (time / seconds).min(1.0) * (TAU * 5.5 * time).sin();
            let hz = 440.0 * ((note + off + slide + vibrato - 69.0) / 12.0).exp2();
            phase = (phase + hz / RATE).fract();
            let mut x = 0.0;
            for h in 1..=30 {
                let f = hz * h as f32;
                if f > 10_000.0 {
                    break;
                }
                let gain = 0.7 / h as f32
                    + 0.6 * (-((f - 700.0) / 150.0).powi(2)).exp()
                    + 0.4 * (-((f - 1200.0) / 200.0).powi(2)).exp()
                    + 0.15 * (-((f - 2600.0) / 300.0).powi(2)).exp();
                x += gain * (TAU * phase * h as f32).sin();
            }
            let fade = (time * 40.0).min(1.0) * ((seconds - time) * 25.0).min(1.0);
            mono.push(0.18 * x * fade);
        }
        last = note + off;
    }
    let sung = Sample::new(mono.clone(), mono, f64::from(RATE));
    let write = |name: &str, sample: &Sample| -> Result<(), String> {
        let path = folder.join(name);
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        audiozero_engine::wav::write_stereo_16(
            std::io::BufWriter::new(file),
            RATE as u32,
            sample.left(),
            sample.right(),
        )
        .map_err(|e| e.to_string())?;
        println!("{}", path.display());
        Ok(())
    };
    write("sung.wav", &sung)?;
    for (name, speed_ms) in [("natural.wav", 80.0), ("hard.wav", 0.0)] {
        let settings = Settings {
            key: 0,
            scale: scale_by_id("major"),
            speed_ms,
            amount: 1.0,
        };
        let tuned = tune(&sung, settings, |_| true).ok_or("stopped")?;
        write(name, &tuned)?;
    }
    Ok(())
}
