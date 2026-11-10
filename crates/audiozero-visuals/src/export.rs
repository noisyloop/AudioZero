//! Rendering a look over a song to a video file (through FFmpeg) or to PNG frames.

use crate::analysis::{Analyzer, RATE, Range, render_song};
use crate::canvas::Canvas;
use crate::looks::create_look;
use audiozero_engine::Project;
use audiozero_engine::sample::SamplePool;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A frame size: id, name, width, height.
pub const FORMATS: [(&str, &str, u32, u32); 5] = [
    ("widescreen", "Widescreen 16:9 (1920×1080)", 1920, 1080),
    (
        "shorts",
        "Shorts / Reels / TikTok 9:16 (1080×1920)",
        1080,
        1920,
    ),
    ("square", "Square 1:1 (1080×1080)", 1080, 1080),
    ("portrait", "Portrait 4:5 (1080×1350)", 1080, 1350),
    ("4k", "4K 16:9 (3840×2160)", 3840, 2160),
];

/// Video containers written through FFmpeg, by file extension.
pub const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "mov", "mkv", "webm"];

/// What to render.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoSettings {
    pub look: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub range: Range,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            look: "spectrum".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            range: Range::Song,
        }
    }
}

impl VideoSettings {
    /// Sets the size from a format id ("shorts") or "WIDTHxHEIGHT".
    pub fn set_format(&mut self, format: &str) -> Result<(), String> {
        if let Some((_, _, w, h)) = FORMATS.iter().find(|(id, ..)| *id == format) {
            (self.width, self.height) = (*w, *h);
            return Ok(());
        }
        let parsed = format
            .split_once(['x', 'X'])
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
        let Some((w, h)) = parsed else {
            let ids: Vec<_> = FORMATS.iter().map(|(id, ..)| *id).collect();
            return Err(format!(
                "unknown format \"{format}\"; use {} or WIDTHxHEIGHT",
                ids.join(", ")
            ));
        };
        (self.width, self.height) = (w, h);
        self.check()
    }

    fn check(&self) -> Result<(), String> {
        if !(16..=7680).contains(&self.width) || !(16..=7680).contains(&self.height) {
            return Err("width and height have to be 16 to 7680".into());
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            return Err("width and height have to be even (video encoders need that)".into());
        }
        if !(1..=120).contains(&self.fps) {
            return Err("fps has to be 1 to 120".into());
        }
        if create_look(&self.look).is_none() {
            let ids: Vec<_> = crate::looks::LOOKS.iter().map(|(id, _)| *id).collect();
            return Err(format!(
                "unknown look \"{}\"; use one of {}",
                self.look,
                ids.join(", ")
            ));
        }
        Ok(())
    }
}

/// What an export produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    pub frames: usize,
    pub seconds: f64,
    pub path: PathBuf,
}

/// The error an export returns when its progress callback cancels it.
pub const CANCELLED: &str = "export cancelled";

/// Whether FFmpeg is available for video files.
pub fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Renders `settings.look` over `project` to `out`: a video file if it ends in .mp4, .mov, .mkv
/// or .webm (needs FFmpeg), otherwise a folder of PNG frames plus `audio.wav`. `progress` gets
/// (frame, frames) as it goes and returns `false` to cancel, which removes a partial video.
pub fn export(
    project: &Project,
    pool: &mut SamplePool,
    settings: &VideoSettings,
    out: &Path,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Exported, String> {
    settings.check()?;
    let video = is_video(out);
    if video && !has_ffmpeg() {
        return Err(
            "exporting video needs FFmpeg (install it, or export to a folder for PNG frames)"
                .into(),
        );
    }
    let song = render_song(project, pool, settings.range)?;
    let mut analyzer = Analyzer::new(&song, settings.fps);
    let frames = analyzer.frames();
    let mut look = create_look(&settings.look).expect("checked");
    let (w, h) = (settings.width as usize, settings.height as usize);
    let mut canvas = Canvas::new(w, h);
    let mut rgba = Vec::new();

    let wav_for = |dir: &Path| dir.join("audio.wav");
    let write_wav = |path: &Path| -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        audiozero_engine::wav::write_stereo_16(
            std::io::BufWriter::new(file),
            RATE as u32,
            &song.left,
            &song.right,
        )
        .map_err(|e| e.to_string())
    };

    if !video {
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
        write_wav(&wav_for(out))?;
        for i in 0..frames {
            look.draw(&mut canvas, &analyzer.next_frame());
            canvas.to_rgba(&mut rgba);
            write_png(&out.join(format!("frame_{:06}.png", i + 1)), w, h, &rgba)?;
            if !progress(i + 1, frames) {
                return Err(CANCELLED.into());
            }
        }
        return Ok(Exported {
            frames,
            seconds: song.seconds(),
            path: out.to_path_buf(),
        });
    }

    // The audio goes in a temporary file next to the output; FFmpeg muxes it in.
    let audio = out.with_extension("audiozero-audio.wav");
    write_wav(&audio)?;
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let codec: &[&str] = if ext == "webm" {
        &[
            "-c:v",
            "libvpx-vp9",
            "-crf",
            "32",
            "-b:v",
            "0",
            "-c:a",
            "libopus",
            "-b:a",
            "160k",
        ]
    } else {
        &[
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ]
    };
    let size = format!("{w}x{h}");
    let fps = settings.fps.to_string();
    let mut child = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
        ])
        .args(["-s", &size, "-r", &fps, "-i", "-", "-i"])
        .arg(&audio)
        .args(["-map", "0:v", "-map", "1:a", "-shortest"])
        .args(codec)
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't start FFmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let mut write_error = None;
    let mut cancelled = false;
    for i in 0..frames {
        look.draw(&mut canvas, &analyzer.next_frame());
        canvas.to_rgba(&mut rgba);
        if let Err(e) = stdin.write_all(&rgba) {
            write_error = Some(e);
            break;
        }
        if !progress(i + 1, frames) {
            cancelled = true;
            break;
        }
    }
    drop(stdin);
    if cancelled {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&audio);
        let _ = std::fs::remove_file(out);
        return Err(CANCELLED.into());
    }
    let output = child.wait_with_output().map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&audio);
    let output = output?;
    if !output.status.success() || write_error.is_some() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFmpeg failed: {}", message.trim()));
    }
    Ok(Exported {
        frames,
        seconds: song.seconds(),
        path: out.to_path_buf(),
    })
}

/// Renders a single frame at `seconds` to a PNG, for previews.
pub fn still(
    project: &Project,
    pool: &mut SamplePool,
    settings: &VideoSettings,
    seconds: f64,
    out: &Path,
) -> Result<(), String> {
    settings.check()?;
    let song = render_song(project, pool, settings.range)?;
    let mut analyzer = Analyzer::new(&song, settings.fps);
    let target = ((seconds.max(0.0) * f64::from(settings.fps)) as usize)
        .min(analyzer.frames().saturating_sub(1));
    let mut look = create_look(&settings.look).expect("checked");
    let (w, h) = (settings.width as usize, settings.height as usize);
    let mut canvas = Canvas::new(w, h);
    // Run the look up to the frame, so trails and particles look as they would in the video.
    for _ in 0..=target {
        look.draw(&mut canvas, &analyzer.next_frame());
    }
    let mut rgba = Vec::new();
    canvas.to_rgba(&mut rgba);
    write_png(out, w, h, &rgba)
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use audiozero_engine::demo;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("audiozero-video-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn short_settings(look: &str) -> VideoSettings {
        let mut project_settings = VideoSettings {
            look: look.into(),
            fps: 10,
            range: Range::Beats(0.0, 4.0),
            ..Default::default()
        };
        project_settings.set_format("160x90").unwrap();
        project_settings
    }

    #[test]
    fn formats_parse_and_bad_settings_are_refused() {
        let mut s = VideoSettings::default();
        s.set_format("shorts").unwrap();
        assert_eq!((s.width, s.height), (1080, 1920));
        s.set_format("1280x720").unwrap();
        assert_eq!((s.width, s.height), (1280, 720));
        assert!(s.set_format("1281x720").is_err(), "odd sizes");
        assert!(s.set_format("cinema").is_err());
        assert!(s.set_format("99999x10").is_err());
        let bad = VideoSettings {
            look: "nope".into(),
            ..Default::default()
        };
        assert!(bad.check().is_err());
    }

    #[test]
    fn exports_png_frames_and_audio() {
        let dir = temp("frames");
        let out = dir.join("frames");
        let mut pool = SamplePool::new(RATE);
        let mut calls = 0;
        let result = export(
            &demo::project(),
            &mut pool,
            &short_settings("rings"),
            &out,
            |_, _| {
                calls += 1;
                true
            },
        )
        .unwrap();
        // 4 beats at 110 BPM is about 2.18 s, so 22 frames at 10 fps.
        assert_eq!(result.frames, 22);
        assert_eq!(calls, 22);
        assert!(out.join("frame_000001.png").exists() && out.join("frame_000022.png").exists());
        assert!(out.join("audio.wav").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_mp4_when_ffmpeg_is_installed() {
        if !has_ffmpeg() {
            eprintln!("FFmpeg isn't installed; skipping");
            return;
        }
        let dir = temp("mp4");
        let out = dir.join("clip.mp4");
        let mut pool = SamplePool::new(RATE);
        let result = export(
            &demo::project(),
            &mut pool,
            &short_settings("particles"),
            &out,
            |_, _| true,
        )
        .unwrap();
        assert_eq!(result.frames, 22);
        let size = std::fs::metadata(&out).unwrap().len();
        assert!(size > 1000, "a real file ({size} bytes)");
        assert!(
            !out.with_extension("audiozero-audio.wav").exists(),
            "the temporary audio is gone"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stills_render_one_frame() {
        let dir = temp("still");
        let out = dir.join("still.png");
        let mut pool = SamplePool::new(RATE);
        still(
            &demo::project(),
            &mut pool,
            &short_settings("spectrum"),
            1.0,
            &out,
        )
        .unwrap();
        assert!(std::fs::metadata(&out).unwrap().len() > 100);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancelling_stops_and_cleans_up() {
        if !has_ffmpeg() {
            return;
        }
        let dir = temp("cancel");
        let out = dir.join("clip.mp4");
        let mut pool = SamplePool::new(RATE);
        let result = export(
            &demo::project(),
            &mut pool,
            &short_settings("spectrum"),
            &out,
            |frame, _| frame < 5,
        );
        assert_eq!(result, Err(CANCELLED.to_string()));
        assert!(!out.exists(), "the partial video is removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
