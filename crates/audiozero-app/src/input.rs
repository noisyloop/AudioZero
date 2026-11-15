//! Live input: MIDI keyboards (through midir) and the audio input chosen in the audio settings
//! (for recording).
//!
//! MIDI notes go from the MIDI thread straight to the audio thread through the engine's
//! [`LiveInput`] queue, so playing doesn't wait for the UI. A copy, stamped with the playhead,
//! goes to the UI for recording.

use crate::audio::{Direction, choose_config, find_device, input_watch};
use crate::settings::AudioSettings;
use audiozero_engine::{LiveInput, LiveNote};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};

/// No track is armed for live notes.
const NO_TRACK: usize = usize::MAX;

/// A note played on a MIDI keyboard, for the recorder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiEvent {
    /// The playhead when it arrived, in beats.
    pub beat: f64,
    pub playing: bool,
    pub track: usize,
    pub pitch: u8,
    pub velocity: f32,
    pub on: bool,
}

/// Turns one MIDI message into (pitch, velocity, on). Only note on and off matter for now.
pub fn parse_note(message: &[u8]) -> Option<(u8, f32, bool)> {
    let (&status, rest) = message.split_first()?;
    let (&pitch, rest) = rest.split_first()?;
    let &velocity = rest.first()?;
    let pitch = pitch & 0x7f;
    let velocity = velocity & 0x7f;
    match status & 0xf0 {
        0x90 if velocity > 0 => Some((pitch, f32::from(velocity) / 127.0, true)),
        0x80 | 0x90 => Some((pitch, 0.0, false)),
        _ => None,
    }
}

/// Every MIDI input, connected at once (as the C++ app does).
pub struct MidiIn {
    _connections: Vec<midir::MidiInputConnection<()>>,
    /// The names of the connected inputs.
    pub ports: Vec<String>,
    events: mpsc::Receiver<MidiEvent>,
    target: Arc<AtomicUsize>,
}

impl MidiIn {
    /// Connects to every MIDI input. `wake` is called when a note arrives, to redraw the UI.
    pub fn open(live: LiveInput, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let probe = midir::MidiInput::new("AudioZero").map_err(|e| e.to_string())?;
        let ports = probe.ports();
        let live = Arc::new(Mutex::new(live));
        let wake = Arc::new(wake);
        let target = Arc::new(AtomicUsize::new(NO_TRACK));
        let (tx, events) = mpsc::channel();
        let mut connections = Vec::new();
        let mut names = Vec::new();
        for port in &ports {
            let name = probe
                .port_name(port)
                .unwrap_or_else(|_| "MIDI input".into());
            // Each connection needs its own MidiInput.
            let Ok(input) = midir::MidiInput::new("AudioZero") else {
                continue;
            };
            let (live, wake, target, tx) = (
                Arc::clone(&live),
                Arc::clone(&wake),
                Arc::clone(&target),
                tx.clone(),
            );
            let callback = move |_: u64, message: &[u8], _: &mut ()| {
                let Some((pitch, velocity, on)) = parse_note(message) else {
                    return;
                };
                let track = target.load(Relaxed);
                if track == NO_TRACK {
                    return;
                }
                let Ok(mut live) = live.lock() else { return };
                live.send(LiveNote {
                    track,
                    pitch,
                    velocity,
                    on,
                });
                let _ = tx.send(MidiEvent {
                    beat: live.position(),
                    playing: live.is_playing(),
                    track,
                    pitch,
                    velocity,
                    on,
                });
                drop(live);
                wake();
            };
            if let Ok(connection) = input.connect(port, "AudioZero input", callback, ()) {
                connections.push(connection);
                names.push(name);
            }
        }
        Ok(Self {
            _connections: connections,
            ports: names,
            events,
            target,
        })
    }

    /// The track live MIDI plays, or `None` to ignore MIDI input.
    pub fn set_target(&self, track: Option<usize>) {
        self.target.store(track.unwrap_or(NO_TRACK), Relaxed);
    }

    /// Notes that arrived since the last call.
    pub fn events(&self) -> impl Iterator<Item = MidiEvent> + '_ {
        self.events.try_iter()
    }
}

/// The audio input, captured into a lock-free buffer that the UI drains.
pub struct AudioIn {
    _stream: cpal::Stream,
    samples: rtrb::Consumer<f32>,
    pub rate: f64,
    pub channels: usize,
    /// Set when the UI didn't drain fast enough and audio was lost.
    overflowed: Arc<AtomicBool>,
    /// Set when the device went away.
    lost: Arc<AtomicBool>,
    pub description: String,
}

impl AudioIn {
    /// Opens the settings' input device (or the default one), at the settings' sample rate if
    /// the device supports it. Capture starts at once; drain it regularly.
    pub fn open(settings: &AudioSettings) -> Result<Self, String> {
        let (device, note) = find_device(Direction::Input, settings.input.as_deref())?;
        let (config, format, _) = choose_config(&device, Direction::Input, settings)?;
        let (stream, samples, overflowed, lost) = match capture(&device, config, format) {
            Ok(started) => started,
            // A device can refuse a buffer size it claimed to support; try its own choice.
            Err(e) if config.buffer_size != cpal::BufferSize::Default => {
                let fallback = cpal::StreamConfig {
                    buffer_size: cpal::BufferSize::Default,
                    ..config
                };
                capture(&device, fallback, format).map_err(|_| e)?
            }
            Err(e) => return Err(e),
        };
        let mut description = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "Audio input".into());
        description.push_str(&format!(" at {} Hz", config.sample_rate));
        if let Some(note) = note {
            description.push_str(&format!(" ({note})"));
        }
        Ok(Self {
            _stream: stream,
            samples,
            rate: f64::from(config.sample_rate),
            channels: usize::from(config.channels).max(1),
            overflowed,
            lost,
            description,
        })
    }

    /// Whether the input device went away (unplugged, say).
    pub fn is_lost(&self) -> bool {
        self.lost.load(Relaxed)
    }

    /// Moves everything captured so far into `left` and `right` (the first two channels; mono
    /// goes to both). Returns `true` if audio was lost since the last call.
    pub fn drain(&mut self, left: &mut Vec<f32>, right: &mut Vec<f32>) -> bool {
        let available = self.samples.slots();
        let frames = available / self.channels;
        if let Ok(chunk) = self.samples.read_chunk(frames * self.channels) {
            let (a, b) = chunk.as_slices();
            let mut channel = 0;
            for &x in a.iter().chain(b) {
                match channel {
                    0 => {
                        left.push(x);
                        if self.channels == 1 {
                            right.push(x);
                        }
                    }
                    1 => right.push(x),
                    _ => {}
                }
                channel = (channel + 1) % self.channels;
            }
            chunk.commit_all();
        }
        self.overflowed.swap(false, Relaxed)
    }

    /// Throws away what's been captured (before a take starts).
    pub fn discard(&mut self) {
        let n = self.samples.slots();
        if let Ok(chunk) = self.samples.read_chunk(n) {
            chunk.commit_all();
        }
        self.overflowed.store(false, Relaxed);
    }
}

type Capture = (
    cpal::Stream,
    rtrb::Consumer<f32>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
);

/// Builds and starts an input stream: the stream, what it captures, and its overflow and
/// device-lost flags.
fn capture(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    format: cpal::SampleFormat,
) -> Result<Capture, String> {
    let rate = config.sample_rate as usize;
    let channels = usize::from(config.channels).max(1);
    // Four seconds of buffer: plenty, since the UI drains it every frame.
    let (tx, samples) = rtrb::RingBuffer::new((rate * channels * 4).max(4096));
    let overflowed = Arc::new(AtomicBool::new(false));
    let (on_error, lost) = input_watch();
    use cpal::SampleFormat as F;
    let stream = match format {
        F::F32 => build::<f32>(device, config, tx, Arc::clone(&overflowed), on_error),
        F::I16 => build::<i16>(device, config, tx, Arc::clone(&overflowed), on_error),
        F::I32 => build::<i32>(device, config, tx, Arc::clone(&overflowed), on_error),
        F::U16 => build::<u16>(device, config, tx, Arc::clone(&overflowed), on_error),
        other => return Err(format!("unsupported input format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, samples, overflowed, lost))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut tx: rtrb::Producer<f32>,
    overflowed: Arc<AtomicBool>,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // No allocation here: push into the preallocated ring, or note the overflow.
                for &x in data {
                    if tx.push(<f32 as cpal::Sample>::from_sample(x)).is_err() {
                        overflowed.store(true, Relaxed);
                        break;
                    }
                }
            },
            on_error,
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_messages() {
        assert_eq!(parse_note(&[0x90, 60, 127]), Some((60, 1.0, true)));
        assert_eq!(
            parse_note(&[0x93, 61, 0]),
            Some((61, 0.0, false)),
            "on with 0 is off"
        );
        assert_eq!(parse_note(&[0x80, 62, 40]), Some((62, 0.0, false)));
        assert_eq!(
            parse_note(&[0xB0, 64, 127]),
            None,
            "controllers are ignored"
        );
        assert_eq!(parse_note(&[0x90, 60]), None, "short message");
        assert_eq!(
            parse_note(&[0x90, 200, 200]),
            Some((72, 72.0 / 127.0, true))
        );
    }
}
