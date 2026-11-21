//! The **Tune vocal** window: pitch correction for a recorded audio clip.
//!
//! It finds the pitch of the clip's audio on a background thread, shows it against the notes of
//! the chosen scale, and shows where the settings would move it. **Apply** renders the tuned
//! audio to a new WAV in the project's `Tuned` folder and points the clip at it, as one undo
//! step. The recording itself is never changed.

use super::{App, theme};
use audiozero_engine::music::{self, KEYS, SCALES};
use audiozero_engine::sample::Sample;
use audiozero_engine::session::ClipRef;
use audiozero_engine::tune::{self, PitchTrack, Settings};
use audiozero_engine::{AudioRegion, Clip};
use eframe::egui::{self, Align2, FontId, Pos2, Sense, Stroke, Vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::Receiver;

/// The longest clip that can be tuned, in seconds.
const MAX_SECONDS: f64 = 15.0 * 60.0;

/// Work on a background thread: its progress (0 to 1000), a way to stop it, and its result.
struct Job<T> {
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<Option<T>>,
}

impl<T: Send + 'static> Job<T> {
    fn start(work: impl FnOnce(&mut dyn FnMut(f32) -> bool) -> Option<T> + Send + 'static) -> Self {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, result) = std::sync::mpsc::channel();
        let (p, c) = (Arc::clone(&progress), Arc::clone(&cancel));
        std::thread::spawn(move || {
            let mut report = |done: f32| {
                p.store((done * 1000.0) as u32, Relaxed);
                !c.load(Relaxed)
            };
            let _ = send.send(work(&mut report));
        });
        Self {
            progress,
            cancel,
            result,
        }
    }

    fn percent(&self) -> u32 {
        self.progress.load(Relaxed) / 10
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.cancel.store(true, Relaxed);
    }
}

pub(super) struct TuneWindow {
    at: ClipRef,
    /// The clip as it was when the window opened; Apply is refused if it has changed since.
    clip: Clip,
    /// The part of the file the clip plays.
    source: Arc<Sample>,
    key: u8,
    scale: usize,
    speed_ms: f32,
    amount: f32,
    track: Option<PitchTrack>,
    analysis: Option<Job<PitchTrack>>,
    render: Option<Job<Sample>>,
}

impl TuneWindow {
    fn settings(&self) -> Settings {
        Settings {
            key: self.key,
            scale: SCALES[self.scale],
            speed_ms: self.speed_ms,
            amount: self.amount,
        }
    }
}

/// "A3", "C#4".
fn note_name(note: i32) -> String {
    format!(
        "{}{}",
        KEYS[note.rem_euclid(12) as usize],
        note.div_euclid(12) - 1
    )
}

/// A file in `folder` for the tuned version of a clip called `clip_name`: plain characters
/// only, and not one that exists.
fn tuned_file_name(folder: &std::path::Path, clip_name: &str) -> std::path::PathBuf {
    let clean: String = clip_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let clean = clean.trim();
    let base = if clean.is_empty() { "Clip" } else { clean };
    (1..)
        .map(|n| folder.join(format!("{base} tuned {n}.wav")))
        .find(|p| !p.exists())
        .expect("some number is free")
}

impl App {
    /// Opens the window for an audio clip and starts finding its pitch.
    pub(super) fn open_tune(&mut self, at: ClipRef) {
        let Some(clip) = self.clip(at).cloned() else {
            return;
        };
        let Some(region) = clip.audio.clone() else {
            return;
        };
        let sample = match self.audio.controller.samples().get(&region.file) {
            Ok(sample) => sample,
            Err(e) => {
                self.status = format!("Can't tune this clip: {e}");
                return;
            }
        };
        // The part the clip plays: from its offset, for its length at the song's tempo.
        let project = self.session.project();
        let rate = sample.rate();
        let from = ((region.offset.max(0.0) * rate) as usize).min(sample.frames());
        let seconds = clip.length * 60.0 / project.bpm;
        let to = (from + (seconds * rate).ceil() as usize).min(sample.frames());
        if to <= from {
            self.status = "This clip plays no audio.".into();
            return;
        }
        if (to - from) as f64 / rate > MAX_SECONDS {
            self.status = format!(
                "This clip is too long to tune (the limit is {} minutes).",
                MAX_SECONDS / 60.0
            );
            return;
        }
        let source = Arc::new(Sample::new(
            sample.left()[from..to].to_vec(),
            sample.right()[from..to].to_vec(),
            rate,
        ));
        let scale = SCALES
            .iter()
            .position(|s| s.id == project.scale)
            .unwrap_or(0);
        let key = project.key % 12;
        let audio = Arc::clone(&source);
        self.tune = Some(TuneWindow {
            at,
            clip,
            source,
            key,
            scale,
            speed_ms: 80.0,
            amount: 1.0,
            track: None,
            analysis: Some(Job::start(move |progress| {
                let mono: Vec<f32> = audio
                    .left()
                    .iter()
                    .zip(audio.right())
                    .map(|(l, r)| 0.5 * (l + r))
                    .collect();
                tune::analyze_with(&mono, audio.rate(), progress)
            })),
            render: None,
        });
    }

    /// Draws the window if it's open. Called every frame.
    pub(super) fn tune_ui(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.tune else {
            return;
        };
        // Results from the background.
        if let Some(Ok(track)) = w.analysis.as_ref().map(|j| j.result.try_recv()) {
            w.analysis = None;
            w.track = track;
        }
        let mut rendered = None;
        if let Some(Ok(sample)) = w.render.as_ref().map(|j| j.result.try_recv()) {
            w.render = None;
            rendered = sample;
        }
        if w.analysis.is_some() || w.render.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let (mut open, mut apply, mut close) = (true, false, false);
        egui::Window::new(format!("Tune vocal · {}", w.clip.name))
            .id(egui::Id::unique("tune-vocal"))
            .open(&mut open)
            .collapsible(false)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Key");
                    egui::ComboBox::from_id_salt("tune-key")
                        .selected_text(KEYS[usize::from(w.key)])
                        .width(56.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in KEYS.iter().enumerate() {
                                ui.selectable_value(&mut w.key, i as u8, *name);
                            }
                        });
                    egui::ComboBox::from_id_salt("tune-scale")
                        .selected_text(SCALES[w.scale].name)
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for (i, scale) in SCALES.iter().enumerate() {
                                ui.selectable_value(&mut w.scale, i, scale.name);
                            }
                        });
                    ui.separator();
                    for (name, speed, amount, tip) in [
                        ("Natural", 80.0, 1.0, "Corrects drift, keeps vibrato and slides"),
                        ("Tight", 25.0, 1.0, "Firmly on the note, still sounds sung"),
                        ("Hard tune", 0.0, 1.0, "Snaps to each note at once: the robotic effect"),
                    ] {
                        let on = w.speed_ms == speed && w.amount == amount;
                        if ui.selectable_label(on, name).on_hover_text(tip).clicked() {
                            (w.speed_ms, w.amount) = (speed, amount);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Retune speed");
                    ui.add(
                        egui::Slider::new(&mut w.speed_ms, 0.0..=400.0)
                            .fixed_decimals(0)
                            .suffix(" ms"),
                    )
                    .on_hover_text("How long a correction takes to arrive. 0 snaps at once.");
                    ui.label("Amount");
                    let mut percent = w.amount * 100.0;
                    ui.add(
                        egui::Slider::new(&mut percent, 0.0..=100.0)
                            .fixed_decimals(0)
                            .suffix(" %"),
                    )
                    .on_hover_text("How much of the correction to apply.");
                    w.amount = percent / 100.0;
                });

                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width().max(360.0), 240.0),
                    Sense::hover(),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4.0, theme::bg());
                match (&w.track, &w.analysis) {
                    (Some(track), _) if track.voiced_share() > 0.0 => {
                        draw_pitch(&painter, rect, track, w.settings(), w.source.rate());
                    }
                    (Some(_), _) => {
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "No pitch found. Tuning needs one voice or one instrument playing single notes.",
                            FontId::proportional(13.0),
                            theme::text_dim(),
                        );
                    }
                    (None, Some(job)) => {
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            format!("Finding the pitch… {}%", job.percent()),
                            FontId::proportional(13.0),
                            theme::text_dim(),
                        );
                    }
                    (None, None) => {}
                }
                ui.label(
                    egui::RichText::new(
                        "Grey: as sung. Coloured: after tuning. Lines: the notes of the scale.",
                    )
                    .small()
                    .color(theme::text_dim()),
                );
                ui.horizontal(|ui| {
                    let ready = w.track.as_ref().is_some_and(|t| t.voiced_share() > 0.0)
                        && w.render.is_none();
                    if ui
                        .add_enabled(ready, egui::Button::new("Apply"))
                        .on_hover_text(
                            "Writes the tuned audio to a new file and uses it in the clip. \
                             Undo brings the recording back.",
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    if let Some(job) = &w.render {
                        ui.label(format!("Tuning… {}%", job.percent()));
                    }
                });
            });

        if apply && let Some(track) = w.track.clone() {
            let (source, settings) = (Arc::clone(&w.source), w.settings());
            w.render = Some(Job::start(move |progress| {
                let semitones = tune::correction(&track, source.rate(), settings);
                tune::shift(&source, &track, &semitones, progress)
            }));
        }
        if let Some(sample) = rendered {
            self.finish_tune(&sample);
        } else if close || !open {
            self.tune = None;
        }
    }

    /// Saves the tuned audio and points the clip at it.
    fn finish_tune(&mut self, sample: &Sample) {
        let Some(w) = self.tune.take() else {
            return;
        };
        if self.clip(w.at) != Some(&w.clip) {
            self.status = "The clip changed while it was being tuned, so nothing was done.".into();
            return;
        }
        let folder = crate::project_folder(self.file.as_deref())
            .unwrap_or_else(crate::default_folder)
            .join("Tuned");
        if let Err(e) = std::fs::create_dir_all(&folder) {
            self.status = format!("Can't save the tuned audio in {}: {e}", folder.display());
            return;
        }
        let path = tuned_file_name(&folder, &w.clip.name);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|f| {
                audiozero_engine::wav::write_stereo_24(
                    std::io::BufWriter::new(f),
                    sample.rate().round() as u32,
                    sample.left(),
                    sample.right(),
                )
            });
        if let Err(e) = written {
            self.status = format!("Can't save the tuned audio {}: {e}", path.display());
            return;
        }
        let file = self.audio_file_name(&path);
        self.audio.controller.samples().insert(&file, sample);
        let gain = w.clip.audio.as_ref().map_or(1.0, |a| a.gain);
        let result = self.session.update_clip(w.at, |clip| {
            clip.audio = Some(AudioRegion {
                file,
                offset: 0.0,
                gain,
            });
        });
        if self.done(result.map(|c| ((), c))).is_some() {
            self.status = format!(
                "Tuned to {}. Undo brings the recording back.",
                music::key_name(w.key, SCALES[w.scale].id)
            );
        }
    }
}

/// The pitch as sung and as it would be tuned, over the notes of the scale.
fn draw_pitch(
    painter: &egui::Painter,
    rect: egui::Rect,
    track: &PitchTrack,
    settings: Settings,
    rate: f64,
) {
    let semitones = tune::correction(track, rate, settings);
    let sung: Vec<f32> = track
        .hz
        .iter()
        .map(|&hz| {
            if hz > 0.0 {
                tune::hz_to_note(hz)
            } else {
                f32::NAN
            }
        })
        .collect();
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for (&note, &s) in sung.iter().zip(&semitones) {
        if note.is_finite() {
            low = low.min(note.min(note + s));
            high = high.max(note.max(note + s));
        }
    }
    let (low, high) = (low.floor() - 1.5, high.ceil() + 1.5);
    let inner = rect.shrink2(Vec2::new(8.0, 6.0));
    let left = inner.left() + 30.0;
    let x = |frame: usize| {
        left + (inner.right() - left) * frame as f32 / (sung.len().max(2) - 1) as f32
    };
    let y = |note: f32| inner.bottom() - inner.height() * (note - low) / (high - low);
    // The scale's notes, with names when there's room.
    let room = inner.height() / (high - low);
    for note in low.ceil() as i32..=high.floor() as i32 {
        if !(0..=127).contains(&note) || !music::in_scale(note as u8, settings.key, settings.scale)
        {
            continue;
        }
        let at = y(note as f32);
        painter.line_segment(
            [Pos2::new(left, at), Pos2::new(inner.right(), at)],
            Stroke::new(1.0, theme::grid()),
        );
        if room >= 11.0 || note.rem_euclid(12) == i32::from(settings.key) {
            painter.text(
                Pos2::new(left - 4.0, at),
                Align2::RIGHT_CENTER,
                note_name(note),
                FontId::proportional(10.0),
                theme::text_dim(),
            );
        }
    }
    // One point per pixel column is plenty.
    let step = (sung.len() / (inner.width() as usize * 2).max(1)).max(1);
    let curve = |offset: &dyn Fn(usize) -> f32, stroke: Stroke| {
        let mut run: Vec<Pos2> = Vec::new();
        for frame in (0..sung.len()).step_by(step) {
            if sung[frame].is_finite() {
                run.push(Pos2::new(x(frame), y(sung[frame] + offset(frame))));
            } else if !run.is_empty() {
                painter.add(egui::Shape::line(std::mem::take(&mut run), stroke));
            }
        }
        if !run.is_empty() {
            painter.add(egui::Shape::line(run, stroke));
        }
    };
    curve(&|_| 0.0, Stroke::new(1.5, theme::text_dim()));
    curve(&|f| semitones[f], Stroke::new(2.0, theme::accent()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_have_names() {
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(0), "C-1");
    }

    #[test]
    fn tuned_files_get_plain_free_names() {
        let dir = std::env::temp_dir().join(format!("az-tuned-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = tuned_file_name(&dir, "../Lead: vox?");
        assert_eq!(first.parent().unwrap(), dir);
        assert_eq!(first.file_name().unwrap(), "___Lead_ vox_ tuned 1.wav");
        std::fs::write(&first, b"x").unwrap();
        let second = tuned_file_name(&dir, "../Lead: vox?");
        assert_eq!(second.file_name().unwrap(), "___Lead_ vox_ tuned 2.wav");
        assert_eq!(
            tuned_file_name(&dir, "").file_name().unwrap(),
            "Clip tuned 1.wav"
        );
    }

    #[test]
    fn a_job_reports_and_can_be_stopped() {
        let job = Job::start(|progress| {
            progress(0.5);
            Some(7)
        });
        assert_eq!(job.result.recv().unwrap(), Some(7));
        assert_eq!(job.percent(), 50);
        let stopped: Job<u8> = Job::start(|progress| {
            while progress(0.1) {
                std::thread::yield_now();
            }
            None
        });
        stopped.cancel.store(true, Relaxed);
        assert_eq!(stopped.result.recv().unwrap(), None);
    }
}
