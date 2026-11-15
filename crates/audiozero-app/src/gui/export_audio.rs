//! The **Export audio** window: the song (or the loop) as a WAV or MP3 file, rendered offline on
//! a background thread so the app stays usable, with progress and Cancel.

use super::{App, theme};
use audiozero_engine::Project;
use audiozero_engine::export::{self, CANCELLED, Format, MP3_RATES, Range, SAMPLE_RATES, Settings};
use audiozero_engine::sample::SamplePool;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

pub(super) struct AudioExportDialog {
    format: Format,
    sample_rate: u32,
    loop_only: bool,
    path: String,
    replace: bool,
    ffmpeg: bool,
}

pub(super) struct AudioExportJob {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    result: std::sync::mpsc::Receiver<Result<export::Done, String>>,
    path: PathBuf,
}

fn format_name(format: Format) -> String {
    match format {
        Format::Wav16 => "WAV, 16-bit".into(),
        Format::Wav24 => "WAV, 24-bit".into(),
        Format::Mp3(kbps) => format!("MP3, {kbps} kbps"),
    }
}

/// `path` with the extension `format` writes.
fn with_extension(path: &str, format: Format) -> String {
    let mut path = PathBuf::from(path.trim());
    path.set_extension(format.extension());
    path.display().to_string()
}

/// "3.4 MB", "812 KB".
fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{} KB", (bytes / 1000).max(1))
    }
}

impl App {
    pub(super) fn open_audio_export(&mut self) {
        let stem = self
            .file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map_or("AudioZero song".into(), |s| {
                s.to_string_lossy().into_owned()
            });
        let folder =
            crate::project_folder(self.file.as_deref()).unwrap_or_else(crate::default_folder);
        let format = Format::Wav16;
        let path = folder.join(format!("{stem}.{}", format.extension()));
        self.audio_export = Some(AudioExportDialog {
            format,
            sample_rate: 48_000,
            loop_only: false,
            path: path.display().to_string(),
            replace: false,
            ffmpeg: export::has_ffmpeg(),
        });
    }

    /// Whether the song has anything to export, for the menu.
    pub(super) fn can_export_audio(&self) -> bool {
        self.session.project().end_beats() > 0.0
    }

    /// The window, and the progress of an export that's running.
    pub(super) fn audio_export_ui(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.audio_job {
            if let Ok(result) = job.result.try_recv() {
                self.status = match result {
                    Ok(done) => format!(
                        "Exported {:.1} s of audio to {} ({})",
                        done.seconds,
                        done.path.display(),
                        size_text(done.bytes)
                    ),
                    Err(e) if e == CANCELLED => "Audio export cancelled.".into(),
                    Err(e) => format!("Audio export failed: {e}"),
                };
                self.audio_job = None;
            } else {
                let (done, total) = (job.done.load(Relaxed), job.total.load(Relaxed).max(1));
                self.status = format!(
                    "Exporting {}… {}%",
                    job.path.file_name().unwrap_or_default().to_string_lossy(),
                    done * 100 / total
                );
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let Some(dialog) = &mut self.audio_export else {
            return;
        };
        let (mut start, mut close) = (false, false);
        let running = self.audio_job.is_some();
        let project = self.session.project();
        let settings = Settings {
            format: dialog.format,
            sample_rate: dialog.sample_rate,
            range: if dialog.loop_only {
                Range::Loop
            } else {
                Range::Song
            },
            ..Default::default()
        };
        let length = export::plan(project, &settings)
            .map(|(frames, _)| frames as f64 / f64::from(dialog.sample_rate));
        egui::Modal::new(egui::Id::unique("export-audio")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Export audio");
            egui::Grid::new("export-audio-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Format");
                    let before = dialog.format;
                    egui::ComboBox::from_id_salt("export-audio-format")
                        .selected_text(format_name(dialog.format))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for format in [Format::Wav16, Format::Wav24] {
                                ui.selectable_value(
                                    &mut dialog.format,
                                    format,
                                    format_name(format),
                                );
                            }
                            for kbps in MP3_RATES {
                                let format = Format::Mp3(kbps);
                                ui.add_enabled_ui(dialog.ffmpeg, |ui| {
                                    ui.selectable_value(
                                        &mut dialog.format,
                                        format,
                                        format_name(format),
                                    )
                                });
                            }
                        });
                    if dialog.format != before {
                        dialog.path = with_extension(&dialog.path, dialog.format);
                    }
                    ui.end_row();
                    ui.label("Sample rate");
                    ui.horizontal(|ui| {
                        for rate in SAMPLE_RATES {
                            ui.selectable_value(
                                &mut dialog.sample_rate,
                                rate,
                                format!("{rate} Hz"),
                            );
                        }
                    });
                    ui.end_row();
                    ui.label("Length");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut dialog.loop_only, false, "Whole song");
                        ui.selectable_value(&mut dialog.loop_only, true, "Loop range");
                    });
                    ui.end_row();
                    ui.label("Save to");
                    ui.add(egui::TextEdit::singleline(&mut dialog.path).desired_width(360.0));
                    ui.end_row();
                });
            ui.checkbox(&mut dialog.replace, "Replace the file if it exists");
            let mut notes = vec![match length {
                Some(seconds) => format!(
                    "About {:.0}:{:02.0} of audio{}.",
                    (seconds / 60.0).floor(),
                    seconds % 60.0,
                    if dialog.loop_only {
                        ""
                    } else {
                        ", with a 2 second tail for reverb and delays"
                    }
                ),
                None => "There's nothing to export yet: the song has no clips.".into(),
            }];
            if !dialog.ffmpeg {
                notes.push("MP3 needs FFmpeg, which isn't installed. WAV works without it.".into());
            }
            ui.label(egui::RichText::new(notes.join(" ")).color(theme::text_dim()));
            ui.horizontal(|ui| {
                if running {
                    ui.add(egui::Spinner::new());
                    ui.label(&self.status);
                    if ui.button("Cancel export").clicked()
                        && let Some(job) = &self.audio_job
                    {
                        job.cancel.store(true, Relaxed);
                    }
                } else {
                    start = ui
                        .add_enabled(length.is_some(), egui::Button::new("Export"))
                        .clicked();
                }
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        });
        if close {
            self.audio_export = None;
        }
        if start {
            self.start_audio_export();
        }
    }

    fn start_audio_export(&mut self) {
        let Some(dialog) = &self.audio_export else {
            return;
        };
        let path = PathBuf::from(dialog.path.trim());
        if path.as_os_str().is_empty() {
            self.status = "Choose where to save the audio.".into();
            return;
        }
        let settings = Settings {
            format: dialog.format,
            sample_rate: dialog.sample_rate,
            range: if dialog.loop_only {
                Range::Loop
            } else {
                Range::Song
            },
            ..Default::default()
        };
        let replace = dialog.replace;
        let project: Project = self.session.project().clone();
        let folder = crate::project_folder(self.file.as_deref());
        let (done, total, cancel) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let job_path = path.clone();
        let (d, t, c) = (Arc::clone(&done), Arc::clone(&total), Arc::clone(&cancel));
        let spawned = std::thread::Builder::new()
            .name("audiozero-audio-export".into())
            .spawn(move || {
                let mut pool = SamplePool::new(f64::from(settings.sample_rate));
                pool.set_base(folder);
                let result =
                    export::export(&project, &mut pool, &settings, &path, replace, |n, of| {
                        d.store(n, Relaxed);
                        t.store(of, Relaxed);
                        !c.load(Relaxed)
                    });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.audio_job = Some(AudioExportJob {
                    done,
                    total,
                    cancel,
                    result: rx,
                    path: job_path,
                });
            }
            Err(e) => self.status = format!("Can't start the export: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_name_follows_the_format() {
        assert_eq!(
            with_extension("/music/song.wav", Format::Mp3(320)),
            "/music/song.mp3"
        );
        assert_eq!(with_extension(" song ", Format::Wav24), "song.wav");
        assert_eq!(with_extension("a.b.mp3", Format::Wav16), "a.b.wav");
        assert_eq!(size_text(812), "1 KB");
        assert_eq!(size_text(3_400_000), "3.4 MB");
    }
}
