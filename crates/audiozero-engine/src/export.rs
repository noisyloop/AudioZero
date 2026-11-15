//! Exporting a song as an audio file: WAV, or MP3 through FFmpeg.
//!
//! The song is rendered offline, a block at a time, and written as it goes, so a long song
//! doesn't have to fit in memory. Output goes to a temporary file next to the target and is
//! renamed into place only when it's complete, so a cancelled or failed export never leaves a
//! half-written file (and never damages one that was already there).

use crate::engine::Engine;
use crate::project::Project;
use crate::sample::SamplePool;
use crate::wav::WavWriter;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 16-bit WAV: plays everywhere, CD quality.
    Wav16,
    /// 24-bit WAV: more headroom for mastering.
    Wav24,
    /// MP3 at this bit rate in kilobits per second (needs FFmpeg).
    Mp3(u32),
}

impl Format {
    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Wav16 | Format::Wav24 => "wav",
            Format::Mp3(_) => "mp3",
        }
    }
}

/// Which part of the song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// From the start to the end of the last clip, plus a tail.
    Song,
    /// Exactly the loop range, once.
    Loop,
}

/// Sample rates offered for export.
pub const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 96_000];
/// MP3 bit rates offered for export, in kilobits per second.
pub const MP3_RATES: [u32; 4] = [128, 192, 256, 320];
/// The longest export, in seconds.
pub const MAX_SECONDS: f64 = 2.0 * 3600.0;
/// Frames rendered at a time.
const BLOCK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub format: Format,
    pub sample_rate: u32,
    pub range: Range,
    /// Seconds of silence-or-ring-out added after the last clip, for reverb and delay tails.
    /// Only used for [`Range::Song`].
    pub tail: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Wav16,
            sample_rate: 48_000,
            range: Range::Song,
            tail: 2.0,
        }
    }
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub path: PathBuf,
    pub seconds: f64,
    pub bytes: u64,
}

/// Whether FFmpeg is installed, for MP3.
pub fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// How many frames `settings` makes of `project`, and where the playhead starts (in beats).
/// `None` if there's nothing to export.
pub fn plan(project: &Project, settings: &Settings) -> Option<(usize, f64)> {
    let rate = f64::from(settings.sample_rate);
    let seconds_per_beat = 60.0 / project.bpm;
    let (start, seconds) = match settings.range {
        Range::Song => {
            let end = project.end_beats();
            if end <= 0.0 {
                return None;
            }
            (0.0, end * seconds_per_beat + settings.tail.clamp(0.0, 30.0))
        }
        Range::Loop => (
            project.loop_start,
            (project.loop_end - project.loop_start) * seconds_per_beat,
        ),
    };
    let frames = (seconds * rate).ceil();
    (frames >= 1.0).then_some((frames as usize, start))
}

/// Renders the song and writes it to `path`. `progress(done, total)` is told how many frames
/// are done, and returns `false` to cancel. Fails if `path` already exists unless `replace`.
pub fn export(
    project: &Project,
    pool: &mut SamplePool,
    settings: &Settings,
    path: &Path,
    replace: bool,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Done, String> {
    if !SAMPLE_RATES.contains(&settings.sample_rate) {
        return Err(format!(
            "the sample rate has to be one of {SAMPLE_RATES:?}, got {}",
            settings.sample_rate
        ));
    }
    if let Format::Mp3(kbps) = settings.format {
        if !MP3_RATES.contains(&kbps) {
            return Err(format!("the MP3 bit rate has to be one of {MP3_RATES:?}"));
        }
        if !has_ffmpeg() {
            return Err("MP3 export needs FFmpeg (install it, or export a WAV)".into());
        }
    }
    if path.is_dir() {
        return Err(format!("{} is a folder", path.display()));
    }
    if path.exists() && !replace {
        return Err(format!("{} already exists", path.display()));
    }
    let (frames, start) =
        plan(project, settings).ok_or("there's nothing to export: the song has no clips")?;
    let seconds = frames as f64 / f64::from(settings.sample_rate);
    if seconds > MAX_SECONDS {
        return Err(format!(
            "that's {:.0} minutes of audio; the limit is {:.0}",
            seconds / 60.0,
            MAX_SECONDS / 60.0
        ));
    }

    let rate = f64::from(settings.sample_rate);
    let mut engine = Engine::with_samples(project.clone(), rate, BLOCK, pool)
        .map_err(|e| format!("can't render the song: {e}"))?;
    // Play it through once: no looping, from the start of the range.
    engine.set_loop(false, project.loop_start, project.loop_end);
    engine.set_position(start);
    engine.play();

    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)
            .map_err(|e| format!("can't create {}: {e}", folder.display()))?;
    }
    let part = part_path(path);
    let result = write(&mut engine, settings, &part, frames, &mut progress);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&part, path) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("can't save {}: {e}", path.display()));
    }
    let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
    Ok(Done {
        path: path.to_path_buf(),
        seconds,
        bytes,
    })
}

/// The temporary file an export is written to before it's renamed into place.
fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// The cancellation message, so callers can tell it from a failure.
pub const CANCELLED: &str = "cancelled";

fn write(
    engine: &mut Engine,
    settings: &Settings,
    part: &Path,
    frames: usize,
    progress: &mut impl FnMut(usize, usize) -> bool,
) -> Result<(), String> {
    let bits = if settings.format == Format::Wav24 {
        24
    } else {
        16
    };
    match settings.format {
        Format::Wav16 | Format::Wav24 => {
            let file = File::create(part).map_err(|e| format!("{}: {e}", part.display()))?;
            let mut wav = WavWriter::new(BufWriter::new(file), settings.sample_rate, bits, frames)
                .map_err(|e| e.to_string())?;
            render(engine, frames, progress, |l, r| wav.write(l, r))?;
            wav.finish().map_err(|e| e.to_string())
        }
        Format::Mp3(kbps) => {
            // FFmpeg reads a WAV from its standard input and writes the MP3 itself. 24-bit
            // in, so the encoder gets everything we have.
            let mut child = Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "wav",
                    "-i",
                    "-",
                ])
                .args([
                    "-codec:a",
                    "libmp3lame",
                    "-b:a",
                    &format!("{kbps}k"),
                    "-f",
                    "mp3",
                ])
                .arg(part)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("can't start FFmpeg: {e}"))?;
            let stdin = child.stdin.take().ok_or("can't talk to FFmpeg")?;
            let mut wav = WavWriter::new(BufWriter::new(stdin), settings.sample_rate, 24, frames)
                .map_err(|e| e.to_string())?;
            let rendered = render(engine, frames, progress, |l, r| wav.write(l, r));
            // Finishing drops the writer, which closes FFmpeg's input: that's how it knows
            // the audio is complete. If we stopped early, FFmpeg is stopped too.
            let finished = rendered.and_then(|()| wav.finish().map_err(|e| e.to_string()));
            if finished.is_err() {
                let _ = child.kill();
            }
            let output = child
                .wait_with_output()
                .map_err(|e| format!("FFmpeg: {e}"))?;
            finished?;
            if output.status.success() {
                Ok(())
            } else {
                let message = String::from_utf8_lossy(&output.stderr);
                Err(format!("FFmpeg failed: {}", message.trim()))
            }
        }
    }
}

/// Renders `frames` frames a block at a time, handing each block to `sink`.
fn render(
    engine: &mut Engine,
    frames: usize,
    progress: &mut impl FnMut(usize, usize) -> bool,
    mut sink: impl FnMut(&[f32], &[f32]) -> std::io::Result<()>,
) -> Result<(), String> {
    let (mut left, mut right) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut done = 0;
    while done < frames {
        let n = BLOCK.min(frames - done);
        engine.process(&mut left[..n], &mut right[..n]);
        sink(&left[..n], &right[..n]).map_err(|e| e.to_string())?;
        done += n;
        if !progress(done, frames) {
            return Err(CANCELLED.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::sample::decode_file;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("audiozero-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pool() -> SamplePool {
        SamplePool::new(48_000.0)
    }

    #[test]
    fn plans_the_song_or_the_loop() {
        let mut project = demo::project();
        let rate = 48_000.0;
        let seconds = project.end_beats() * 60.0 / project.bpm;
        let (frames, start) = plan(&project, &Settings::default()).unwrap();
        assert_eq!(start, 0.0);
        assert!(
            (frames as f64 - (seconds + 2.0) * rate).abs() <= 1.0,
            "song plus a 2 s tail"
        );

        project.loop_start = 4.0;
        project.loop_end = 8.0;
        let settings = Settings {
            range: Range::Loop,
            ..Default::default()
        };
        let (frames, start) = plan(&project, &settings).unwrap();
        assert_eq!(start, 4.0);
        assert_eq!(
            frames,
            (4.0 * 60.0 / project.bpm * rate).ceil() as usize,
            "exactly the loop"
        );

        assert!(
            plan(&Project::default(), &Settings::default()).is_none(),
            "no clips"
        );
    }

    #[test]
    fn exports_a_wav_that_matches_the_engine() {
        let dir = temp_dir("wav");
        let project = demo::project();
        let path = dir.join("song.wav");
        let mut calls = 0;
        let done = export(
            &project,
            &mut pool(),
            &Settings::default(),
            &path,
            false,
            |d, t| {
                assert!(d <= t);
                calls += 1;
                true
            },
        )
        .unwrap();
        assert!(calls > 10, "progress is reported");
        assert_eq!(done.path, path);
        assert_eq!(done.bytes, std::fs::metadata(&path).unwrap().len());
        assert!(!part_path(&path).exists(), "no temporary file left");

        let back = decode_file(&path).unwrap();
        let (frames, _) = plan(&project, &Settings::default()).unwrap();
        assert_eq!(back.frames(), frames);
        // The same audio as rendering directly.
        let mut engine = Engine::new(project, 48_000.0, 512).unwrap();
        engine.play();
        let (left, _) = engine.render(48_000);
        let mut worst = 0.0f32;
        for (a, b) in back.left().iter().zip(&left) {
            worst = worst.max((a - b).abs());
        }
        assert!(worst < 1.0 / 16_000.0, "16-bit rounding only, got {worst}");
        assert!(left.iter().any(|x| x.abs() > 0.05), "the song isn't silent");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_loop_exports_the_looped_section() {
        let dir = temp_dir("loop");
        let mut project = demo::project();
        project.loop_enabled = true;
        project.loop_start = 8.0;
        project.loop_end = 12.0;
        let settings = Settings {
            range: Range::Loop,
            format: Format::Wav24,
            sample_rate: 44_100,
            ..Default::default()
        };
        let path = dir.join("loop.wav");
        let done = export(&project, &mut pool(), &settings, &path, false, |_, _| true).unwrap();
        let expected = 4.0 * 60.0 / project.bpm;
        assert!((done.seconds - expected).abs() < 0.001);
        let back = decode_file(&path).unwrap();
        assert_eq!(back.frames(), (expected * 44_100.0).ceil() as usize);
        assert_eq!(back.rate(), 44_100.0);
        assert!(back.left().iter().any(|x| x.abs() > 0.01));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_bad_requests_without_touching_files() {
        let dir = temp_dir("refuse");
        let project = demo::project();
        let path = dir.join("song.wav");
        std::fs::write(&path, b"keep me").unwrap();
        let run = |settings: &Settings, path: &Path, replace| {
            export(&project, &mut pool(), settings, path, replace, |_, _| true)
        };
        assert!(
            run(&Settings::default(), &path, false)
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
        assert!(
            run(
                &Settings {
                    sample_rate: 12_345,
                    ..Default::default()
                },
                &dir.join("a.wav"),
                false
            )
            .is_err()
        );
        assert!(
            run(
                &Settings {
                    format: Format::Mp3(77),
                    ..Default::default()
                },
                &dir.join("a.mp3"),
                false
            )
            .is_err()
        );
        assert!(
            run(&Settings::default(), &dir, true)
                .unwrap_err()
                .contains("folder")
        );
        let empty = export(
            &Project::default(),
            &mut pool(),
            &Settings::default(),
            &dir.join("e.wav"),
            false,
            |_, _| true,
        );
        assert!(empty.unwrap_err().contains("nothing to export"));

        // Replacing works, and a cancelled export leaves the old file alone.
        let cancelled = export(
            &project,
            &mut pool(),
            &Settings::default(),
            &path,
            true,
            |_, _| false,
        );
        assert_eq!(cancelled.unwrap_err(), CANCELLED);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"keep me",
            "the old file survives a cancel"
        );
        assert!(!part_path(&path).exists());
        run(&Settings::default(), &path, true).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 1000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_an_mp3_when_ffmpeg_is_installed() {
        if !has_ffmpeg() {
            eprintln!("FFmpeg isn't installed; skipping");
            return;
        }
        let dir = temp_dir("mp3");
        let project = demo::project();
        let settings = Settings {
            format: Format::Mp3(192),
            tail: 0.5,
            ..Default::default()
        };
        let path = dir.join("song.mp3");
        let done = export(&project, &mut pool(), &settings, &path, false, |_, _| true);
        let done = match done {
            Ok(done) => done,
            // Some FFmpeg builds have no MP3 encoder; that's reported, not a crash.
            Err(e) if e.contains("FFmpeg failed") => {
                eprintln!("this FFmpeg can't encode MP3: {e}");
                assert!(!path.exists() && !part_path(&path).exists());
                return;
            }
            Err(e) => panic!("{e}"),
        };
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(done.bytes, bytes.len() as u64);
        let wav_bytes = (done.seconds * 48_000.0 * 4.0) as u64;
        assert!(done.bytes * 4 < wav_bytes, "much smaller than the WAV");
        assert!(
            bytes.starts_with(b"ID3") || (bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0),
            "an MP3 header"
        );
        let back = decode_file(&path).unwrap();
        assert!(
            (back.seconds() - done.seconds).abs() < 0.2,
            "about the right length"
        );
        assert!(back.left().iter().any(|x| x.abs() > 0.05));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
