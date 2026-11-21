//! Exporting a song as an audio file: WAV, or MP3 through FFmpeg.
//!
//! The song is rendered offline, a block at a time, and written as it goes, so a long song
//! doesn't have to fit in memory. Output goes to a temporary file next to the target and is
//! renamed into place only when it's complete, so a cancelled or failed export never leaves a
//! half-written file (and never damages one that was already there).

use crate::engine::Engine;
use crate::metadata::Auto;
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
    /// Write the project's song info (artist, ISRC, ...) into the file. Nothing is written
    /// when the project has none.
    pub embed_metadata: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Wav16,
            sample_rate: 48_000,
            range: Range::Song,
            tail: 2.0,
            embed_metadata: true,
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

    // Song info is checked before any rendering, so a typo in the ISRC is found in a second,
    // not after the export.
    let mut info = project.metadata.clone();
    info.sanitize();
    info.validate()?;
    let tags = (settings.embed_metadata && !info.is_empty()).then(|| {
        let software = format!("AudioZero {}", env!("CARGO_PKG_VERSION"));
        (info, Auto::for_project(project, &software))
    });

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
    let result = write(
        &mut engine,
        settings,
        &part,
        frames,
        tags.as_ref().map(|(m, a)| (m, a)),
        &mut progress,
    );
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
    tags: Option<(&crate::metadata::Metadata, &Auto)>,
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
            let chunks = tags
                .map(|(m, a)| crate::tags::wav_chunks(m, a, settings.sample_rate, bits))
                .unwrap_or_default();
            let mut wav = WavWriter::with_chunks(
                BufWriter::new(file),
                settings.sample_rate,
                bits,
                frames,
                &chunks.before_data,
                chunks.after_data,
            )
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
                // With song info, our own ID3 tag goes on the front instead of FFmpeg's.
                .args(if tags.is_some() {
                    &["-write_id3v2", "0"][..]
                } else {
                    &[][..]
                })
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
                match tags {
                    Some((m, a)) => prepend_tag(part, &crate::tags::id3v24(m, a)),
                    None => Ok(()),
                }
            } else {
                let message = String::from_utf8_lossy(&output.stderr);
                Err(format!("FFmpeg failed: {}", message.trim()))
            }
        }
    }
}

/// Puts `tag` (an ID3 tag) in front of the audio in `file`, by writing a new file and moving it
/// over the old one.
fn prepend_tag(file: &Path, tag: &[u8]) -> Result<(), String> {
    if tag.is_empty() {
        return Ok(());
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".tagged");
    let tagged = file.with_file_name(name);
    let copy = || -> std::io::Result<()> {
        let mut out = BufWriter::new(File::create(&tagged)?);
        std::io::Write::write_all(&mut out, tag)?;
        std::io::copy(&mut File::open(file)?, &mut out)?;
        std::io::Write::flush(&mut out)?;
        std::fs::rename(&tagged, file)
    };
    copy().map_err(|e| {
        let _ = std::fs::remove_file(&tagged);
        format!("can't write the song info: {e}")
    })
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

    fn song_info() -> crate::metadata::Metadata {
        crate::metadata::Metadata {
            title: "Night Drive".into(),
            artist: "Jane Doe".into(),
            isrc: "US-RC1-26-00001".into(),
            producer: "Sam Producer".into(),
            ..Default::default()
        }
    }

    /// The chunk ids of a RIFF/WAVE file, in order.
    fn chunk_ids(bytes: &[u8]) -> Vec<String> {
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8,
            "the RIFF size covers every chunk"
        );
        let (mut i, mut ids) = (12, Vec::new());
        while i < bytes.len() {
            let n = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
            ids.push(String::from_utf8_lossy(&bytes[i..i + 4]).into_owned());
            i += 8 + n + n % 2;
        }
        assert_eq!(i, bytes.len());
        ids
    }

    #[test]
    fn a_wav_carries_the_song_info_and_still_plays() {
        let dir = temp_dir("meta-wav");
        let mut project = demo::project();
        project.metadata = song_info();
        let plain = dir.join("plain.wav");
        let tagged = dir.join("tagged.wav");
        let loop_only = Settings {
            range: Range::Loop,
            ..Default::default()
        };
        let mut no_info = demo::project();
        no_info.metadata = Default::default();
        export(&no_info, &mut pool(), &loop_only, &plain, false, |_, _| {
            true
        })
        .unwrap();
        export(&project, &mut pool(), &loop_only, &tagged, false, |_, _| {
            true
        })
        .unwrap();

        assert_eq!(
            chunk_ids(&std::fs::read(&plain).unwrap()),
            ["fmt ", "data"],
            "no info, no extras"
        );
        let bytes = std::fs::read(&tagged).unwrap();
        assert_eq!(chunk_ids(&bytes), ["fmt ", "bext", "data", "LIST", "id3 "]);
        // The audio is the same with or without the info.
        let (a, b) = (decode_file(&plain).unwrap(), decode_file(&tagged).unwrap());
        assert_eq!(a.frames(), b.frames());
        assert_eq!(a.left(), b.left());
        // And leaving it out is a setting.
        let off = dir.join("off.wav");
        let settings = Settings {
            embed_metadata: false,
            ..loop_only
        };
        export(&project, &mut pool(), &settings, &off, false, |_, _| true).unwrap();
        assert_eq!(chunk_ids(&std::fs::read(&off).unwrap()), ["fmt ", "data"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bad_identifier_stops_the_export_before_it_starts() {
        let dir = temp_dir("meta-bad");
        let mut project = demo::project();
        project.metadata.isrc = "not-an-isrc".into();
        let out = dir.join("x.wav");
        let mut rendered = false;
        let result = export(
            &project,
            &mut pool(),
            &Settings::default(),
            &out,
            false,
            |_, _| {
                rendered = true;
                true
            },
        );
        assert!(result.unwrap_err().contains("ISRC"));
        assert!(!rendered && !out.exists() && !part_path(&out).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_mp3_starts_with_the_id3_tag_when_ffmpeg_is_installed() {
        if !has_ffmpeg() {
            return;
        }
        let dir = temp_dir("meta-mp3");
        let mut project = demo::project();
        project.metadata = song_info();
        let out = dir.join("t.mp3");
        let settings = Settings {
            format: Format::Mp3(128),
            range: Range::Loop,
            ..Default::default()
        };
        export(&project, &mut pool(), &settings, &out, false, |_, _| true).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], b"ID3\x04", "our ID3v2.4 tag is first");
        let size = bytes[6..10]
            .iter()
            .fold(0usize, |n, b| (n << 7) | usize::from(*b));
        let tag = &bytes[10..10 + size];
        assert!(
            tag.windows(4).any(|w| w == b"TSRC") && tag.windows(12).any(|w| w == b"USRC12600001")
        );
        assert_eq!(
            bytes[..10 + size]
                .windows(3)
                .filter(|w| *w == b"ID3")
                .count(),
            1,
            "just one tag"
        );
        assert!(bytes.len() > 10 + size + 1000, "the audio follows the tag");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
