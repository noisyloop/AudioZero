//! Connects the engine's [`Player`] to the default audio output.
//!
//! If there's no output device, a background thread drives the player in real time instead,
//! silently, so the playhead and the rest of the app still work.

use audiozero_engine::control::{Controller, Player, channel};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Largest block the engine renders at once.
const MAX_BLOCK: usize = 512;

pub struct Output {
    pub controller: Controller,
    /// Shown in the status bar.
    pub description: String,
    _keep_alive: KeepAlive,
}

/// Held only so the stream or the silent clock stops when the [`Output`] is dropped.
#[allow(dead_code)]
enum KeepAlive {
    Stream(cpal::Stream),
    Silent(SilentClock),
}

impl Output {
    /// Opens the default output device, or falls back to a silent clock.
    pub fn open() -> Self {
        match open_device() {
            Ok((controller, stream, description)) => Self {
                controller,
                description,
                _keep_alive: KeepAlive::Stream(stream),
            },
            Err(reason) => {
                let (controller, player) = channel(48_000.0, MAX_BLOCK);
                Self {
                    controller,
                    description: format!("No audio output ({reason}); playing silently"),
                    _keep_alive: KeepAlive::Silent(SilentClock::start(player, 48_000.0)),
                }
            }
        }
    }

    pub fn has_device(&self) -> bool {
        matches!(self._keep_alive, KeepAlive::Stream(_))
    }
}

fn open_device() -> Result<(Controller, cpal::Stream, String), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no output device found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (controller, player) = channel(f64::from(config.sample_rate), MAX_BLOCK);
    let description = format!(
        "{} · {} Hz · {} ch",
        device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "Audio output".into()),
        config.sample_rate,
        config.channels
    );

    use cpal::SampleFormat as F;
    let stream = match format {
        F::F32 => build::<f32>(&device, config, player),
        F::I16 => build::<i16>(&device, config, player),
        F::I32 => build::<i32>(&device, config, player),
        F::U16 => build::<u16>(&device, config, player),
        other => return Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((controller, stream, description))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut player: Player,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    // Allocated once here, never in the callback.
    const CHUNK: usize = 4096;
    let (mut left, mut right) = (vec![0.0f32; CHUNK], vec![0.0f32; CHUNK]);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frames in data.chunks_mut(CHUNK * channels) {
                    let n = frames.len() / channels;
                    player.process(&mut left[..n], &mut right[..n]);
                    for (i, frame) in frames.chunks_mut(channels).enumerate() {
                        if channels == 1 {
                            frame[0] = T::from_sample(0.5 * (left[i] + right[i]));
                            continue;
                        }
                        for (ch, sample) in frame.iter_mut().enumerate() {
                            let value = match ch {
                                0 => left[i],
                                1 => right[i],
                                _ => 0.0,
                            };
                            *sample = T::from_sample(value);
                        }
                    }
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Drives a player in real time without a device, rendering into a discarded buffer.
struct SilentClock {
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SilentClock {
    fn start(mut player: Player, sample_rate: f64) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&running);
        let thread = std::thread::Builder::new()
            .name("audiozero-silent-clock".into())
            .spawn(move || {
                let (mut left, mut right) = (vec![0.0f32; MAX_BLOCK], vec![0.0f32; MAX_BLOCK]);
                let block = Duration::from_secs_f64(MAX_BLOCK as f64 / sample_rate);
                let mut next = Instant::now();
                while flag.load(Ordering::Relaxed) {
                    player.process(&mut left, &mut right);
                    next += block;
                    match next.checked_duration_since(Instant::now()) {
                        Some(wait) => std::thread::sleep(wait),
                        None => next = Instant::now(), // fell behind; don't try to catch up
                    }
                }
            })
            .ok();
        Self { running, thread }
    }
}

impl Drop for SilentClock {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
