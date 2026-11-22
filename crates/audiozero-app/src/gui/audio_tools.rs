//! Tools that make new audio or notes out of what's already on the timeline, run on a background
//! thread with a progress window and Cancel:
//!
//! - **Bounce to audio track:** plays a MIDI (or group) track offline into a WAV and puts it on a
//!   new audio track just below.
//! - **Reverse:** reverses an audio clip (or every clip of an audio track, flipping the track's
//!   order in time too).
//! - **Convert to MIDI:** finds the melody, the chords or the drums in an audio clip and puts them
//!   on a new MIDI track just below.
//! - **Separate stems:** splits an audio clip into drums, bass, vocals and other, on four new
//!   audio tracks just below.
//!
//! Every result is written to a file next to the project (in `Bounces`, `Edits` or `Stems`),
//! never over an existing one, and the change to the project is one undo step.

use super::App;
use super::theme;
use audiozero_engine::bounce::bounce_track;
use audiozero_engine::export::CANCELLED;
use audiozero_engine::project::{Clip, Note};
use audiozero_engine::sample::{Sample, SamplePool, decode_file};
use audiozero_engine::session::{ClipRef, NewAudio};
use audiozero_engine::stems::separate;
use audiozero_engine::transcribe::{Kind, transcribe};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

/// Sample rate a MIDI track is bounced at.
const BOUNCE_RATE: f64 = 48_000.0;
/// Progress is counted in thousandths.
const FULL: usize = 1000;

/// What to do.
pub(super) enum Tool {
    Bounce { track: usize },
    Reverse { clips: Vec<AudioClip>, mirror: bool },
    ToMidi { clip: AudioClip, kind: Kind },
    Stems { clip: AudioClip },
}

/// An audio clip as it was when the tool started, so a change meanwhile can be noticed.
#[derive(Clone)]
pub(super) struct AudioClip {
    at: ClipRef,
    name: String,
    file: String,
    offset: f64,
    start: f64,
    length: f64,
}

/// A file that's been written, and its audio.
struct Written {
    path: PathBuf,
    sample: Sample,
}

/// What a finished tool hands back to be applied to the project.
enum Done {
    Bounce {
        after: usize,
        name: String,
        start: f64,
        length: f64,
        audio: Written,
    },
    Reverse {
        items: Vec<(AudioClip, Written)>,
        mirror: bool,
    },
    Midi {
        clip: AudioClip,
        kind: Kind,
        notes: Vec<Note>,
    },
    Stems {
        clip: AudioClip,
        stems: Vec<(&'static str, Written)>,
    },
}

pub(super) struct ToolJob {
    title: String,
    progress: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    result: std::sync::mpsc::Receiver<Result<Done, String>>,
}

/// Where a tool's files go: a folder next to the project, or the default folder before it's saved.
fn folder_for(project_file: Option<&Path>, sub: &str) -> PathBuf {
    crate::project_folder(project_file)
        .unwrap_or_else(crate::default_folder)
        .join(sub)
}

/// A file name in `folder` made from `name`: plain characters, and one that doesn't exist.
fn unique_wav(folder: &Path, name: &str) -> PathBuf {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let clean = clean.trim();
    let base = if clean.is_empty() { "Audio" } else { clean };
    std::iter::once(folder.join(format!("{base}.wav")))
        .chain((2..).map(|n| folder.join(format!("{base} {n}.wav"))))
        .find(|p| !p.exists())
        .expect("some number is free")
}

/// Writes `sample` as a 24-bit WAV named after `name` in `folder`.
fn write(folder: &Path, name: &str, sample: Sample) -> Result<Written, String> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("can't create {}: {e}", folder.display()))?;
    let path = unique_wav(folder, name);
    let file = audiozero_engine::safe_fs::create_new(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    audiozero_engine::wav::write_stereo_24(
        std::io::BufWriter::new(file),
        sample.rate().round() as u32,
        sample.left(),
        sample.right(),
    )
    .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    Ok(Written { path, sample })
}

/// Decodes the audio an audio clip plays, at the file's own sample rate.
fn load(base: Option<&Path>, clip: &AudioClip) -> Result<Sample, String> {
    let mut pool = SamplePool::new(BOUNCE_RATE);
    pool.set_base(base.map(Path::to_path_buf));
    let path = pool.resolve(&clip.file)?;
    decode_file(&path)
}

fn clip_seconds(clip: &AudioClip, bpm: f64) -> f64 {
    clip.length * 60.0 / bpm
}

impl App {
    /// The audio clip at `at`, if there is one, as the tools need it.
    pub(super) fn audio_clip(&self, at: ClipRef) -> Option<AudioClip> {
        let clip = self.session.clip(at)?;
        let audio = clip.audio.as_ref()?;
        Some(AudioClip {
            at,
            name: clip.name.clone(),
            file: audio.file.clone(),
            offset: audio.offset,
            start: clip.start,
            length: clip.length,
        })
    }

    /// Every audio clip of a track.
    pub(super) fn audio_clips_of(&self, track: usize) -> Vec<AudioClip> {
        let n = self.session.track(track).map_or(0, |t| t.clips.len());
        (0..n)
            .filter_map(|clip| self.audio_clip(ClipRef { track, clip }))
            .collect()
    }

    /// Whether `track` is an audio track: no instrument, and at least one audio clip.
    pub(super) fn is_audio_track(&self, track: usize) -> bool {
        self.session.track(track).is_some_and(|t| {
            !t.group && t.instrument.is_none() && t.clips.iter().any(Clip::is_audio)
        })
    }

    pub(super) fn start_tool(&mut self, tool: Tool) {
        if self.tool_job.is_some() {
            self.status = "Wait for the current job to finish first.".into();
            return;
        }
        let project = self.session.project().clone();
        let bpm = project.bpm;
        let base = crate::project_folder(self.file.as_deref());
        let file = self.file.clone();
        let (progress, cancel) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (p, c) = (Arc::clone(&progress), Arc::clone(&cancel));
        let (tx, rx) = std::sync::mpsc::channel();
        let (title, work): (String, Box<dyn FnOnce() -> Result<Done, String> + Send>) = match tool {
            Tool::Bounce { track } => {
                let name = project
                    .tracks
                    .get(track)
                    .map_or("Track".into(), |t| t.name.clone());
                (
                    format!("Bouncing \"{name}\" to audio…"),
                    Box::new(move || {
                        let mut pool = SamplePool::new(BOUNCE_RATE);
                        pool.set_base(base);
                        let bounced =
                            bounce_track(&project, &mut pool, track, BOUNCE_RATE, |n, of| {
                                p.store(n * FULL / of.max(1), Relaxed);
                                !c.load(Relaxed)
                            })?;
                        let length = bounced.sample.seconds() * bpm / 60.0;
                        let audio = write(
                            &folder_for(file.as_deref(), "Bounces"),
                            &format!("{name} (audio)"),
                            bounced.sample,
                        )?;
                        Ok(Done::Bounce {
                            after: track,
                            name: format!("{name} (audio)"),
                            start: bounced.start_beat,
                            length,
                            audio,
                        })
                    }),
                )
            }
            Tool::Reverse { clips, mirror } => (
                if clips.len() == 1 {
                    "Reversing the clip…".to_string()
                } else {
                    format!("Reversing {} clips…", clips.len())
                },
                Box::new(move || {
                    let mut items = Vec::new();
                    for (i, clip) in clips.iter().enumerate() {
                        if c.load(Relaxed) {
                            return Err(CANCELLED.into());
                        }
                        p.store(i * FULL / clips.len(), Relaxed);
                        let sample = load(base.as_deref(), clip)?;
                        let part = sample.region(clip.offset, clip_seconds(clip, bpm));
                        if part.frames() == 0 {
                            return Err(format!("\"{}\" has no audio to reverse", clip.name));
                        }
                        let written = write(
                            &folder_for(file.as_deref(), "Edits"),
                            &format!("{} reversed", clip.name),
                            part.reversed(),
                        )?;
                        items.push((clip.clone(), written));
                    }
                    Ok(Done::Reverse { items, mirror })
                }),
            ),
            Tool::ToMidi { clip, kind } => (
                format!(
                    "Finding the {} in \"{}\"…",
                    kind.label().to_lowercase(),
                    clip.name
                ),
                Box::new(move || {
                    let sample = load(base.as_deref(), &clip)?;
                    let notes = transcribe(
                        &sample,
                        clip.offset,
                        clip_seconds(&clip, bpm),
                        bpm,
                        kind,
                        |f| {
                            p.store((f * FULL as f32) as usize, Relaxed);
                            !c.load(Relaxed)
                        },
                    )?;
                    Ok(Done::Midi { clip, kind, notes })
                }),
            ),
            Tool::Stems { clip } => (
                format!("Separating stems of \"{}\"…", clip.name),
                Box::new(move || {
                    let sample = load(base.as_deref(), &clip)?;
                    let part = sample.region(clip.offset, clip_seconds(&clip, bpm));
                    let stems = separate(&part, |f| {
                        p.store((f * 0.9 * FULL as f32) as usize, Relaxed);
                        !c.load(Relaxed)
                    })?;
                    let folder = folder_for(file.as_deref(), "Stems");
                    let mut written = Vec::new();
                    for (i, (name, stem)) in stems.into_iter().enumerate() {
                        p.store(FULL * 9 / 10 + i * FULL / 40, Relaxed);
                        written.push((
                            name,
                            write(&folder, &format!("{} - {name}", clip.name), stem)?,
                        ));
                    }
                    Ok(Done::Stems {
                        clip,
                        stems: written,
                    })
                }),
            ),
        };
        let spawned = std::thread::Builder::new()
            .name("audiozero-audio-tool".into())
            .spawn(move || {
                let _ = tx.send(work());
            });
        match spawned {
            Ok(_) => {
                self.tool_job = Some(ToolJob {
                    title,
                    progress,
                    cancel,
                    result: rx,
                });
            }
            Err(e) => self.status = format!("Can't start that: {e}"),
        }
    }

    /// The progress window of a running tool, and applying its result when it's done.
    pub(super) fn tool_ui(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.tool_job else {
            return;
        };
        match job.result.try_recv() {
            Ok(result) => {
                self.tool_job = None;
                match result {
                    Ok(done) => self.apply_tool(done),
                    Err(e) if e == CANCELLED || e == "cancelled" => {
                        self.status = "Cancelled.".into()
                    }
                    Err(e) => self.status = format!("That didn't work: {e}"),
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.tool_job = None;
                self.status = "That stopped unexpectedly.".into();
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        let (title, fraction, cancel) = (
            job.title.clone(),
            job.progress.load(Relaxed) as f32 / FULL as f32,
            Arc::clone(&job.cancel),
        );
        egui::Modal::new(egui::Id::unique("audio-tool")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(&title);
            ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).show_percentage());
            ui.label(
                egui::RichText::new(
                    "You can keep this window open while it works; Cancel stops it.",
                )
                .color(theme::text_dim()),
            );
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel.store(true, Relaxed);
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }

    /// Registers the new audio with the engine and returns how the project refers to it.
    fn register(&mut self, written: &Written) -> String {
        let file = self.audio_file_name(&written.path);
        self.audio
            .controller
            .samples()
            .insert(&file, &written.sample);
        file
    }

    fn apply_tool(&mut self, done: Done) {
        match done {
            Done::Bounce {
                after,
                name,
                start,
                length,
                audio,
            } => {
                let file = self.register(&audio);
                let result = self.session.add_audio_tracks_below(
                    after,
                    vec![NewAudio {
                        name: name.clone(),
                        file,
                        start,
                        length,
                    }],
                );
                if let Some(track) = self.done(result) {
                    self.selected_track = Some(track);
                    self.selected_clip = Some(ClipRef { track, clip: 0 });
                    self.status = format!(
                        "Bounced to \"{name}\", just below. Mute the original to hear only the bounce. Undo removes it."
                    );
                }
            }
            Done::Reverse { items, mirror } => {
                // The clips have to still be what they were when this started.
                let unchanged = items.iter().all(|(clip, _)| {
                    self.session.clip(clip.at).is_some_and(|now| {
                        now.audio
                            .as_ref()
                            .is_some_and(|a| a.file == clip.file && a.offset == clip.offset)
                            && now.start == clip.start
                    })
                });
                if !unchanged {
                    self.status =
                        "The clips changed while they were being reversed, so nothing was changed."
                            .into();
                    return;
                }
                let count = items.len();
                let edits: Vec<(ClipRef, String)> = items
                    .iter()
                    .map(|(clip, w)| (clip.at, self.register(w)))
                    .collect();
                let result = self.session.reverse_clips(edits, mirror);
                self.finish(result);
                self.status = if count == 1 {
                    "Reversed. Undo brings the original back.".into()
                } else {
                    format!("Reversed {count} clips. Undo brings the originals back.")
                };
            }
            Done::Midi { clip, kind, notes } => {
                if notes.is_empty() {
                    self.status = format!(
                        "No {} found in \"{}\".",
                        kind.label().to_lowercase(),
                        clip.name
                    );
                    return;
                }
                let count = notes.len();
                let instrument = if kind == Kind::Drums {
                    "drums"
                } else {
                    "soundgineer"
                };
                let name = format!("{} - {}", clip.name, kind.label());
                let midi = Clip {
                    name: name.clone(),
                    start: clip.start,
                    length: clip.length,
                    notes,
                    ..Default::default()
                };
                let result =
                    self.session
                        .add_midi_track_below(clip.at.track, &name, instrument, midi);
                if let Some(track) = self.done(result) {
                    self.selected_track = Some(track);
                    self.selected_clip = Some(ClipRef { track, clip: 0 });
                    self.status = format!(
                        "Found {count} notes: they're on a new MIDI track just below. It's approximate, so tidy it in the piano roll. Undo removes it."
                    );
                }
            }
            Done::Stems { clip, stems } => {
                let audio: Vec<NewAudio> = stems
                    .iter()
                    .map(|(name, written)| NewAudio {
                        name: format!("{} - {name}", clip.name),
                        file: self.register(written),
                        start: clip.start,
                        length: clip.length,
                    })
                    .collect();
                let result = self.session.add_audio_tracks_below(clip.at.track, audio);
                if let Some(track) = self.done(result) {
                    self.selected_track = Some(track);
                    self.status = "Separated into Drums, Bass, Vocals and Other on four new tracks just below. They add up to the original. Undo removes them.".into();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_plain_and_never_taken() {
        let dir = std::env::temp_dir().join(format!("audiozero-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_wav(&dir, "Lead: ../evil?");
        assert_eq!(first.parent().unwrap(), dir, "no path escapes: {first:?}");
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".wav")
        );
        assert!(!first.to_string_lossy().contains(".."), "{first:?}");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_wav(&dir, "Lead: ../evil?");
        assert_ne!(first, second);
        assert_eq!(unique_wav(&dir, "").file_name().unwrap(), "Audio.wav");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn written_audio_reads_back_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("audiozero-tools-w-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Sample::new(
            vec![0.0, 0.5, -0.5, 0.25],
            vec![0.1, 0.2, 0.3, 0.4],
            44_100.0,
        );
        let a = write(&dir, "Take", s.clone()).unwrap();
        let b = write(&dir, "Take", s.clone()).unwrap();
        assert_ne!(a.path, b.path);
        let back = decode_file(&a.path).unwrap();
        assert_eq!(back.frames(), 4);
        assert!((back.left()[1] - 0.5).abs() < 1e-4);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
