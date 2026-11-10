//! The desktop window.
//!
//! The project being edited lives in a [`Session`], which is what gets saved. Every edit goes
//! through it: the session validates the edit, records it for undo and reports what changed, and
//! [`App::sync`] sends the audio thread only that ([`Controller::apply`]).
//!
//! [`Controller::apply`]: audiozero_engine::Controller::apply

mod arrangement;
mod piano_roll;
mod theme;

use crate::audio::Output;
use crate::default_folder;
use audiozero_engine::{
    Changes, Clip, ClipRef, Command, Device, EditError, Project, Session, Track, demo,
};
use eframe::egui::{self, Color32, Key, Pos2, Rect, Sense, Vec2};
use std::path::{Path, PathBuf};

pub fn run(start: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AudioZero")
            .with_inner_size([1280.0, 800.0])
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

/// Grid choices. `Bar` follows the time signature.
#[derive(Clone, Copy, PartialEq)]
enum Snap {
    Bar,
    Beat,
    Eighth,
    Sixteenth,
    ThirtySecond,
    Off,
}

impl Snap {
    const ALL: [Snap; 6] = [
        Snap::Bar,
        Snap::Beat,
        Snap::Eighth,
        Snap::Sixteenth,
        Snap::ThirtySecond,
        Snap::Off,
    ];

    fn label(self) -> &'static str {
        match self {
            Snap::Bar => "Bar",
            Snap::Beat => "1/4",
            Snap::Eighth => "1/8",
            Snap::Sixteenth => "1/16",
            Snap::ThirtySecond => "1/32",
            Snap::Off => "Off",
        }
    }

    /// The grid step in beats, or 0 for no snapping.
    fn beats(self, beats_per_bar: u32) -> f64 {
        match self {
            Snap::Bar => f64::from(beats_per_bar),
            Snap::Beat => 1.0,
            Snap::Eighth => 0.5,
            Snap::Sixteenth => 0.25,
            Snap::ThirtySecond => 0.125,
            Snap::Off => 0.0,
        }
    }
}

/// The arrangement view's scroll and zoom.
struct View {
    pixels_per_beat: f32,
    scroll_beats: f32,
    scroll_y: f32,
    follow: bool,
}

struct App {
    audio: Output,
    session: Session,
    file: Option<PathBuf>,
    status: String,
    view: View,
    snap: Snap,
    selected_clip: Option<ClipRef>,
    arrangement_drag: Option<arrangement::Drag>,
    roll: Option<piano_roll::Roll>,
    prompt: Option<PathPrompt>,
    pending: Option<Pending>,
    allow_close: bool,
    title: String,
}

impl App {
    fn new(start: Option<PathBuf>) -> Self {
        let audio = Output::open();
        let mut app = Self {
            status: audio.description.clone(),
            audio,
            session: Session::new(Project::default()).expect("an empty project is valid"),
            file: None,
            view: View {
                pixels_per_beat: 28.0,
                scroll_beats: 0.0,
                scroll_y: 0.0,
                follow: true,
            },
            snap: Snap::Sixteenth,
            selected_clip: None,
            arrangement_drag: None,
            roll: None,
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
    fn install(&mut self, project: Project, file: Option<PathBuf>) {
        let session = match Session::new(project) {
            Ok(session) => session,
            Err(e) => {
                self.status = format!("Can't open project: {e}");
                return;
            }
        };
        match self.audio.controller.load(session.project().clone()) {
            Ok(warnings) => {
                self.status = if warnings.is_empty() {
                    match &file {
                        Some(path) => format!("Opened {}", path.display()),
                        None => "Ready. Double-click a lane to add a clip, double-click a clip to edit its notes.".into(),
                    }
                } else {
                    warnings.join(" · ")
                };
                self.session = session;
                self.file = file;
                self.selected_clip = None;
                self.arrangement_drag = None;
                self.roll = None;
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
        if self.session.is_dirty() {
            self.pending = Some(Pending::New(project, file));
        } else {
            self.install(project, file);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match self.session.project().save(&path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
                self.session.mark_saved();
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

    // ---- the edit pipeline ------------------------------------------------------------------

    /// Applies one edit as one undo step (or as part of the current drag).
    fn edit(&mut self, change: impl FnOnce(&mut Project)) {
        let result = self.session.edit(change);
        self.finish(result);
    }

    /// Starts a drag. Every edit until [`App::end_gesture`] becomes one undo step.
    fn begin_gesture(&mut self) {
        self.session.begin_gesture();
    }

    /// A change in the middle of a drag: applied and heard straight away, recorded for undo when
    /// the drag ends.
    fn live(&mut self, change: impl FnOnce(&mut Project)) {
        self.edit(change);
    }

    fn end_gesture(&mut self) {
        self.session.end_gesture();
    }

    /// Mixer and transport settings aren't undo steps (like most DAWs' faders), but they do
    /// count as unsaved changes.
    fn set_without_undo(&mut self, change: impl FnOnce(&mut Project)) {
        let result = self.session.set(change);
        self.finish(result);
    }

    /// Hears about a typed [`Session`] operation: syncs the audio thread and returns its value,
    /// or shows why it was refused.
    fn done<T>(&mut self, result: Result<(T, Changes), EditError>) -> Option<T> {
        match result {
            Ok((value, changes)) => {
                self.sync(&changes);
                Some(value)
            }
            Err(e) => {
                self.status = format!("Can't do that: {e}");
                None
            }
        }
    }

    fn finish(&mut self, result: Result<Changes, EditError>) {
        self.done(result.map(|changes| ((), changes)));
    }

    /// Sends the audio thread what changed.
    fn sync(&mut self, changes: &Changes) {
        if changes.is_empty() {
            return;
        }
        match self.audio.controller.apply(self.session.project(), changes) {
            Ok(warnings) if !warnings.is_empty() => self.status = warnings.join(" · "),
            Ok(_) => {}
            Err(e) => self.status = e,
        }
    }

    fn undo(&mut self) {
        if let Some(changes) = self.session.undo() {
            self.restored(&changes, "Undone");
        }
    }

    fn redo(&mut self) {
        if let Some(changes) = self.session.redo() {
            self.restored(&changes, "Redone");
        }
    }

    fn restored(&mut self, changes: &Changes, what: &str) {
        self.status = what.into();
        self.sync(changes);
        self.arrangement_drag = None;
        if let Some(roll) = &mut self.roll {
            roll.clear_selection();
        }
        self.validate_selection();
    }

    /// Drops selections that no longer point at anything.
    fn validate_selection(&mut self) {
        if self.selected_clip.is_some_and(|c| self.clip(c).is_none()) {
            self.selected_clip = None;
        }
        if self
            .roll
            .as_ref()
            .is_some_and(|r| self.clip(r.clip).is_none())
        {
            self.roll = None;
        }
    }

    fn clip(&self, at: ClipRef) -> Option<&Clip> {
        self.session.clip(at)
    }

    fn snap_beats(&self) -> f64 {
        self.snap.beats(self.session.project().beats_per_bar)
    }

    /// Rounds `beat` to the grid, unless snapping is off or Alt is held.
    fn snap_to_grid(&self, ctx: &egui::Context, beat: f64) -> f64 {
        let step = self.snap_beats();
        if step <= 0.0 || ctx.input(|i| i.modifiers.alt) {
            beat
        } else {
            (beat / step).round() * step
        }
    }

    // ---- edits ------------------------------------------------------------------------------

    fn add_track(&mut self) {
        let number = self.session.project().tracks.len() + 1;
        let result = self.session.add_track(Track {
            name: format!("Track {number}"),
            instrument: Some(Device::Builtin { id: "sine".into() }),
            ..Default::default()
        });
        self.done(result);
    }

    fn delete_track(&mut self, index: usize) {
        let result = self.session.remove_track(index);
        self.finish(result);
        self.selected_clip = None;
        self.roll = None;
    }

    fn set_instrument(&mut self, index: usize, id: &str) {
        let result = self
            .session
            .set_instrument(index, Some(Device::Builtin { id: id.into() }));
        self.finish(result);
    }

    fn delete_clip(&mut self, at: ClipRef) {
        let result = self.session.remove_clip(at);
        self.finish(result);
        self.selected_clip = None;
        if self.roll.as_ref().is_some_and(|r| r.clip.track == at.track) {
            self.roll = None;
        }
    }

    fn duplicate_clip(&mut self, at: ClipRef) {
        let result = self.session.duplicate_clip(at);
        if let Some(copy) = self.done(result) {
            self.selected_clip = Some(copy);
        }
    }

    fn toggle_clip_mute(&mut self, at: ClipRef) {
        let result = self
            .session
            .update_clip(at, |clip| clip.muted = !clip.muted);
        self.finish(result);
    }

    fn open_roll(&mut self, at: ClipRef) {
        if self.clip(at).is_some() {
            self.selected_clip = Some(at);
            self.roll = Some(piano_roll::Roll::new(at, self.clip(at)));
        }
    }

    /// Delete: notes selected in the piano roll first, else the selected clip.
    fn delete_selection(&mut self) {
        if let Some(roll) = &self.roll
            && roll.has_selection()
        {
            self.delete_selected_notes();
        } else if let Some(at) = self.selected_clip {
            self.delete_clip(at);
        }
    }

    // ---- live mixer and transport -----------------------------------------------------------

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
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(self.session.can_undo(), egui::Button::new("Undo"))
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(self.session.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    self.redo();
                }
                ui.separator();
                let selected = self.selected_clip;
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Duplicate clip"))
                    .clicked()
                    && let Some(at) = selected
                {
                    self.duplicate_clip(at);
                }
                if ui
                    .add_enabled(selected.is_some(), egui::Button::new("Delete clip"))
                    .clicked()
                    && let Some(at) = selected
                {
                    self.delete_clip(at);
                }
                ui.separator();
                if ui.button("Add track").clicked() {
                    self.add_track();
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
            let per_bar = f64::from(self.session.project().beats_per_bar);
            let bar = (position / per_bar).floor() as u64 + 1;
            let beat = (position % per_bar).floor() as u64 + 1;
            let sixteenth = (position.fract() * 4.0).floor() as u64 + 1;
            ui.label(
                egui::RichText::new(format!("{bar:>3}.{beat}.{sixteenth}"))
                    .monospace()
                    .size(16.0),
            );
            ui.separator();

            ui.label("BPM");
            let mut bpm = self.session.project().bpm;
            if ui
                .add(
                    egui::DragValue::new(&mut bpm)
                        .range(20.0..=999.0)
                        .speed(0.5)
                        .fixed_decimals(1),
                )
                .changed()
            {
                self.set_without_undo(|p| p.bpm = bpm);
            }
            let mut looping = self.session.project().loop_enabled;
            if ui.toggle_value(&mut looping, "Loop").changed() {
                self.set_without_undo(|p| p.loop_enabled = looping);
            }
            ui.checkbox(&mut self.view.follow, "Follow");
            ui.separator();

            ui.label("Snap");
            egui::ComboBox::from_id_salt("snap")
                .selected_text(self.snap.label())
                .width(54.0)
                .show_ui(ui, |ui| {
                    for snap in Snap::ALL {
                        ui.selectable_value(&mut self.snap, snap, snap.label());
                    }
                });
            ui.separator();

            ui.label("Master");
            let mut master = self.session.project().master_volume;
            if ui
                .add(egui::Slider::new(&mut master, 0.0..=1.5).show_value(false))
                .changed()
            {
                self.set_without_undo(|p| p.master_volume = master);
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
        let pressed = |key| ctx.input(|i| i.key_pressed(key));
        let (command, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        if command {
            if pressed(Key::S) {
                self.save();
            }
            if pressed(Key::O) {
                self.ask_path(PathMode::Open);
            }
            if pressed(Key::Z) {
                if shift { self.redo() } else { self.undo() }
            }
            if pressed(Key::Y) {
                self.redo();
            }
            if pressed(Key::D)
                && let Some(at) = self.selected_clip
            {
                self.duplicate_clip(at);
            }
            if pressed(Key::A)
                && let Some(roll) = &mut self.roll
            {
                roll.select_all(
                    self.session.project().tracks[roll.clip.track].clips[roll.clip.clip]
                        .notes
                        .len(),
                );
            }
            return;
        }
        if pressed(Key::Space) {
            self.toggle_play();
        }
        if pressed(Key::Home) {
            self.seek(0.0);
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            self.delete_selection();
        }
        if pressed(Key::ArrowUp) {
            self.transpose_selected_notes(if shift { 12 } else { 1 });
        }
        if pressed(Key::ArrowDown) {
            self.transpose_selected_notes(if shift { -12 } else { -1 });
        }
        if pressed(Key::Escape) {
            if let Some(roll) = &mut self.roll
                && roll.has_selection()
            {
                roll.clear_selection();
            } else if self.roll.is_some() {
                self.roll = None;
            } else {
                self.selected_clip = None;
            }
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
        if ctx.input(|i| i.viewport().close_requested())
            && self.session.is_dirty()
            && !self.allow_close
        {
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
        let title = format!(
            "{name}{} — AudioZero",
            if self.session.is_dirty() { " •" } else { "" }
        );
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
        self.validate_selection();
        self.handle_drops_and_close(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("transport").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.roll.is_some() {
            egui::Panel::bottom("piano-roll")
                .resizable(true)
                .default_size(300.0)
                .min_size(160.0)
                .frame(egui::Frame::NONE.fill(theme::PANEL))
                .show(ui, |ui| self.piano_roll(ui));
        }
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
