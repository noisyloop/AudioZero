//! The live visuals window and the video export dialog.
//!
//! The live window draws the chosen look from what's playing: the audio thread's output (through
//! the engine's output tap) and the arrangement's notes under the playhead. It's its own window,
//! so it can go full screen on a second display. Export renders the same look offline, on a
//! background thread, so the video is exact and the app stays usable.

use super::App;
use audiozero_engine::Project;
use audiozero_engine::sample::SamplePool;
use audiozero_visuals::canvas::Canvas;
use audiozero_visuals::export::{CANCELLED, FORMATS, VIDEO_EXTENSIONS};
use audiozero_visuals::{LOOKS, LiveAnalyzer, Look, Range, VideoSettings, create_look};
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

/// The live window's drawing size for its longer side: enough to look sharp when scaled up,
/// small enough to draw every frame.
const LIVE_LONG_SIDE: usize = 640;

pub(super) struct LiveVisuals {
    look_id: &'static str,
    look: Box<dyn Look>,
    format: &'static str,
    analyzer: LiveAnalyzer,
    canvas: Canvas,
    rgba: Vec<u8>,
    texture: Option<egui::TextureHandle>,
}

impl LiveVisuals {
    fn new(look_id: &'static str, format: &'static str, rate: f64) -> Self {
        let (w, h) = live_size(format);
        Self {
            look_id,
            look: create_look(look_id).expect("a built-in look"),
            format,
            analyzer: LiveAnalyzer::new(rate),
            canvas: Canvas::new(w, h),
            rgba: Vec::new(),
            texture: None,
        }
    }

    fn set_look(&mut self, id: &'static str) {
        self.look_id = id;
        self.look = create_look(id).expect("a built-in look");
    }

    fn set_format(&mut self, format: &'static str) {
        self.format = format;
        let (w, h) = live_size(format);
        self.canvas = Canvas::new(w, h);
        self.look = create_look(self.look_id).expect("a built-in look");
    }
}

/// The live canvas size for a format, keeping its shape.
fn live_size(format: &str) -> (usize, usize) {
    let (_, _, w, h) = FORMATS
        .iter()
        .find(|(id, ..)| *id == format)
        .copied()
        .unwrap_or(FORMATS[0]);
    let scale = LIVE_LONG_SIDE as f32 / w.max(h) as f32;
    (
        ((w as f32 * scale) as usize).max(16),
        ((h as f32 * scale) as usize).max(16),
    )
}

pub(super) struct ExportDialog {
    look: &'static str,
    format: &'static str,
    fps: u32,
    loop_only: bool,
    path: String,
    replace: bool,
    ffmpeg: bool,
}

pub(super) struct ExportJob {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    result: std::sync::mpsc::Receiver<Result<audiozero_visuals::export::Exported, String>>,
    path: PathBuf,
}

impl App {
    /// Opens or closes the live visuals window.
    pub(super) fn toggle_visuals(&mut self) {
        if self.visuals.is_some() {
            self.visuals = None;
        } else {
            let rate = self.audio.controller.sample_rate();
            self.visuals = Some(LiveVisuals::new("spectrum", "widescreen", rate));
        }
    }

    /// Runs every frame: reads the output tap, and draws and shows the live window if it's open.
    pub(super) fn visuals_frame(&mut self, ctx: &egui::Context) {
        let mut samples = std::mem::take(&mut self.tap_buffer);
        samples.clear();
        if let Some(tap) = &mut self.tap {
            let n = tap.slots();
            if let Ok(chunk) = tap.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                samples.extend_from_slice(a);
                samples.extend_from_slice(b);
                chunk.commit_all();
            }
        }
        let Some(visuals) = &mut self.visuals else {
            self.tap_buffer = samples;
            return;
        };
        visuals.analyzer.push(&samples);
        self.tap_buffer = samples;
        let beat = self.audio.controller.position();
        let playing = self.audio.controller.is_playing();
        let features = visuals
            .analyzer
            .frame(self.session.project(), beat, playing);
        visuals.look.draw(&mut visuals.canvas, &features);
        visuals.canvas.to_rgba(&mut visuals.rgba);
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [visuals.canvas.width, visuals.canvas.height],
            &visuals.rgba,
        );
        match &mut visuals.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                visuals.texture =
                    Some(ctx.load_texture("live-visuals", image, egui::TextureOptions::LINEAR));
            }
        }

        let mut close = false;
        let mut new_look = None;
        let mut new_format = None;
        let mut export = false;
        let texture = visuals.texture.clone().expect("just set");
        let (look_id, format) = (visuals.look_id, visuals.format);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("audiozero-visuals"),
            egui::ViewportBuilder::default()
                .with_title("AudioZero Visuals")
                .with_inner_size([960.0, 600.0]),
            |ui, _class| {
                let ctx = ui.ctx().clone();
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                egui::Panel::top("visuals-bar").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("live-look")
                            .selected_text(look_name(look_id))
                            .show_ui(ui, |ui| {
                                for (id, name) in LOOKS {
                                    if ui.selectable_label(id == look_id, name).clicked() {
                                        new_look = Some(id);
                                    }
                                }
                            });
                        egui::ComboBox::from_id_salt("live-format")
                            .selected_text(format_name(format))
                            .show_ui(ui, |ui| {
                                for (id, name, ..) in FORMATS {
                                    if ui.selectable_label(id == format, name).clicked() {
                                        new_format = Some(id);
                                    }
                                }
                            });
                        if ui.button("Export video…").clicked() {
                            export = true;
                        }
                        ui.label(
                            egui::RichText::new("F11 or double-click: full screen")
                                .color(super::theme::TEXT_DIM),
                        );
                    });
                });
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                    .show(ui, |ui| {
                        let area = ui.available_rect_before_wrap();
                        let size = texture.size_vec2();
                        let scale = (area.width() / size.x).min(area.height() / size.y);
                        let rect = egui::Rect::from_center_size(area.center(), size * scale);
                        let response = ui.allocate_rect(area, egui::Sense::click());
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                        if response.double_clicked() || ctx.input(|i| i.key_pressed(egui::Key::F11))
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
                        }
                    });
            },
        );
        if let Some(id) = new_look {
            visuals.set_look(id);
        }
        if let Some(id) = new_format {
            visuals.set_format(id);
        }
        if close {
            self.visuals = None;
        }
        if export {
            self.open_export_dialog();
        }
        ctx.request_repaint();
    }

    pub(super) fn open_export_dialog(&mut self) {
        let (look, format) = self
            .visuals
            .as_ref()
            .map_or(("spectrum", "widescreen"), |v| (v.look_id, v.format));
        let stem = self
            .file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map_or("AudioZero video".into(), |s| {
                s.to_string_lossy().into_owned()
            });
        let folder =
            crate::project_folder(self.file.as_deref()).unwrap_or_else(crate::default_folder);
        let ffmpeg = audiozero_visuals::has_ffmpeg();
        let path = if ffmpeg {
            folder.join(format!("{stem} {look}.mp4"))
        } else {
            folder.join(format!("{stem} {look} frames"))
        };
        self.export_dialog = Some(ExportDialog {
            look,
            format,
            fps: 30,
            loop_only: false,
            path: path.display().to_string(),
            replace: false,
            ffmpeg,
        });
    }

    /// The export dialog, and the progress of an export that's running.
    pub(super) fn export_ui(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.export_job {
            if let Ok(result) = job.result.try_recv() {
                self.status = match result {
                    Ok(done) => format!(
                        "Exported {} frames ({:.1} s) to {}",
                        done.frames,
                        done.seconds,
                        done.path.display()
                    ),
                    Err(e) if e == CANCELLED => "Video export cancelled.".into(),
                    Err(e) => format!("Video export failed: {e}"),
                };
                self.export_job = None;
            } else {
                let (done, total) = (job.done.load(Relaxed), job.total.load(Relaxed).max(1));
                self.status = format!(
                    "Exporting {}… {}% ({done}/{total} frames)",
                    job.path.file_name().unwrap_or_default().to_string_lossy(),
                    done * 100 / total
                );
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let Some(dialog) = &mut self.export_dialog else {
            return;
        };
        let mut start = false;
        let mut cancel = false;
        let running = self.export_job.is_some();
        egui::Modal::new(egui::Id::unique("export-video")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Export video");
            egui::Grid::new("export-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Look");
                    egui::ComboBox::from_id_salt("export-look")
                        .selected_text(look_name(dialog.look))
                        .show_ui(ui, |ui| {
                            for (id, name) in LOOKS {
                                ui.selectable_value(&mut dialog.look, id, name);
                            }
                        });
                    ui.end_row();
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("export-format")
                        .selected_text(format_name(dialog.format))
                        .width(320.0)
                        .show_ui(ui, |ui| {
                            for (id, name, ..) in FORMATS {
                                ui.selectable_value(&mut dialog.format, id, name);
                            }
                        });
                    ui.end_row();
                    ui.label("Frame rate");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut dialog.fps, 30, "30 fps");
                        ui.selectable_value(&mut dialog.fps, 60, "60 fps");
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
            let note = if dialog.ffmpeg {
                "End the name with .mp4, .mov, .mkv or .webm for a video with the song's audio; \
                 anything else saves PNG frames and audio.wav in a folder."
            } else {
                "FFmpeg isn't installed, so this saves PNG frames and audio.wav in a folder. \
                 Install FFmpeg to export .mp4, .mov, .mkv or .webm directly."
            };
            ui.label(egui::RichText::new(note).color(super::theme::TEXT_DIM));
            ui.horizontal(|ui| {
                if running {
                    ui.add(egui::Spinner::new());
                    ui.label(&self.status);
                    if ui.button("Cancel export").clicked()
                        && let Some(job) = &self.export_job
                    {
                        job.cancel.store(true, Relaxed);
                    }
                } else {
                    start = ui.button("Export").clicked();
                }
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.export_dialog = None;
        }
        if start {
            self.start_export();
        }
    }

    fn start_export(&mut self) {
        let Some(dialog) = &self.export_dialog else {
            return;
        };
        let path = PathBuf::from(dialog.path.trim());
        let is_video = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        if is_video && !dialog.ffmpeg {
            self.status =
                "Exporting video needs FFmpeg; choose a folder name for PNG frames.".into();
            return;
        }
        if path.as_os_str().is_empty() {
            self.status = "Choose where to save the video.".into();
            return;
        }
        if path.exists() && !dialog.replace {
            self.status = format!(
                "{} already exists; tick Replace or choose another name.",
                path.display()
            );
            return;
        }
        let mut settings = VideoSettings {
            look: dialog.look.into(),
            fps: dialog.fps,
            range: if dialog.loop_only {
                Range::Loop
            } else {
                Range::Song
            },
            ..Default::default()
        };
        if let Err(e) = settings.set_format(dialog.format) {
            self.status = e;
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
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
            .name("audiozero-video-export".into())
            .spawn(move || {
                let mut pool = SamplePool::new(audiozero_visuals::analysis::RATE);
                pool.set_base(folder);
                let result = audiozero_visuals::export(
                    &project,
                    &mut pool,
                    &settings,
                    &path,
                    |frame, frames| {
                        d.store(frame, Relaxed);
                        t.store(frames, Relaxed);
                        !c.load(Relaxed)
                    },
                );
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.export_job = Some(ExportJob {
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

fn look_name(id: &str) -> &'static str {
    LOOKS
        .iter()
        .find(|(i, _)| *i == id)
        .map_or("Look", |(_, n)| n)
}

fn format_name(id: &str) -> &'static str {
    FORMATS
        .iter()
        .find(|(i, ..)| *i == id)
        .map_or("Format", |(_, n, ..)| n)
}
