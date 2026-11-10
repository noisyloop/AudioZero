//! Audio files for audio clips: finding them, decoding them with limits, and keeping them
//! decoded at the engine's sample rate.
//!
//! A project names an audio file by a string. [`SamplePool::resolve`] decides whether that string
//! may be opened (security review, findings 3 and 9):
//!
//! - only files with an audio extension, and only regular files under a size cap;
//! - relative paths resolve against the project's folder and can't climb out of it with `..`;
//! - UNC and device paths (`\\server\share`, `//host/x`, `\\?\...`) are refused, so opening a
//!   project can't make AudioZero reach out to a network share.
//!
//! Decoding is bounded too: sample rate, channel count and decoded length all have limits, and
//! the pool as a whole has a cap on how much audio it keeps.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Biggest audio file that will be opened.
pub const MAX_AUDIO_FILE_BYTES: u64 = 1 << 30;
/// Longest audio file that will be decoded (10 minutes).
pub const MAX_SAMPLE_SECONDS: f64 = 600.0;
/// Most audio the pool keeps decoded at once, in seconds (an hour, about 1.4 GB at 48 kHz).
pub const MAX_POOL_SECONDS: f64 = 3600.0;
/// File extensions that can be opened, lower case.
pub const AUDIO_EXTENSIONS: [&str; 7] = ["wav", "wave", "aif", "aiff", "flac", "mp3", "ogg"];
/// Frames per entry in [`Sample::peaks`].
pub const PEAK_FRAMES: usize = 256;

/// Decoded stereo audio. Mono files have the same data on both sides.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    left: Vec<f32>,
    right: Vec<f32>,
    rate: f64,
    /// (min, max) of the mono mix for every [`PEAK_FRAMES`] frames, for drawing.
    peaks: Vec<(f32, f32)>,
}

impl Sample {
    /// Makes a sample from two channels of the same length (the longer one is cut).
    pub fn new(mut left: Vec<f32>, mut right: Vec<f32>, rate: f64) -> Self {
        let frames = left.len().min(right.len());
        left.truncate(frames);
        right.truncate(frames);
        for x in left.iter_mut().chain(right.iter_mut()) {
            if !x.is_finite() {
                *x = 0.0;
            }
        }
        let peaks = left
            .chunks(PEAK_FRAMES)
            .zip(right.chunks(PEAK_FRAMES))
            .map(|(l, r)| {
                l.iter()
                    .zip(r)
                    .map(|(a, b)| 0.5 * (a + b))
                    .fold((0.0f32, 0.0f32), |(lo, hi), x| (lo.min(x), hi.max(x)))
            })
            .collect();
        Self {
            left,
            right,
            rate,
            peaks,
        }
    }

    pub fn frames(&self) -> usize {
        self.left.len()
    }
    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.rate
    }
    pub fn left(&self) -> &[f32] {
        &self.left
    }
    pub fn right(&self) -> &[f32] {
        &self.right
    }
    pub fn peaks(&self) -> &[(f32, f32)] {
        &self.peaks
    }

    /// The same audio at `rate`, by cubic (Catmull-Rom) interpolation.
    pub fn resampled(&self, rate: f64) -> Self {
        if (rate - self.rate).abs() < 1e-6 || self.frames() == 0 {
            return Self::new(self.left.clone(), self.right.clone(), rate);
        }
        let ratio = self.rate / rate;
        let frames = ((self.frames() as f64) / ratio).floor() as usize;
        let resample = |src: &[f32]| -> Vec<f32> {
            let at = |i: isize| src[i.clamp(0, src.len() as isize - 1) as usize];
            (0..frames)
                .map(|n| {
                    let pos = n as f64 * ratio;
                    let i = pos.floor() as isize;
                    let t = (pos - pos.floor()) as f32;
                    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
                    p1 + 0.5
                        * t
                        * (p2 - p0
                            + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
                                + t * (3.0 * (p1 - p2) + p3 - p0)))
                })
                .collect()
        };
        Self::new(resample(&self.left), resample(&self.right), rate)
    }
}

/// Decodes an audio file at its own sample rate. The caller is expected to have checked the
/// path with [`SamplePool::resolve`].
pub fn decode_file(path: &Path) -> Result<Sample, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("not a supported audio file ({e})"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("the file has no audio")?;
    let track_id = track.id;
    let rate = f64::from(
        track
            .codec_params
            .sample_rate
            .ok_or("unknown sample rate")?,
    );
    if !(1_000.0..=384_000.0).contains(&rate) {
        return Err(format!("unsupported sample rate {rate} Hz"));
    }
    let max_frames = (MAX_SAMPLE_SECONDS * rate) as usize;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported audio format ({e})"))?;

    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("can't read the file ({e})")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A damaged packet: skip it, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("can't decode the file ({e})")),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        if channels == 0 || channels > 32 {
            return Err(format!("unsupported channel count {channels}"));
        }
        let needed = decoded.capacity() as u64;
        if buffer
            .as_ref()
            .is_none_or(|b| b.capacity() < needed as usize)
        {
            buffer = Some(SampleBuffer::new(needed, spec));
        }
        let buf = buffer.as_mut().expect("just made");
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks_exact(channels) {
            left.push(frame[0]);
            right.push(frame[if channels > 1 { 1 } else { 0 }]);
        }
        if left.len() > max_frames {
            return Err(format!(
                "the file is longer than {} minutes",
                MAX_SAMPLE_SECONDS / 60.0
            ));
        }
    }
    if left.is_empty() {
        return Err("the file has no audio".into());
    }
    Ok(Sample::new(left, right, rate))
}

/// Decoded audio files, by the name a project gives them, at one sample rate.
#[derive(Debug)]
pub struct SamplePool {
    rate: f64,
    base: Option<PathBuf>,
    cache: HashMap<String, Result<Arc<Sample>, String>>,
}

impl SamplePool {
    pub fn new(rate: f64) -> Self {
        Self {
            rate,
            base: None,
            cache: HashMap::new(),
        }
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Sets the folder relative paths are resolved against: the project's folder, or `None`
    /// for a project that hasn't been saved.
    pub fn set_base(&mut self, base: Option<PathBuf>) {
        if self.base != base {
            self.base = base;
            // Relative names now mean other files; absolute ones still mean the same.
            self.cache.retain(|k, _| Path::new(k).is_absolute());
        }
    }

    pub fn base(&self) -> Option<&Path> {
        self.base.as_deref()
    }

    /// Checks whether `file` (as written in a project) may be opened and returns where it is.
    pub fn resolve(&self, file: &str) -> Result<PathBuf, String> {
        if file.is_empty() || file.len() > 4096 || file.contains('\0') {
            return Err("not a valid file name".into());
        }
        if file.starts_with("\\\\") || file.starts_with("//") {
            return Err("network and device paths aren't allowed".into());
        }
        let path = Path::new(file);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            if path
                .components()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            {
                return Err("relative paths can't leave the project folder".into());
            }
            let base = self
                .base
                .as_ref()
                .ok_or("the project hasn't been saved, so relative paths can't be found")?;
            base.join(path)
        };
        let ext = full
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !ext.is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.as_str())) {
            return Err("not an audio file (use wav, aif, flac, mp3 or ogg)".into());
        }
        let meta = std::fs::metadata(&full).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("not a file".into());
        }
        if meta.len() > MAX_AUDIO_FILE_BYTES {
            return Err(format!(
                "the file is over {} MB",
                MAX_AUDIO_FILE_BYTES / (1 << 20)
            ));
        }
        Ok(full)
    }

    fn cached_seconds(&self) -> f64 {
        self.cache
            .values()
            .filter_map(|s| s.as_ref().ok())
            .map(|s| s.seconds())
            .sum()
    }

    /// The decoded audio for `file`, decoding it the first time. Failures are remembered, so
    /// a broken file isn't decoded again on every edit.
    pub fn get(&mut self, file: &str) -> Result<Arc<Sample>, String> {
        if let Some(cached) = self.cache.get(file) {
            return cached.clone();
        }
        let result = self.load(file);
        self.cache.insert(file.to_owned(), result.clone());
        result
    }

    fn load(&self, file: &str) -> Result<Arc<Sample>, String> {
        let path = self.resolve(file)?;
        let sample = decode_file(&path)?;
        if self.cached_seconds() + sample.seconds() > MAX_POOL_SECONDS {
            return Err(format!(
                "more than {} minutes of audio is loaded",
                MAX_POOL_SECONDS / 60.0
            ));
        }
        Ok(Arc::new(sample.resampled(self.rate)))
    }

    /// Adds audio that didn't come from a file the pool opened, such as a recording.
    pub fn insert(&mut self, file: &str, sample: &Sample) -> Arc<Sample> {
        let sample = Arc::new(sample.resampled(self.rate));
        self.cache.insert(file.to_owned(), Ok(Arc::clone(&sample)));
        sample
    }

    /// Forgets every file not in `used`, including remembered failures.
    pub fn retain(&mut self, used: &[&str]) {
        self.cache.retain(|k, _| used.contains(&k.as_str()));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn write_test_wav(path: &Path, rate: u32, frames: usize) {
        let left: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let right = left.iter().map(|x| -x).collect::<Vec<_>>();
        let file = std::fs::File::create(path).unwrap();
        crate::wav::write_stereo_16(std::io::BufWriter::new(file), rate, &left, &right).unwrap();
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "audiozero-sample-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn decodes_and_resamples_a_wav() {
        let dir = temp("decode");
        let path = dir.join("tone.wav");
        write_test_wav(&path, 44_100, 44_100);
        let sample = decode_file(&path).unwrap();
        assert_eq!((sample.frames(), sample.rate()), (44_100, 44_100.0));
        assert!((sample.left()[20] - (20.0f32 * 0.05).sin() * 0.5).abs() < 1e-3);
        assert!((sample.right()[20] + sample.left()[20]).abs() < 1e-3);
        let up = sample.resampled(48_000.0);
        assert!((up.seconds() - 1.0).abs() < 0.001);
        assert_eq!(sample.peaks().len(), 44_100usize.div_ceil(PEAK_FRAMES));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_refuses_risky_paths() {
        let dir = temp("resolve");
        write_test_wav(&dir.join("ok.wav"), 48_000, 100);
        std::fs::write(dir.join("notes.txt"), "hi").unwrap();
        let mut pool = SamplePool::new(48_000.0);
        assert!(
            pool.resolve("ok.wav").is_err(),
            "no project folder yet, so relative paths can't resolve"
        );
        pool.set_base(Some(dir.clone()));
        assert!(pool.resolve("ok.wav").is_ok());
        assert!(pool.resolve("./ok.wav").is_ok());
        for bad in [
            "../ok.wav",
            "sub/../../ok.wav",
            "\\\\server\\share\\a.wav",
            "//server/share/a.wav",
            "notes.txt",
            "",
            "missing.wav",
        ] {
            assert!(pool.resolve(bad).is_err(), "{bad} should be refused");
        }
        let absolute = dir.join("ok.wav");
        assert!(pool.resolve(absolute.to_str().unwrap()).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn garbage_files_fail_cleanly_and_are_remembered() {
        let dir = temp("garbage");
        std::fs::write(dir.join("junk.wav"), b"RIFF\x10\0\0\0WAVEjunkjunkjunk").unwrap();
        let mut pool = SamplePool::new(48_000.0);
        pool.set_base(Some(dir.clone()));
        assert!(pool.get("junk.wav").is_err());
        assert!(pool.cache.contains_key("junk.wav"));
        pool.retain(&[]);
        assert!(pool.cache.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pool_resamples_to_its_rate() {
        let dir = temp("pool");
        write_test_wav(&dir.join("a.wav"), 24_000, 24_000);
        let mut pool = SamplePool::new(48_000.0);
        pool.set_base(Some(dir.clone()));
        let a = pool.get("a.wav").unwrap();
        assert_eq!(a.rate(), 48_000.0);
        assert!((a.seconds() - 1.0).abs() < 0.001);
        assert!(Arc::ptr_eq(&a, &pool.get("a.wav").unwrap()), "cached");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
