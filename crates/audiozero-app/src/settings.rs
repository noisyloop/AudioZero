//! The audio settings: which sound card to play through and record from, and at what sample
//! rate and buffer size. Kept in a small JSON file in the user's configuration folder, so the
//! window, `--play` and the MCP server all use the same device.
//!
//! Nothing here touches a device, so it can be tested without one.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Sample rates offered in the settings, when the device supports them.
pub const COMMON_RATES: [u32; 8] = [
    22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
];
/// Buffer sizes offered in the settings, in frames, when the device supports them.
pub const COMMON_BUFFERS: [u32; 8] = [32, 64, 128, 256, 512, 1024, 2048, 4096];

const MIN_RATE: u32 = 8_000;
const MAX_RATE: u32 = 384_000;
const MIN_BUFFER: u32 = 16;
const MAX_BUFFER: u32 = 8192;
/// Longest device id kept from a settings file.
const MAX_ID_LENGTH: usize = 512;
/// Largest settings file that's read.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// What the user chose. `None` means "let the system or the device decide".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// The output device's id, as [`crate::audio::DeviceInfo::id`] gives it.
    pub output: Option<String>,
    /// The input device's id.
    pub input: Option<String>,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Buffer size in frames. Smaller means less delay and more risk of crackles.
    pub buffer: Option<u32>,
}

impl AudioSettings {
    /// Drops anything out of range, as if it had never been chosen.
    pub fn sanitized(mut self) -> Self {
        let keep = |id: &String| !id.trim().is_empty() && id.len() <= MAX_ID_LENGTH;
        self.output = self.output.filter(keep);
        self.input = self.input.filter(keep);
        self.sample_rate = self
            .sample_rate
            .filter(|r| (MIN_RATE..=MAX_RATE).contains(r));
        self.buffer = self
            .buffer
            .filter(|b| (MIN_BUFFER..=MAX_BUFFER).contains(b));
        self
    }

    /// Reads settings from JSON. Anything that isn't valid settings gives the defaults.
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str::<Self>(text)
            .unwrap_or_default()
            .sanitized()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings are plain data")
    }

    /// The saved settings, or the defaults if there are none (or they can't be read).
    pub fn load() -> Self {
        let Some(path) = file() else {
            return Self::default();
        };
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_BYTES => {}
            _ => return Self::default(),
        }
        std::fs::read_to_string(&path)
            .map_or_else(|_| Self::default(), |text| Self::from_json(&text))
    }

    /// Saves the settings for next time.
    pub fn save(&self) -> Result<(), String> {
        let path = file().ok_or("there's no home folder to keep settings in")?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        std::fs::write(&path, self.to_json()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Where the settings are kept: `%APPDATA%\AudioZero` on Windows,
/// `~/Library/Application Support/AudioZero` on macOS, and `$XDG_CONFIG_HOME/audiozero`
/// (normally `~/.config/audiozero`) elsewhere.
pub fn file() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let folder = if cfg!(windows) {
        var("APPDATA")?.join("AudioZero")
    } else if cfg!(target_os = "macos") {
        var("HOME")?
            .join("Library")
            .join("Application Support")
            .join("AudioZero")
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".config")))?
            .join("audiozero")
    };
    Some(folder.join("settings.json"))
}

/// A range of sample rates a device supports, in Hz.
pub type RateRange = (u32, u32);

/// The common sample rates inside any of `ranges`, lowest first.
pub fn offered_rates(ranges: &[RateRange]) -> Vec<u32> {
    COMMON_RATES
        .into_iter()
        .filter(|r| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(r)))
        .collect()
}

/// The common buffer sizes inside `range`, smallest first.
pub fn offered_buffers(range: Option<(u32, u32)>) -> Vec<u32> {
    COMMON_BUFFERS
        .into_iter()
        .filter(|b| range.is_none_or(|(lo, hi)| (lo..=hi).contains(b)))
        .collect()
}

/// The supported rate closest to `wanted`: `wanted` itself if a range holds it, else the
/// nearest end of a range. `None` if there are no ranges.
pub fn nearest_rate(ranges: &[RateRange], wanted: u32) -> Option<u32> {
    ranges
        .iter()
        .map(|&(lo, hi)| wanted.clamp(lo.min(hi), hi.max(lo)))
        .min_by_key(|r| r.abs_diff(wanted))
}

/// How long `frames` last at `rate`, in milliseconds.
pub fn milliseconds(frames: u32, rate: u32) -> f64 {
    f64::from(frames) * 1000.0 / f64::from(rate.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_json() {
        let settings = AudioSettings {
            output: Some("alsa:hw:CARD=Scarlett,DEV=0".into()),
            input: None,
            sample_rate: Some(96_000),
            buffer: Some(128),
        };
        assert_eq!(AudioSettings::from_json(&settings.to_json()), settings);
    }

    #[test]
    fn bad_settings_files_give_the_defaults() {
        for text in ["", "not json", "[1, 2]", r#"{"sample_rate": "fast"}"#] {
            assert_eq!(AudioSettings::from_json(text), AudioSettings::default());
        }
        // Unknown keys and missing ones are fine.
        let partial = AudioSettings::from_json(r#"{"buffer": 256, "theme": "dark"}"#);
        assert_eq!(partial.buffer, Some(256));
        assert_eq!(partial.output, None);
    }

    #[test]
    fn out_of_range_values_are_dropped() {
        let long = "x".repeat(MAX_ID_LENGTH + 1);
        let text = format!(
            r#"{{"output": "{long}", "input": "  ", "sample_rate": 12, "buffer": 1000000}}"#
        );
        assert_eq!(AudioSettings::from_json(&text), AudioSettings::default());
    }

    #[test]
    fn offers_only_what_the_device_supports() {
        assert_eq!(
            offered_rates(&[(44_100, 48_000), (96_000, 96_000)]),
            vec![44_100, 48_000, 96_000]
        );
        assert!(offered_rates(&[]).is_empty());
        assert_eq!(offered_buffers(Some((100, 600))), vec![128, 256, 512]);
        assert_eq!(offered_buffers(None), COMMON_BUFFERS.to_vec());
    }

    #[test]
    fn picks_the_nearest_supported_rate() {
        let ranges = [(44_100, 48_000), (96_000, 96_000)];
        assert_eq!(nearest_rate(&ranges, 48_000), Some(48_000));
        assert_eq!(nearest_rate(&ranges, 88_200), Some(96_000));
        assert_eq!(nearest_rate(&ranges, 22_050), Some(44_100));
        assert_eq!(nearest_rate(&[], 48_000), None);
    }

    #[test]
    fn buffer_lengths_in_milliseconds() {
        assert!((milliseconds(480, 48_000) - 10.0).abs() < 1e-9);
        assert!((milliseconds(256, 44_100) - 5.805).abs() < 1e-3);
    }
}
