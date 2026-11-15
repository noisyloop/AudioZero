//! Connects the engine's [`Player`] to a sound card: lists the audio devices, opens the one the
//! [`AudioSettings`] name (or the system's default) at the chosen sample rate and buffer size,
//! and notices when it's unplugged.
//!
//! If there's no output device, a background thread drives the player in real time instead,
//! silently, so the playhead and the rest of the app still work.

use crate::settings::{AudioSettings, RateRange, milliseconds, nearest_rate};
use audiozero_engine::control::{Controller, Player, channel};
use audiozero_engine::{Changes, Command, Project};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Largest block the engine renders at once.
const MAX_BLOCK: usize = 512;
/// The rate the silent clock runs at.
const SILENT_RATE: f64 = 48_000.0;

/// Playback or recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Output,
    Input,
}

impl Direction {
    fn noun(self) -> &'static str {
        match self {
            Direction::Output => "output",
            Direction::Input => "input",
        }
    }
}

/// An audio device the settings can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Stable across runs; this is what the settings file keeps.
    pub id: String,
    pub name: String,
    /// The one the system uses when nothing is chosen.
    pub is_default: bool,
}

/// What a device can do, for the settings dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub rates: Vec<RateRange>,
    /// Smallest and largest buffer in frames, if the device says.
    pub buffer: Option<(u32, u32)>,
    pub default_rate: Option<u32>,
    pub channels: u16,
}

fn device_name(device: &cpal::Device, direction: Direction) -> String {
    device
        .description()
        .map(|d| d.to_string())
        .unwrap_or_else(|_| format!("Audio {}", direction.noun()))
}

fn default_device(host: &cpal::Host, direction: Direction) -> Option<cpal::Device> {
    match direction {
        Direction::Output => host.default_output_device(),
        Direction::Input => host.default_input_device(),
    }
}

fn all_devices(host: &cpal::Host, direction: Direction) -> Vec<cpal::Device> {
    match direction {
        Direction::Output => host.output_devices().map(Iterator::collect),
        Direction::Input => host.input_devices().map(Iterator::collect),
    }
    .unwrap_or_default()
}

/// The devices that can play (or record), with the system's default first.
pub fn devices(direction: Direction) -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default_id = default_device(&host, direction)
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    let mut list: Vec<DeviceInfo> = all_devices(&host, direction)
        .iter()
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            Some(DeviceInfo {
                name: device_name(device, direction),
                is_default: default_id.as_deref() == Some(&id),
                id,
            })
        })
        .collect();
    list.sort_by_key(|d| !d.is_default);
    list.dedup_by(|a, b| a.id == b.id);
    list
}

/// Finds the device a person meant by `text`: an id, a name, or part of a name (any case).
/// Returns its id.
pub fn resolve(direction: Direction, text: &str) -> Result<String, String> {
    let list = devices(direction);
    pick(&list, text).map(|d| d.id.clone()).ok_or_else(|| {
        format!(
            "no audio {} matches \"{text}\" (see --devices)",
            direction.noun()
        )
    })
}

/// The device `text` names: an exact id first, then an exact name, then part of a name.
fn pick<'a>(list: &'a [DeviceInfo], text: &str) -> Option<&'a DeviceInfo> {
    let wanted = text.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    list.iter()
        .find(|d| d.id == text.trim())
        .or_else(|| list.iter().find(|d| d.name.to_lowercase() == wanted))
        .or_else(|| {
            list.iter()
                .find(|d| d.name.to_lowercase().contains(&wanted))
        })
}

/// The device with `id`, or the default one when `id` is `None`.
fn device_by_id(host: &cpal::Host, direction: Direction, id: Option<&str>) -> Option<cpal::Device> {
    let Some(id) = id else {
        return default_device(host, direction);
    };
    all_devices(host, direction)
        .into_iter()
        .find(|d| d.id().is_ok_and(|found| found.to_string() == id))
}

/// The device the settings name. If it's gone (unplugged, say), the default one, with a note
/// saying so.
pub(crate) fn find_device(
    direction: Direction,
    wanted: Option<&str>,
) -> Result<(cpal::Device, Option<String>), String> {
    let host = cpal::default_host();
    if let Some(device) = device_by_id(&host, direction, wanted) {
        return Ok((device, None));
    }
    let noun = direction.noun();
    let fallback =
        default_device(&host, direction).ok_or_else(|| format!("no audio {noun} found"))?;
    let note =
        wanted.map(|_| format!("The chosen audio {noun} isn't connected; using the default"));
    Ok((fallback, note))
}

fn usable(format: cpal::SampleFormat) -> bool {
    use cpal::SampleFormat as F;
    matches!(format, F::F32 | F::I16 | F::I32 | F::U16)
}

fn config_ranges(
    device: &cpal::Device,
    direction: Direction,
) -> Vec<cpal::SupportedStreamConfigRange> {
    match direction {
        Direction::Output => device.supported_output_configs().map(Iterator::collect),
        Direction::Input => device.supported_input_configs().map(Iterator::collect),
    }
    .unwrap_or_default()
}

fn default_config(
    device: &cpal::Device,
    direction: Direction,
) -> Result<cpal::SupportedStreamConfig, String> {
    let default = match direction {
        Direction::Output => device.default_output_config(),
        Direction::Input => device.default_input_config(),
    };
    match default {
        Ok(config) if usable(config.sample_format()) => Ok(config),
        other => {
            // No default, or one in a format we don't convert: take the best of the rest.
            let mut ranges: Vec<_> = config_ranges(device, direction)
                .into_iter()
                .filter(|r| usable(r.sample_format()))
                .collect();
            ranges.sort_by(|a, b| b.cmp_default_heuristics(a));
            let best = ranges.into_iter().next().ok_or_else(|| match other {
                Err(e) => e.to_string(),
                Ok(config) => format!("unsupported sample format {}", config.sample_format()),
            })?;
            let rate = 48_000.clamp(best.min_sample_rate(), best.max_sample_rate());
            Ok(best.with_sample_rate(rate))
        }
    }
}

fn buffer_range(size: &cpal::SupportedBufferSize) -> Option<(u32, u32)> {
    match *size {
        cpal::SupportedBufferSize::Range { min, max } => Some((min, max)),
        cpal::SupportedBufferSize::Unknown => None,
    }
}

/// What the device with `id` (or the default one) can do. `None` if it can't be asked.
pub fn capabilities(direction: Direction, id: Option<&str>) -> Option<Capabilities> {
    let host = cpal::default_host();
    let device = device_by_id(&host, direction, id)?;
    let default = default_config(&device, direction).ok();
    let ranges: Vec<_> = config_ranges(&device, direction)
        .into_iter()
        .filter(|r| usable(r.sample_format()))
        .collect();
    if ranges.is_empty() && default.is_none() {
        return None;
    }
    let mut rates: Vec<RateRange> = ranges
        .iter()
        .map(|r| (r.min_sample_rate(), r.max_sample_rate()))
        .collect();
    rates.extend(default.iter().map(|d| (d.sample_rate(), d.sample_rate())));
    rates.sort_unstable();
    rates.dedup();
    let buffers = ranges
        .iter()
        .map(|r| r.buffer_size())
        .chain(default.iter().map(|d| d.buffer_size()))
        .filter_map(buffer_range);
    let buffer = buffers.reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
    Some(Capabilities {
        rates,
        buffer,
        default_rate: default.as_ref().map(|d| d.sample_rate()),
        channels: default.as_ref().map_or_else(
            || ranges.iter().map(|r| r.channels()).max().unwrap_or(0),
            |d| d.channels(),
        ),
    })
}

/// The stream configuration for `device`: its default, moved to the settings' sample rate and
/// buffer size where the device supports them. The notes say what it couldn't do.
pub(crate) fn choose_config(
    device: &cpal::Device,
    direction: Direction,
    settings: &AudioSettings,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat, Vec<String>), String> {
    let mut chosen = default_config(device, direction)?;
    let mut notes = Vec::new();
    if let Some(rate) = settings.sample_rate
        && rate != chosen.sample_rate()
    {
        let mut ranges: Vec<_> = config_ranges(device, direction)
            .into_iter()
            .filter(|r| usable(r.sample_format()))
            .collect();
        // Closest to the default's channel count first, then float samples.
        let channels = chosen.channels();
        ranges.sort_by_key(|r| {
            (
                r.channels().abs_diff(channels),
                r.sample_format() != cpal::SampleFormat::F32,
            )
        });
        let exact = ranges.iter().find_map(|r| r.try_with_sample_rate(rate));
        let all: Vec<RateRange> = ranges
            .iter()
            .map(|r| (r.min_sample_rate(), r.max_sample_rate()))
            .collect();
        let nearest = nearest_rate(&all, rate)
            .and_then(|near| ranges.iter().find_map(|r| r.try_with_sample_rate(near)));
        match exact.or(nearest) {
            Some(config) => {
                if config.sample_rate() != rate {
                    notes.push(format!(
                        "{rate} Hz isn't supported; using {} Hz",
                        config.sample_rate()
                    ));
                }
                chosen = config;
            }
            None => notes.push(format!(
                "{rate} Hz isn't supported; using {} Hz",
                chosen.sample_rate()
            )),
        }
    }
    let range = buffer_range(chosen.buffer_size());
    let format = chosen.sample_format();
    let mut config = chosen.config();
    if let Some(wanted) = settings.buffer {
        let frames = range.map_or(wanted, |(lo, hi)| wanted.clamp(lo.min(hi), hi.max(lo)));
        if frames != wanted {
            notes.push(format!(
                "a {wanted}-frame buffer isn't supported; using {frames} frames"
            ));
        }
        config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    Ok((config, format, notes))
}

/// What the stream's error callback has seen.
#[derive(Default)]
struct Health {
    lost: AtomicBool,
    xruns: AtomicU32,
}

fn watch(health: Arc<Health>, what: &'static str) -> impl FnMut(cpal::Error) + Send + 'static {
    move |err: cpal::Error| match err.kind() {
        cpal::ErrorKind::Xrun => {
            health.xruns.fetch_add(1, Ordering::Relaxed);
        }
        // The system moved the stream to another device by itself; nothing to do.
        cpal::ErrorKind::DeviceChanged => {}
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
            health.lost.store(true, Ordering::Relaxed);
        }
        _ => eprintln!("audio {what} error: {err}"),
    }
}

/// Watches an input stream the way an output is watched. Returns the error callback and the
/// flag it sets when the device goes away.
pub(crate) fn input_watch() -> (impl FnMut(cpal::Error) + Send + 'static, Arc<AtomicBool>) {
    let lost = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&lost);
    let callback = move |err: cpal::Error| match err.kind() {
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged => {}
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
            flag.store(true, Ordering::Relaxed);
        }
        _ => eprintln!("audio input error: {err}"),
    };
    (callback, lost)
}

pub struct Output {
    pub controller: Controller,
    /// Shown in the status bar.
    pub description: String,
    /// Something the user should know about how it opened (a fallback, say).
    pub note: Option<String>,
    health: Arc<Health>,
    _keep_alive: KeepAlive,
}

/// Held only so the stream or the silent clock stops when the [`Output`] is dropped.
#[allow(dead_code)]
enum KeepAlive {
    Stream(cpal::Stream),
    Silent(SilentClock),
    Closed,
}

struct Opened {
    controller: Controller,
    stream: cpal::Stream,
    description: String,
    notes: Vec<String>,
    health: Arc<Health>,
}

impl Output {
    /// Opens the settings' output device (or the default one), or falls back to a silent clock.
    pub fn open(settings: &AudioSettings) -> Self {
        match open_device(settings) {
            Ok(opened) => Self {
                controller: opened.controller,
                description: opened.description,
                note: (!opened.notes.is_empty()).then(|| opened.notes.join("; ")),
                health: opened.health,
                _keep_alive: KeepAlive::Stream(opened.stream),
            },
            Err(reason) => Self::silent(&reason),
        }
    }

    /// No device: a clock that keeps the playhead moving without sound.
    pub fn silent(reason: &str) -> Self {
        let (controller, player) = channel(SILENT_RATE, MAX_BLOCK);
        Self {
            controller,
            description: format!("No audio output ({reason}); playing silently"),
            note: None,
            health: Arc::default(),
            _keep_alive: KeepAlive::Silent(SilentClock::start(player, SILENT_RATE)),
        }
    }

    /// Stops the stream and lets go of the device, so it can be opened again. Some sound cards
    /// only allow one stream at a time.
    pub fn close(&mut self) {
        self._keep_alive = KeepAlive::Closed;
    }

    pub fn has_device(&self) -> bool {
        matches!(self._keep_alive, KeepAlive::Stream(_))
    }

    /// Whether the device went away (it was unplugged, or the system took it back). Reading it
    /// clears it.
    pub fn take_lost(&self) -> bool {
        self.health.lost.swap(false, Ordering::Relaxed)
    }

    /// How many times the sound card ran out of audio (a crackle) since the stream opened.
    pub fn xruns(&self) -> u32 {
        self.health.xruns.load(Ordering::Relaxed)
    }
}

/// Lets the MCP server play through this output.
impl audiozero_mcp::Audio for Output {
    fn load(&mut self, project: &Project) -> Result<Vec<String>, String> {
        self.controller
            .load(project.clone())
            .map_err(|e| e.to_string())
    }
    fn apply(&mut self, project: &Project, changes: &Changes) -> Result<Vec<String>, String> {
        self.controller.apply(project, changes)
    }
    fn send(&mut self, command: Command) -> bool {
        self.controller.send(command)
    }
    fn position(&self) -> f64 {
        self.controller.position()
    }
    fn is_playing(&self) -> bool {
        self.controller.is_playing()
    }
    fn collect_garbage(&mut self) {
        self.controller.collect_garbage();
    }
}

fn open_device(settings: &AudioSettings) -> Result<Opened, String> {
    let (device, note) = find_device(Direction::Output, settings.output.as_deref())?;
    let (config, format, mut notes) = choose_config(&device, Direction::Output, settings)?;
    notes.splice(0..0, note);
    let name = device_name(&device, Direction::Output);

    // A device can refuse a buffer size it claimed to support; its own choice always works.
    let (stream, controller, health) = match start(&device, config, format) {
        Ok(started) => started,
        Err(e) if config.buffer_size != cpal::BufferSize::Default => {
            let fallback = cpal::StreamConfig {
                buffer_size: cpal::BufferSize::Default,
                ..config
            };
            let started = start(&device, fallback, format).map_err(|_| e.clone())?;
            notes.push(format!("the device refused that buffer size ({e})"));
            started
        }
        Err(e) => return Err(e),
    };
    let buffer = stream.buffer_size().ok().filter(|&frames| frames > 0);
    let mut description = format!(
        "{name} · {} Hz · {} ch",
        config.sample_rate, config.channels
    );
    if let Some(frames) = buffer {
        description.push_str(&format!(
            " · {frames} frames ({:.1} ms)",
            milliseconds(frames, config.sample_rate)
        ));
    }
    Ok(Opened {
        controller,
        stream,
        description,
        notes,
        health,
    })
}

/// Builds and starts a stream on `device`, with a fresh controller and player for it.
fn start(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    format: cpal::SampleFormat,
) -> Result<(cpal::Stream, Controller, Arc<Health>), String> {
    let (controller, player) = channel(f64::from(config.sample_rate), MAX_BLOCK);
    let health = Arc::new(Health::default());
    use cpal::SampleFormat as F;
    let stream = match format {
        F::F32 => build::<f32>(device, config, player, Arc::clone(&health)),
        F::I16 => build::<i16>(device, config, player, Arc::clone(&health)),
        F::I32 => build::<i32>(device, config, player, Arc::clone(&health)),
        F::U16 => build::<u16>(device, config, player, Arc::clone(&health)),
        other => return Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, controller, health))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut player: Player,
    health: Arc<Health>,
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
            watch(health, "output"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            name: name.into(),
            is_default: false,
        }
    }

    #[test]
    fn devices_are_found_by_id_name_or_part_of_a_name() {
        let list = [
            info("alsa:default", "Default Audio Device"),
            info("alsa:hw:CARD=USB,DEV=0", "Scarlett 2i2 USB"),
            info("alsa:hw:CARD=PCH,DEV=0", "HDA Intel PCH"),
            info("scarlett", "Something else"),
        ];
        let id = |text| pick(&list, text).map(|d| d.id.as_str());
        assert_eq!(id("alsa:hw:CARD=PCH,DEV=0"), Some("alsa:hw:CARD=PCH,DEV=0"));
        assert_eq!(id("scarlett"), Some("scarlett"), "an exact id wins");
        assert_eq!(id("hda intel pch"), Some("alsa:hw:CARD=PCH,DEV=0"));
        assert_eq!(id("2i2"), Some("alsa:hw:CARD=USB,DEV=0"));
        assert_eq!(id("Focusrite"), None);
        assert_eq!(id("  "), None);
    }

    #[test]
    fn without_a_device_the_output_is_silent_but_runs() {
        let mut output = Output::silent("testing");
        assert!(!output.has_device() && !output.take_lost());
        assert_eq!(output.xruns(), 0);
        assert!(output.description.contains("playing silently"));
        assert!(output.controller.send(Command::Play));
        let started = Instant::now();
        while !output.controller.is_playing() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            output.controller.is_playing(),
            "the silent clock is running"
        );
        output.close();
        assert!(!output.has_device());
    }
}
