//! The desktop window: transport, arrangement view, mixer controls, open and save.
//!
//! The UI keeps its own copy of the project, which is what gets saved. Every change is applied
//! to that copy and sent to the audio thread as a [`Command`], where the engine clamps it the
//! same way it clamps project files.

use crate::audio::Output;
use audiozero_engine::{Command, Project, TrackMix, demo};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use std::path::{Path, PathBuf};

pub fn run(start: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AudioZero")
            .with_inner_size([1280.0, 760.0])
            .with_min_inner_size([800.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "AudioZero",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(start)))
        }),
    )
    .map_err(|e| e.to_string())
}

mod theme {
    use eframe::egui::{self, Color32};

    pub const BG: Color32 = Color32::from_rgb(0x16, 0x17, 0x1a);
    pub const PANEL: Color32 = Color32::from_rgb(0x1e, 0x20, 0x24);
    pub const LANE: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x20);
    pub const LANE_ALT: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x23);
    pub const GRID: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x33);
    pub const BAR: Color32 = Color32::from_rgb(0x3a, 0x3e, 0x46);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x90, 0x9a);
    pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x8a, 0x3d);
    pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0xd1, 0x66);
    pub const LOOP: Color32 = Color32::from_rgba_premultiplied(0x30, 0x24, 0x10, 0x60);

    const TRACK_COLOURS: [Color32; 8] = [
        Color32::from_rgb(0xff, 0x8a, 0x3d),
        Color32::from_rgb(0x4f, 0xc3, 0xf7),
        Color32::from_rgb(0xa5, 0xd6, 0x6b),
        Color32::from_rgb(0xce, 0x93, 0xd8),
        Color32::from_rgb(0xff, 0xd5, 0x4f),
        Color32::from_rgb(0x4d, 0xd0, 0xc1),
        Color32::from_rgb(0xf4, 0x8f, 0xb1),
        Color32::from_rgb(0x90, 0xa4, 0xff),
    ];

    pub fn track(index: usize) -> Color32 {
        TRACK_COLOURS[index % TRACK_COLOURS.len()]
    }

    pub fn apply(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.selection.bg_fill = ACCENT.linear_multiply(0.6);
        visuals.widgets.active.bg_fill = ACCENT;
        visuals.hyperlink_color = ACCENT;
        ctx.set_visuals(visuals);
    }
}

/// Something that would throw away unsaved changes, waiting for the user to confirm.
enum Pending {
    New(Project, Option<PathBuf>),
    Quit,
}

#[derive(Clone, Copy, PartialEq)]
enum PathMode {
    Open,
    SaveAs,
}

struct PathPrompt {
    mode: PathMode,
    text: String,
    focused: bool,
}

struct View {
    pixels_per_beat: f32,
    scroll_beats: f32,
    scroll_y: f32,
    follow: bool,
}

struct App {
    audio: Output,
    project: Project,
    file: Option<PathBuf>,
    dirty: bool,
    status: String,
    view: View,
    prompt: Option<PathPrompt>,
    pending: Option<Pending>,
    allow_close: bool,
    title: String,
}

const HEADER_WIDTH: f32 = 210.0;
const RULER_HEIGHT: f32 = 26.0;
const TRACK_HEIGHT: f32 = 68.0;

fn default_folder() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    home.map_or_else(|| PathBuf::from("."), |h| h.join("Music").join("AudioZero"))
}

impl App {
    fn new(start: Option<PathBuf>) -> Self {
        let audio = Output::open();
        let mut app = Self {
            status: audio.description.clone(),
            audio,
            project: Project::default(),
            file: None,
            dirty: false,
            view: View {
                pixels_per_beat: 28.0,
                scroll_beats: 0.0,
                scroll_y: 0.0,
                follow: true,
            },
            prompt: None,
            pending: None,
            allow_close: false,
            title: String::new(),
        };
        match start {
            Some(path) => app.open_path(&path),
            None => app.install(demo::project(), None),
        }
        app
    }

    // ---- project lifecycle ------------------------------------------------------------------

    /// Makes `project` current, in the UI and on the audio thread.
    fn install(&mut self, mut project: Project, file: Option<PathBuf>) {
        if let Err(e) = project.sanitize() {
            self.status = format!("Can't open project: {e}");
            return;
        }
        match self.audio.controller.load(project.clone()) {
            Ok(warnings) => {
                self.status = if warnings.is_empty() {
                    match &file {
                        Some(path) => format!("Opened {}", path.display()),
                        None => "Demo song loaded. Press Space to play.".into(),
                    }
                } else {
                    warnings.join(" · ")
                };
                self.project = project;
                self.file = file;
                self.dirty = false;
                self.view.scroll_beats = 0.0;
                self.view.scroll_y = 0.0;
            }
            Err(e) => self.status = format!("Can't open project: {e}"),
        }
    }

    fn open_path(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(project) => self.replace(project, Some(path.to_path_buf())),
            Err(e) => self.status = format!("Can't open {}: {e}", path.display()),
        }
    }

    /// Replaces the current project, asking first if that would lose unsaved changes.
    fn replace(&mut self, project: Project, file: Option<PathBuf>) {
        if self.dirty {
            self.pending = Some(Pending::New(project, file));
        } else {
            self.install(project, file);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match self.project.save(&path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
                self.dirty = false;
            }
            Err(e) => self.status = format!("Can't save {}: {e}", path.display()),
        }
    }

    fn save(&mut self) {
        match self.file.clone() {
            Some(path) => self.save_to(path),
            None => self.ask_path(PathMode::SaveAs),
        }
    }

    fn ask_path(&mut self, mode: PathMode) {
        let suggestion = match (&self.file, mode) {
            (Some(path), _) => path.clone(),
            (None, PathMode::SaveAs) => default_folder().join("Untitled.json"),
            (None, PathMode::Open) => default_folder(),
        };
        self.prompt = Some(PathPrompt {
            mode,
            text: suggestion.display().to_string(),
            focused: false,
        });
    }

    // ---- live changes -----------------------------------------------------------------------

    fn send(&mut self, command: Command) {
        if !self.audio.controller.send(command) {
            self.status = "The audio engine isn't responding.".into();
        }
    }

    fn toggle_play(&mut self) {
        let command = if self.audio.controller.is_playing() {
            Command::Stop
        } else {
            Command::Play
        };
        self.send(command);
    }

    fn seek(&mut self, beats: f64) {
        self.send(Command::Seek(beats.max(0.0)));
    }

    fn send_mix(&mut self, index: usize) {
        if let Some(t) = self.project.tracks.get(index) {
            let mix = TrackMix {
                volume: t.volume,
                pan: t.pan,
                mute: t.mute,
                solo: t.solo,
            };
            self.send(Command::SetTrackMix { track: index, mix });
            self.dirty = true;
        }
    }

    fn send_loop(&mut self) {
        let p = &self.project;
        let command = Command::SetLoop {
            enabled: p.loop_enabled,
            start: p.loop_start,
            end: p.loop_end,
        };
        self.send(command);
        self.dirty = true;
    }

    // ---- drawing ----------------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New empty project").clicked() {
                    self.replace(Project::default(), None);
                }
                if ui.button("Demo song").clicked() {
                    self.replace(demo::project(), None);
                }
                ui.separator();
                if ui.button("Open…").clicked() {
                    self.ask_path(PathMode::Open);
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
                if ui.button("Save as…").clicked() {
                    self.ask_path(PathMode::SaveAs);
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.separator();

            if ui
                .button("Start")
                .on_hover_text("Back to the start (Home)")
                .clicked()
            {
                self.seek(0.0);
            }
            let playing = self.audio.controller.is_playing();
            let label = if playing { "Stop" } else { "Play" };
            if ui
                .add(egui::Button::new(label).selected(playing))
                .on_hover_text("Space")
                .clicked()
            {
                self.toggle_play();
            }

            let position = self.audio.controller.position();
            let per_bar = f64::from(self.project.beats_per_bar);
            let bar = (position / per_bar).floor() as u64 + 1;
            let beat = (position % per_bar).floor() as u64 + 1;
            let sixteenth = ((position.fract()) * 4.0).floor() as u64 + 1;
            ui.label(
                egui::RichText::new(format!("{bar:>3}.{beat}.{sixteenth}"))
                    .monospace()
                    .size(16.0),
            );
            ui.separator();

            ui.label("BPM");
            let mut bpm = self.project.bpm;
            if ui
                .add(
                    egui::DragValue::new(&mut bpm)
                        .range(20.0..=999.0)
                        .speed(0.5)
                        .fixed_decimals(1),
                )
                .changed()
            {
                self.project.bpm = bpm;
                self.send(Command::SetBpm(bpm));
                self.dirty = true;
            }

            let mut looping = self.project.loop_enabled;
            if ui.toggle_value(&mut looping, "Loop").changed() {
                self.project.loop_enabled = looping;
                self.send_loop();
            }
            ui.checkbox(&mut self.view.follow, "Follow");
            ui.separator();

            ui.label("Master");
            let mut master = self.project.master_volume;
            if ui
                .add(egui::Slider::new(&mut master, 0.0..=1.5).show_value(false))
                .changed()
            {
                self.project.master_volume = master;
                self.send(Command::SetMasterVolume(master));
                self.dirty = true;
            }
            let (l, r) = self.audio.controller.master_peak();
            meter(ui, l, r, Vec2::new(90.0, 14.0));
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&self.status).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let colour = if self.audio.has_device() {
                    theme::TEXT_DIM
                } else {
                    theme::ACCENT
                };
                ui.label(egui::RichText::new(&self.audio.description).color(colour));
            });
        });
    }

    fn arrangement(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(area, Sense::click_and_drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, theme::BG);

        let lanes_left = area.left() + HEADER_WIDTH;
        let ruler = Rect::from_min_max(
            Pos2::new(lanes_left, area.top()),
            Pos2::new(area.right(), area.top() + RULER_HEIGHT),
        );
        let tracks_top = ruler.bottom();
        let lane_width = (area.right() - lanes_left).max(1.0);
        let content_height = self.project.tracks.len() as f32 * TRACK_HEIGHT;
        let visible_height = (area.bottom() - tracks_top).max(1.0);

        // Zoom with Ctrl/Cmd + wheel or pinch, scroll with the wheel (Shift for sideways).
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom != 1.0 {
                let anchor = ui
                    .input(|i| i.pointer.hover_pos())
                    .map_or(lanes_left, |p| p.x.max(lanes_left));
                let beat_at =
                    self.view.scroll_beats + (anchor - lanes_left) / self.view.pixels_per_beat;
                self.view.pixels_per_beat = (self.view.pixels_per_beat * zoom).clamp(4.0, 400.0);
                self.view.scroll_beats =
                    beat_at - (anchor - lanes_left) / self.view.pixels_per_beat;
            }
            self.view.scroll_beats -= scroll.x / self.view.pixels_per_beat;
            self.view.scroll_y -= scroll.y;
            if scroll.x != 0.0 {
                self.view.follow = false;
            }
        }
        self.view.scroll_y = self
            .view
            .scroll_y
            .clamp(0.0, (content_height - visible_height).max(0.0));

        let position = self.audio.controller.position() as f32;
        if self.view.follow && self.audio.controller.is_playing() {
            let visible_beats = lane_width / self.view.pixels_per_beat;
            if position > self.view.scroll_beats + visible_beats * 0.9
                || position < self.view.scroll_beats
            {
                self.view.scroll_beats = position - visible_beats * 0.1;
            }
        }
        self.view.scroll_beats = self.view.scroll_beats.max(0.0);

        let ppb = self.view.pixels_per_beat;
        let scroll_beats = self.view.scroll_beats;
        let beat_to_x = |beat: f32| lanes_left + (beat - scroll_beats) * ppb;
        let x_to_beat = |x: f32| scroll_beats + (x - lanes_left) / ppb;

        // Clicking or dragging on the ruler moves the playhead.
        // Clicking or dragging on the ruler moves the playhead. (`press_origin` is only known while
        // the button is down, so a quick click is handled through `clicked`.)
        let pressed_on_ruler = ui
            .input(|i| i.pointer.press_origin())
            .is_some_and(|o| ruler.contains(o));
        if let Some(pointer) = response.interact_pointer_pos() {
            let clicked_ruler = response.clicked() && ruler.contains(pointer);
            if clicked_ruler || (pressed_on_ruler && response.is_pointer_button_down_on()) {
                self.seek(f64::from(x_to_beat(pointer.x.max(lanes_left))));
            }
        }

        // Grid and ruler.
        let per_bar = self.project.beats_per_bar.max(1) as f32;
        let first_beat = scroll_beats.floor() as i64;
        let last_beat = (scroll_beats + lane_width / ppb).ceil() as i64;
        let label_every = ((48.0 / (ppb * per_bar)).ceil() as i64).max(1);
        let lanes_rect = Rect::from_min_max(Pos2::new(lanes_left, tracks_top), area.right_bottom());
        let lane_painter = painter.with_clip_rect(Rect::from_min_max(
            Pos2::new(lanes_left, area.top()),
            area.right_bottom(),
        ));

        if self.project.loop_enabled {
            let x0 = beat_to_x(self.project.loop_start as f32);
            let x1 = beat_to_x(self.project.loop_end as f32);
            let band = Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, area.bottom()));
            lane_painter.rect_filled(band, 0.0, theme::LOOP);
            lane_painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, ruler.top()), Pos2::new(x1, ruler.top() + 4.0)),
                0.0,
                theme::ACCENT,
            );
        }
        for beat in first_beat.max(0)..=last_beat {
            let x = beat_to_x(beat as f32);
            let is_bar = beat as f32 % per_bar == 0.0;
            if ppb >= 8.0 || is_bar {
                let colour = if is_bar { theme::BAR } else { theme::GRID };
                lane_painter.line_segment(
                    [Pos2::new(x, tracks_top), Pos2::new(x, area.bottom())],
                    Stroke::new(1.0, colour),
                );
            }
            let bar_index = (beat as f32 / per_bar) as i64;
            if is_bar && bar_index % label_every == 0 {
                lane_painter.line_segment(
                    [
                        Pos2::new(x, ruler.bottom() - 8.0),
                        Pos2::new(x, ruler.bottom()),
                    ],
                    Stroke::new(1.0, theme::TEXT_DIM),
                );
                lane_painter.text(
                    Pos2::new(x + 3.0, ruler.center().y),
                    Align2::LEFT_CENTER,
                    bar_index + 1,
                    FontId::proportional(11.0),
                    theme::TEXT_DIM,
                );
            }
        }
        painter.line_segment(
            [ruler.left_bottom(), ruler.right_bottom()],
            Stroke::new(1.0, theme::BAR),
        );

        // Tracks.
        let mut mix_changed = Vec::new();
        for index in 0..self.project.tracks.len() {
            let top = tracks_top + index as f32 * TRACK_HEIGHT - self.view.scroll_y;
            if top > area.bottom() || top + TRACK_HEIGHT < tracks_top {
                continue;
            }
            let row = Rect::from_min_size(
                Pos2::new(area.left(), top),
                Vec2::new(area.width(), TRACK_HEIGHT),
            );
            let lane = Rect::from_min_max(Pos2::new(lanes_left, row.top()), row.right_bottom());
            let header = Rect::from_min_max(row.left_top(), Pos2::new(lanes_left, row.bottom()));
            let colour = theme::track(index);
            let rows_painter = painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(area.left(), tracks_top),
                area.right_bottom(),
            ));

            rows_painter.rect_filled(
                lane.intersect(lanes_rect),
                0.0,
                if index % 2 == 0 {
                    theme::LANE
                } else {
                    theme::LANE_ALT
                }
                .gamma_multiply(0.9),
            );
            let track = &self.project.tracks[index];
            for clip in &track.clips {
                let x0 = beat_to_x(clip.start as f32);
                let x1 = beat_to_x(clip.end() as f32);
                if x1 < lanes_left || x0 > area.right() {
                    continue;
                }
                let body = Rect::from_min_max(
                    Pos2::new(x0 + 1.0, lane.top() + 3.0),
                    Pos2::new(x1 - 1.0, lane.bottom() - 3.0),
                );
                let fill = if clip.muted {
                    colour.gamma_multiply(0.25)
                } else {
                    colour.gamma_multiply(0.45)
                };
                lane_painter.rect_filled(body, CornerRadius::same(4), fill);
                lane_painter.rect_stroke(
                    body,
                    CornerRadius::same(4),
                    Stroke::new(1.0, colour),
                    StrokeKind::Inside,
                );
                let title =
                    Rect::from_min_max(body.left_top(), Pos2::new(body.right(), body.top() + 15.0));
                lane_painter.rect_filled(
                    title,
                    CornerRadius {
                        nw: 4,
                        ne: 4,
                        sw: 0,
                        se: 0,
                    },
                    colour.gamma_multiply(0.8),
                );
                lane_painter
                    .with_clip_rect(
                        title.intersect(lanes_rect.expand2(Vec2::new(0.0, RULER_HEIGHT))),
                    )
                    .text(
                        title.left_center() + Vec2::new(5.0, 0.0),
                        Align2::LEFT_CENTER,
                        &clip.name,
                        FontId::proportional(11.0),
                        Color32::BLACK,
                    );
                // A miniature of the notes.
                if let (Some(lo), Some(hi)) = (
                    clip.notes.iter().map(|n| n.pitch).min(),
                    clip.notes.iter().map(|n| n.pitch).max(),
                ) {
                    let notes_area = Rect::from_min_max(
                        Pos2::new(body.left(), title.bottom() + 3.0),
                        Pos2::new(body.right(), body.bottom() - 3.0),
                    );
                    let span = f32::from(hi - lo).max(12.0);
                    for note in &clip.notes {
                        let nx0 = beat_to_x((clip.start + note.start) as f32);
                        let nx1 = beat_to_x(
                            (clip.start + note.start + note.length).min(clip.end()) as f32,
                        );
                        let y = notes_area.bottom()
                            - f32::from(note.pitch - lo) / span * notes_area.height();
                        let r = Rect::from_min_max(
                            Pos2::new(nx0, y - 1.5),
                            Pos2::new(nx1.max(nx0 + 2.0), y + 1.0),
                        );
                        lane_painter.rect_filled(
                            r.intersect(body),
                            1.0,
                            Color32::from_white_alpha(200),
                        );
                    }
                }
            }

            // Header: name, mute/solo, volume, meter.
            rows_painter.rect_filled(header, 0.0, theme::PANEL);
            rows_painter.rect_filled(
                Rect::from_min_size(header.left_top(), Vec2::new(4.0, header.height())),
                0.0,
                colour,
            );
            rows_painter.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(1.0, theme::BG),
            );
            let inner = header
                .shrink2(Vec2::new(10.0, 6.0))
                .translate(Vec2::new(3.0, 0.0));
            if inner.top() < tracks_top {
                continue; // partly scrolled under the ruler; skip the widgets
            }
            let mut track = self.project.tracks[index].clone();
            let mut changed = false;
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .id_salt(("track", index)),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&track.name).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            changed |= ui
                                .toggle_value(&mut track.solo, "S")
                                .on_hover_text("Solo")
                                .changed();
                            changed |= ui
                                .toggle_value(&mut track.mute, "M")
                                .on_hover_text("Mute")
                                .changed();
                        });
                    });
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut track.volume, 0.0..=2.0).show_value(false))
                            .on_hover_text(format!("Volume {:.0}%", track.volume * 100.0))
                            .changed();
                        let (l, r) = self.audio.controller.track_peak(index);
                        meter(ui, l, r, Vec2::new(ui.available_width(), 10.0));
                    });
                },
            );
            if changed {
                self.project.tracks[index] = track;
                mix_changed.push(index);
            }
        }
        for index in mix_changed {
            self.send_mix(index);
        }

        if self.project.tracks.is_empty() {
            painter.text(
                lanes_rect.center(),
                Align2::CENTER_CENTER,
                "Empty project. Open one with File > Open, or load the demo song.",
                FontId::proportional(14.0),
                theme::TEXT_DIM,
            );
        }

        // Playhead on top.
        let x = beat_to_x(position);
        if x >= lanes_left {
            lane_painter.line_segment(
                [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
                Stroke::new(1.5, theme::PLAYHEAD),
            );
        }
        painter.line_segment(
            [
                Pos2::new(lanes_left, area.top()),
                Pos2::new(lanes_left, area.bottom()),
            ],
            Stroke::new(1.0, theme::BG),
        );
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(prompt) = &mut self.prompt {
            let mut submit = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("path-prompt")).show(ctx, |ui| {
                ui.set_width(520.0);
                ui.heading(if prompt.mode == PathMode::Open {
                    "Open project"
                } else {
                    "Save project as"
                });
                ui.label("Path to an AudioZero .json project:");
                // Read the keys before the text box, which consumes Enter and Escape.
                let (enter, escape) =
                    ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut prompt.text).desired_width(f32::INFINITY));
                if !prompt.focused {
                    edit.request_focus();
                    prompt.focused = true;
                }
                submit = enter;
                cancel = escape;
                ui.horizontal(|ui| {
                    submit |= ui
                        .button(if prompt.mode == PathMode::Open {
                            "Open"
                        } else {
                            "Save"
                        })
                        .clicked();
                    cancel |= ui.button("Cancel").clicked();
                });
            });
            if submit {
                let mode = prompt.mode;
                let mut path = PathBuf::from(prompt.text.trim());
                self.prompt = None;
                match mode {
                    PathMode::Open => self.open_path(&path),
                    PathMode::SaveAs => {
                        if path.extension().is_none() {
                            path.set_extension("json");
                        }
                        self.save_to(path);
                    }
                }
            } else if cancel {
                self.prompt = None;
            }
        }

        if self.pending.is_some() {
            let mut choice = None;
            egui::Modal::new(egui::Id::unique("discard-changes")).show(ctx, |ui| {
                ui.heading("Unsaved changes");
                ui.label("This project has changes that haven't been saved.");
                ui.horizontal(|ui| {
                    if ui.button("Discard changes").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        choice = Some(false);
                    }
                });
            });
            match (choice, self.pending.take()) {
                (Some(true), Some(Pending::New(project, file))) => self.install(project, file),
                (Some(true), Some(Pending::Quit)) => {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                (Some(false), _) | (Some(true), None) => {}
                (None, pending) => self.pending = pending,
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.prompt.is_some() || self.pending.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let (space, home, save, open) = ctx.input(|i| {
            (
                i.key_pressed(Key::Space),
                i.key_pressed(Key::Home),
                i.modifiers.command && i.key_pressed(Key::S),
                i.modifiers.command && i.key_pressed(Key::O),
            )
        });
        if space {
            self.toggle_play();
        }
        if home {
            self.seek(0.0);
        }
        if save {
            self.save();
        }
        if open {
            self.ask_path(PathMode::Open);
        }
    }

    fn handle_drops_and_close(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "json"))
        {
            self.open_path(&path);
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self
            .file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned());
        let title = format!("{name}{} — AudioZero", if self.dirty { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// A stereo level meter: two thin bars, green to orange to red.
fn meter(ui: &mut egui::Ui, left: f32, right: f32, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::BG);
    let half = rect.height() / 2.0;
    for (i, level) in [left, right].into_iter().enumerate() {
        let level = level.clamp(0.0, 1.0);
        // Show in dB-ish terms so quiet signals are still visible.
        let shown = if level <= 0.0 {
            0.0
        } else {
            (1.0 + 20.0 * level.log10() / 48.0).clamp(0.0, 1.0)
        };
        let colour = if level > 0.89 {
            Color32::from_rgb(0xef, 0x53, 0x50)
        } else if level > 0.5 {
            theme::ACCENT
        } else {
            Color32::from_rgb(0x66, 0xbb, 0x6a)
        };
        let top = rect.top() + i as f32 * half;
        let bar = Rect::from_min_size(
            Pos2::new(rect.left(), top + 0.5),
            Vec2::new(rect.width() * shown, half - 1.0),
        );
        painter.rect_filled(bar, 1.0, colour);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.audio.controller.collect_garbage();
        self.handle_drops_and_close(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("transport").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.arrangement(ui));
        self.dialogs(&ctx);
        self.update_title(&ctx);

        // Keep the playhead and meters moving.
        if self.audio.controller.is_playing() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
