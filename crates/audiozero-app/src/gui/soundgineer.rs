//! Soundgineer's own window: every control of the synth, its presets, and a keyboard to play it.
//!
//! It's a second operating-system window, so it can sit beside the arrangement. While it's
//! open, the computer keyboard and the on-screen keyboard play its track, and the Record button
//! puts what you play into a clip.

use super::devices::param_control;
use super::sg_presets::{self, Stored};
use super::{App, theme};
use audiozero_engine::effects::Param;
use audiozero_engine::soundgineer::userpreset::{self, UserPreset};
use audiozero_engine::soundgineer::wavetable::{self, FRAME_SIZE, FRAMES};
use audiozero_engine::soundgineer::{
    FIRST_MATRIX_PARAM, MOD_SLOTS, MOD_SOURCES, PARAMS, PRESET_NAMES, SECTIONS, SHAPE_POINTS,
    Shape, mod_dest_for, shape_param,
};
use audiozero_engine::{Clip, Device};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

/// The window's pages, and which of the synth's sections each shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Sound,
    Filter,
    Modulators,
    Matrix,
    Effects,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Sound,
        Tab::Filter,
        Tab::Modulators,
        Tab::Matrix,
        Tab::Effects,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Sound => "Sound",
            Tab::Filter => "Filter",
            Tab::Modulators => "Envelopes & LFOs",
            Tab::Matrix => "Matrix",
            Tab::Effects => "Effects",
        }
    }

    /// The page a section is on.
    fn of(section: &str) -> Tab {
        match section {
            "Master" | "Osc 1" | "Osc 2" | "Osc 3" | "Sub" | "Noise" => Tab::Sound,
            "Filter 1" | "Filter 2" | "Filter routing" | "Distortion" => Tab::Filter,
            "Matrix" => Tab::Matrix,
            s if s.starts_with("Env ") || s.starts_with("LFO ") || s == "Macros" => Tab::Modulators,
            _ => Tab::Effects,
        }
    }
}

/// Lays `count` controls out in rows of equal cells, as many to a row as fit. (egui's wrapping
/// layout can't wrap a framed group, so the rows are made here.)
fn flow(ui: &mut egui::Ui, count: usize, mut cell: impl FnMut(&mut egui::Ui, usize)) {
    const CELL: f32 = 264.0;
    let columns = ((ui.available_width() / CELL).floor() as usize).max(1);
    for start in (0..count).step_by(columns) {
        ui.horizontal(|ui| {
            for i in start..(start + columns).min(count) {
                ui.allocate_ui(egui::vec2(CELL - 8.0, 30.0), |ui| {
                    ui.group(|ui| cell(ui, i));
                });
            }
        });
    }
}

/// The window's state.
pub(super) struct Window {
    pub track: usize,
    tab: Tab,
    /// The note under the mouse on the on-screen keyboard.
    mouse_note: Option<u8>,
    /// The lowest C shown on the keyboard.
    low_c: u8,
    /// The preset last chosen, and whether anything has been changed since.
    preset: Option<usize>,
    edited: bool,
    /// The LFO shown in the curve editor (0 to 7).
    lfo: usize,
    /// The oscillator shown in the wavetable view (0 to 2).
    wave_osc: usize,
    /// The user's saved presets, read from their folder when the window opens and after changes.
    user: Vec<Stored>,
    /// What's typed in the "Save as" and "Import" boxes.
    save_name: String,
    import_path: String,
}

impl App {
    pub(super) fn open_soundgineer(&mut self, track: usize) {
        self.sg_window = Some(Window {
            track,
            tab: Tab::Sound,
            mouse_note: None,
            low_c: 36,
            preset: None,
            edited: false,
            lfo: 0,
            wave_osc: 0,
            user: sg_presets::folder()
                .map(|d| sg_presets::list(&d))
                .unwrap_or_default(),
            save_name: String::new(),
            import_path: String::new(),
        });
    }

    /// Draws the window if it's open. Called every frame.
    pub(super) fn soundgineer_window(&mut self, ctx: &egui::Context) {
        let Some(track) = self.sg_window.as_ref().map(|w| w.track) else {
            return;
        };
        // Close it if its track went away or isn't a Soundgineer any more.
        let is_sg = self
            .session
            .project()
            .tracks
            .get(track)
            .and_then(|t| t.instrument.as_ref())
            .and_then(Device::builtin_id)
            == Some("soundgineer");
        if !is_sg {
            self.release_mouse_note();
            self.sg_window = None;
            return;
        }
        let title = format!(
            "Soundgineer · {}",
            self.session.project().tracks[track].name
        );
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("audiozero-soundgineer"),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([1120.0, 760.0])
                .with_min_inner_size([760.0, 520.0]),
            |ui, _class| {
                let ctx = ui.ctx().clone();
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                // The computer keyboard plays this synth while the window has the focus.
                self.computer_keys(&ctx);
                self.sg_contents(ui, track);
            },
        );
        if close {
            self.release_mouse_note();
            self.sg_window = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn release_mouse_note(&mut self) {
        let held = self.sg_window.as_mut().and_then(|w| w.mouse_note.take());
        if let (Some(track), Some(pitch)) = (self.sg_window.as_ref().map(|w| w.track), held) {
            self.play_note(track, pitch, 0.0, false);
        }
    }

    fn sg_contents(&mut self, ui: &mut egui::Ui, track: usize) {
        egui::Panel::top("sg-top").show(ui, |ui| {
            ui.add_space(4.0);
            self.sg_top_bar(ui, track);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("sg-keys")
            .frame(egui::Frame::NONE.fill(theme::panel()).inner_margin(6.0))
            .show(ui, |ui| self.sg_keyboard(ui, track));
        egui::CentralPanel::default().show(ui, |ui| {
            let tab = self.sg_window.as_ref().map_or(Tab::Sound, |w| w.tab);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().slider_width = 110.0;
                    if tab == Tab::Matrix {
                        self.sg_matrix(ui, track);
                    } else {
                        self.sg_sections(ui, track, tab);
                    }
                });
        });
    }

    /// The presets, the pages, and the buttons for playing and recording.
    fn sg_top_bar(&mut self, ui: &mut egui::Ui, track: usize) {
        let mut chosen: Option<usize> = None;
        let mut extra: Option<UserPreset> = None;
        let mut remove: Option<std::path::PathBuf> = None;
        ui.horizontal(|ui| {
            let current = self
                .sg_window
                .as_ref()
                .and_then(|w| w.preset.map(|p| (p, w.edited)));
            let label = match current {
                Some((p, true)) => format!("{} (edited)", PRESET_NAMES[p].0),
                Some((p, false)) => PRESET_NAMES[p].0.to_string(),
                None => "Presets".to_string(),
            };
            let count = PRESET_NAMES.len();
            if ui
                .small_button("<")
                .on_hover_text("Previous preset")
                .clicked()
            {
                chosen = Some(current.map_or(count - 1, |(p, _)| (p + count - 1) % count));
            }
            ui.menu_button(egui::RichText::new(label).strong(), |ui| {
                ui.set_max_height(520.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut category = "";
                    for (i, (name, cat)) in PRESET_NAMES.iter().enumerate() {
                        if *cat != category {
                            category = cat;
                            ui.add_space(2.0);
                            ui.label(egui::RichText::new(*cat).color(theme::text_dim()).small());
                        }
                        if ui.button(*name).clicked() {
                            chosen = Some(i);
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.menu_button(
                        format!("More presets ({})", userpreset::extras().len()),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(480.0)
                                .show(ui, |ui| {
                                    let mut category = "";
                                    for (cat, preset) in userpreset::extras() {
                                        if cat != category {
                                            category = cat;
                                            ui.label(
                                                egui::RichText::new(cat)
                                                    .color(theme::text_dim())
                                                    .small(),
                                            );
                                        }
                                        if ui.button(&preset.name).clicked() {
                                            extra = Some(preset.clone());
                                            ui.close();
                                        }
                                    }
                                });
                        },
                    );
                    let mine = self.sg_window.as_ref().map_or(0, |w| w.user.len());
                    ui.menu_button(format!("My presets ({mine})"), |ui| {
                        if mine == 0 {
                            ui.label(
                                egui::RichText::new("None yet. Use Save / Import.")
                                    .color(theme::text_dim()),
                            );
                        }
                        let list = self
                            .sg_window
                            .as_ref()
                            .map(|w| w.user.clone())
                            .unwrap_or_default();
                        for stored in &list {
                            ui.horizontal(|ui| {
                                if ui.button(&stored.preset.name).clicked() {
                                    extra = Some(stored.preset.clone());
                                    ui.close();
                                }
                                if ui
                                    .small_button("×")
                                    .on_hover_text("Delete this preset's file")
                                    .clicked()
                                {
                                    remove = Some(stored.path.clone());
                                }
                            });
                        }
                    });
                });
            })
            .response
            .on_hover_text("59 factory presets. Undo brings back what you had.");
            if ui.small_button(">").on_hover_text("Next preset").clicked() {
                chosen = Some(current.map_or(0, |(p, _)| (p + 1) % count));
            }
            ui.menu_button("Save / Import", |ui| self.sg_preset_files(ui, track));
            ui.separator();
            for tab in Tab::ALL {
                let on = self.sg_window.as_ref().is_some_and(|w| w.tab == tab);
                if ui.selectable_label(on, tab.label()).clicked()
                    && let Some(w) = &mut self.sg_window
                {
                    w.tab = tab;
                }
            }
        });
        ui.horizontal(|ui| {
            let playing = self.audio.controller.is_playing();
            if ui
                .add(egui::Button::new(if playing { "Stop" } else { "Play" }).selected(playing))
                .on_hover_text("Play the song (Space in the main window)")
                .clicked()
            {
                self.toggle_play();
            }
            let recording = self.is_recording();
            if ui
                .add(
                    egui::Button::new(if recording {
                        "Stop recording"
                    } else {
                        "Record"
                    })
                    .selected(recording),
                )
                .on_hover_text(
                    "Records what you play on this synth into a new clip, from the playhead",
                )
                .clicked()
            {
                if !recording && self.armed != Some(track) {
                    self.toggle_arm(track);
                }
                self.toggle_record();
            }
            if ui
                .button("Add clip")
                .on_hover_text(
                    "Adds an empty 4-bar clip on this track and opens it in the piano roll",
                )
                .clicked()
            {
                self.sg_add_clip(track);
            }
            ui.label(
                egui::RichText::new(format!(
                    "Play with A–K (Z/X octave) or click the keys. {}",
                    self.keys_label()
                ))
                .color(theme::text_dim()),
            );
        });
        if let Some(preset) = extra {
            self.sg_apply_user(track, &preset);
        }
        if let Some(path) = remove {
            self.sg_delete_user(&path);
        }
        // A .json file dropped on this window is imported and played.
        let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in dropped.iter().take(8) {
            self.sg_import(track, path);
        }
        if let Some(i) = chosen {
            let name = PRESET_NAMES[i].0;
            let result = self.session.set_instrument_preset(track, name);
            if self.done(result.map(|c| ((), c))).is_some() {
                self.status = format!("Soundgineer: {name}.");
                if let Some(w) = &mut self.sg_window {
                    w.preset = Some(i);
                    w.edited = false;
                }
            }
        }
    }

    /// The "Save / Import" menu: save the sound as a preset file, or import one.
    fn sg_preset_files(&mut self, ui: &mut egui::Ui, track: usize) {
        let Some(dir) = sg_presets::folder() else {
            ui.label("There's no home folder to keep presets in.");
            return;
        };
        ui.set_min_width(300.0);
        ui.label(egui::RichText::new("Save this sound").strong());
        let mut save = false;
        let mut import = false;
        let mut pick = false;
        let mut export = false;
        if let Some(w) = &mut self.sg_window {
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut w.save_name)
                        .hint_text("Preset name")
                        .char_limit(userpreset::MAX_NAME_LENGTH)
                        .desired_width(190.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                save = ui.button("Save").clicked() || enter;
            });
            ui.separator();
            ui.label(egui::RichText::new("Import a preset file").strong());
            ui.horizontal(|ui| {
                pick = ui.button("Choose file…").clicked();
                export = ui.button("Export current sound…").clicked();
            });
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut w.import_path)
                        .hint_text("Path to a .json file")
                        .desired_width(190.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                import = ui.button("Import").clicked() || enter;
            });
        }
        ui.label(
            egui::RichText::new("You can also drop a .json file on this window.")
                .small()
                .color(theme::text_dim()),
        );
        ui.label(
            egui::RichText::new(format!("Presets are kept in {}", dir.display()))
                .small()
                .color(theme::text_dim()),
        );
        if pick {
            ui.close();
            let file = rfd::FileDialog::new()
                .set_title("Import a Soundgineer preset")
                .add_filter("Preset (.json)", &["json"])
                .pick_file();
            if let Some(file) = file {
                self.sg_import(track, &file);
            }
        }
        if export {
            ui.close();
            self.sg_export(track);
        }
        if save {
            self.sg_save_user(track, &dir);
        }
        if import {
            let path = self
                .sg_window
                .as_ref()
                .map(|w| w.import_path.trim().trim_matches('"').to_owned())
                .unwrap_or_default();
            if !path.is_empty() {
                self.sg_import(track, std::path::Path::new(&path));
            }
        }
    }

    /// Saves the current sound to a file anywhere, chosen in a save dialog.
    fn sg_export(&mut self, track: usize) {
        let Some(Device::Builtin { params, .. }) =
            self.session.project().tracks[track].instrument.clone()
        else {
            return;
        };
        let name = self
            .sg_window
            .as_ref()
            .map(|w| w.save_name.clone())
            .unwrap_or_default();
        let preset = UserPreset::capture(&name, &params);
        let file = rfd::FileDialog::new()
            .set_title("Export this sound as a preset")
            .set_file_name(format!("{}.json", userpreset::file_stem(&preset.name)))
            .add_filter("Preset (.json)", &["json"])
            .save_file();
        if let Some(file) = file {
            self.status = match sg_presets::export(&file, &preset) {
                Ok(path) => format!("Exported \"{}\" to {}.", preset.name, path.display()),
                Err(e) => format!("Can't export the preset: {e}"),
            };
        }
    }

    fn sg_refresh_user(&mut self) {
        if let (Some(w), Some(dir)) = (&mut self.sg_window, sg_presets::folder()) {
            w.user = sg_presets::list(&dir);
        }
    }

    /// Saves the track's current sound as a new preset file.
    fn sg_save_user(&mut self, track: usize, dir: &std::path::Path) {
        let Some(Device::Builtin { params, .. }) =
            self.session.project().tracks[track].instrument.clone()
        else {
            return;
        };
        let name = self
            .sg_window
            .as_ref()
            .map(|w| w.save_name.clone())
            .unwrap_or_default();
        let preset = UserPreset::capture(&name, &params);
        match sg_presets::save(dir, &preset) {
            Ok(path) => {
                self.status = format!("Saved \"{}\" to {}.", preset.name, path.display());
                if let Some(w) = &mut self.sg_window {
                    w.save_name.clear();
                }
                self.sg_refresh_user();
            }
            Err(e) => self.status = format!("Can't save the preset: {e}"),
        }
    }

    /// Imports a preset file into the user's folder and plays it on the track.
    fn sg_import(&mut self, track: usize, path: &std::path::Path) {
        let Some(dir) = sg_presets::folder() else {
            self.status = "There's no home folder to keep presets in.".into();
            return;
        };
        match sg_presets::import(&dir, path) {
            Ok(stored) => {
                self.sg_refresh_user();
                self.sg_apply_user(track, &stored.preset);
                if let Some(w) = &mut self.sg_window {
                    w.import_path.clear();
                }
            }
            Err(e) => self.status = format!("Can't import that preset: {e}"),
        }
    }

    fn sg_delete_user(&mut self, path: &std::path::Path) {
        let Some(dir) = sg_presets::folder() else {
            return;
        };
        match sg_presets::delete(&dir, path) {
            Ok(()) => self.sg_refresh_user(),
            Err(e) => self.status = format!("Can't delete the preset: {e}"),
        }
    }

    /// Sets every parameter from a saved or generated preset. One undo step.
    fn sg_apply_user(&mut self, track: usize, preset: &UserPreset) {
        let result = self
            .session
            .set_instrument_settings(track, "soundgineer", &preset.params);
        if self.done(result.map(|c| ((), c))).is_some() {
            self.status = format!("Soundgineer: {}.", preset.name);
            if let Some(w) = &mut self.sg_window {
                w.preset = None;
                w.edited = false;
            }
        }
    }

    /// The oscillators' wavetables drawn in 3D: every frame as a line receding into the
    /// distance, with the frame the Morph knob is on drawn bright. (Morph shown is the knob's
    /// own value; modulation isn't included.)
    fn sg_wavetable_view(&mut self, ui: &mut egui::Ui, device: &Device) {
        let Some(osc) = self.sg_window.as_ref().map(|w| w.wave_osc) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Wavetable")
                    .strong()
                    .color(theme::accent()),
            );
            for n in 0..3 {
                if ui
                    .selectable_label(osc == n, format!("Osc {}", n + 1))
                    .clicked()
                    && let Some(w) = &mut self.sg_window
                {
                    w.wave_osc = n;
                }
            }
        });
        let get = |name: String| {
            let default = PARAMS
                .iter()
                .find(|p| p.name == name)
                .map_or(0.0, |p| p.default);
            device.param(&name).unwrap_or(default)
        };
        let table = get(format!("osc{}.wavetable", osc + 1)).round().max(0.0) as usize;
        let morph = get(format!("osc{}.morph", osc + 1)).clamp(0.0, 1.0);
        let tables = wavetable::tables();
        let table = &tables[table.min(tables.len() - 1)];

        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().clamp(240.0, 760.0), 190.0),
            Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, theme::bg());
        // The same view as the original's: yaw, then pitch, then a simple perspective.
        let project = |x: f32, y: f32, z: f32| -> Pos2 {
            let (px, py, pz) = (x * 1.7 - 0.85, y * 0.35, z * 1.5 - 0.75);
            let (cy, sy) = (0.55f32.cos(), 0.55f32.sin());
            let (px, pz) = (px * cy + pz * sy, -px * sy + pz * cy);
            let (cx, sx) = (0.42f32.cos(), 0.42f32.sin());
            let (py, pz) = (py * cx - pz * sx, py * sx + pz * cx);
            let depth = pz + 2.6;
            let (nx, ny) = (px * 2.0 / depth, py * 2.0 / depth + 0.05);
            let scale = rect.height() * 0.5;
            Pos2::new(rect.center().x + nx * scale, rect.center().y - ny * scale)
        };
        const POINTS: usize = 128;
        let line = |sample: &dyn Fn(usize) -> f32, z: f32| -> Vec<Pos2> {
            (0..POINTS)
                .map(|i| project(i as f32 / (POINTS - 1) as f32, sample(i), z))
                .collect()
        };
        // Back to front, so nearer frames are drawn over farther ones.
        for f in (0..FRAMES).rev() {
            let cycle = table.frame(f);
            let pts = line(
                &|i| cycle[(i * FRAME_SIZE / POINTS).min(FRAME_SIZE - 1)],
                f as f32 / (FRAMES - 1) as f32,
            );
            let fade = 0.25 + 0.5 * (1.0 - f as f32 / (FRAMES - 1) as f32);
            painter.add(egui::Shape::line(
                pts,
                Stroke::new(1.0, theme::text_dim().gamma_multiply(fade)),
            ));
        }
        // The frame the knob is on, blended between the two nearest.
        let at = morph * (FRAMES - 1) as f32;
        let f0 = (at as usize).min(FRAMES - 2);
        let t = at - f0 as f32;
        let (a, b) = (table.frame(f0), table.frame(f0 + 1));
        let pts = line(
            &|i| {
                let k = (i * FRAME_SIZE / POINTS).min(FRAME_SIZE - 1);
                a[k] + (b[k] - a[k]) * t
            },
            morph,
        );
        painter.add(egui::Shape::line(pts, Stroke::new(2.0, theme::accent())));
    }

    /// An empty clip of four bars after the track's last one, opened in the piano roll.
    fn sg_add_clip(&mut self, track: usize) {
        let project = self.session.project();
        let bar = f64::from(project.beats_per_bar);
        let end = project.tracks[track]
            .clips
            .iter()
            .map(Clip::end)
            .fold(0.0, f64::max);
        let clip = Clip {
            name: "Soundgineer".into(),
            start: (end / bar).ceil() * bar,
            length: 4.0 * bar,
            ..Default::default()
        };
        let result = self.session.add_clip(track, clip);
        if let Some(at) = self.done(result) {
            self.selected_clip = Some(at);
            self.open_roll(at);
        }
    }

    /// The controls of every section on `tab`, as flowing groups.
    fn sg_sections(&mut self, ui: &mut egui::Ui, track: usize, tab: Tab) {
        let Some(device) = self.session.project().tracks[track].instrument.clone() else {
            return;
        };
        if tab == Tab::Modulators {
            self.sg_lfo_editor(ui, track, &device);
        }
        if tab == Tab::Sound {
            self.sg_wavetable_view(ui, &device);
        }
        // (The LFO curves have their own editor above, not a slider each.)
        for (title, range) in SECTIONS
            .iter()
            .filter(|(title, _)| Tab::of(title) == tab && !title.ends_with(" curve"))
        {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(*title).strong().color(theme::accent()));
            let params = &PARAMS[range.clone()];
            flow(ui, params.len(), |ui, i| {
                self.sg_param(ui, track, &device, &params[i])
            });
        }
    }

    /// One parameter's control, as an undo step (a drag is one step). Right-click a control that
    /// can be modulated to send an envelope, LFO or other source to it.
    fn sg_param(&mut self, ui: &mut egui::Ui, track: usize, device: &Device, p: &Param) {
        let mut value = device.param(p.name).unwrap_or(p.default);
        let mut response = param_control(ui, (track, p.name), p, &mut value);
        // Sliders only sense drags and treat any mouse button as one, so a right-click would move
        // the value and never open the menu. Ignore the secondary button and sense clicks too.
        let secondary = ui.input(|i| {
            i.pointer.button_down(egui::PointerButton::Secondary)
                || i.pointer.button_released(egui::PointerButton::Secondary)
        });
        if secondary && response.hovered() {
            response.flags.remove(egui::response::Flags::CHANGED);
        }
        let changed = response.changed();
        response = response.interact(egui::Sense::click());
        if response.drag_started() && !secondary {
            self.begin_gesture();
        }
        if changed {
            let result = self.session.set_instrument_param(track, p.name, value);
            self.finish(result);
            if let Some(w) = &mut self.sg_window {
                w.edited = true;
            }
        }
        if response.drag_stopped() {
            self.end_gesture();
        }

        let dest = PARAMS
            .iter()
            .position(|q| q.name == p.name)
            .and_then(mod_dest_for);
        let Some(dest) = dest else {
            return;
        };
        let routes = routes_to(device, dest);
        if !routes.is_empty() {
            let names: Vec<String> = routes
                .iter()
                .map(|r| format!("{} ({:+.2})", MOD_SOURCES[r.source], r.depth))
                .collect();
            ui.label(egui::RichText::new("~").strong().color(theme::accent()))
                .on_hover_text(format!(
                    "Modulated by {}. Right-click the control to change it.",
                    names.join(", ")
                ));
        }
        let mut action: Option<ModAction> = None;
        response.context_menu(|ui| {
            ui.label(egui::RichText::new(p.label).color(theme::text_dim()));
            ui.menu_button("Modulate with", |ui| {
                for (source, name) in MOD_SOURCES.iter().enumerate().skip(1) {
                    if ui.button(*name).clicked() {
                        action = Some(ModAction::Add(source));
                        ui.close();
                    }
                }
            });
            for route in &routes {
                ui.horizontal(|ui| {
                    ui.label(MOD_SOURCES[route.source]);
                    let mut depth = route.depth;
                    if ui
                        .add(egui::Slider::new(&mut depth, -1.0..=1.0).fixed_decimals(2))
                        .changed()
                    {
                        action = Some(ModAction::Depth(route.slot, depth));
                    }
                    if ui.small_button("Remove").clicked() {
                        action = Some(ModAction::Remove(route.slot));
                        ui.close();
                    }
                });
            }
        });
        match action {
            Some(ModAction::Add(source)) => self.sg_add_route(track, device, source, dest, p.label),
            Some(ModAction::Depth(slot, depth)) => {
                let result = self.session.set_instrument_param(
                    track,
                    &format!("mod{}.depth", slot + 1),
                    depth,
                );
                self.finish(result);
            }
            Some(ModAction::Remove(slot)) => {
                self.begin_gesture();
                for (name, value) in [("source", 0.0), ("dest", 0.0), ("depth", 0.0)] {
                    let result = self.session.set_instrument_param(
                        track,
                        &format!("mod{}.{name}", slot + 1),
                        value,
                    );
                    self.finish(result);
                }
                self.end_gesture();
            }
            None => {}
        }
    }

    /// Sends `source` to a parameter, in the first free slot of the matrix.
    fn sg_add_route(
        &mut self,
        track: usize,
        device: &Device,
        source: usize,
        dest: usize,
        label: &str,
    ) {
        let free = (0..MOD_SLOTS).find(|&slot| {
            device
                .param(&format!("mod{}.source", slot + 1))
                .unwrap_or(0.0)
                < 0.5
        });
        let Some(slot) = free else {
            self.status = "All 16 modulation slots are in use (see the Matrix page).".into();
            return;
        };
        self.begin_gesture();
        for (name, value) in [
            ("source", source as f32),
            ("dest", dest as f32),
            ("depth", 0.5),
        ] {
            let result =
                self.session
                    .set_instrument_param(track, &format!("mod{}.{name}", slot + 1), value);
            self.finish(result);
        }
        self.end_gesture();
        self.status = format!(
            "{} now modulates {label}. Right-click it to set the depth.",
            MOD_SOURCES[source]
        );
        if let Some(w) = &mut self.sg_window {
            w.edited = true;
        }
    }

    /// The modulation matrix: source, target and depth for each slot.
    fn sg_matrix(&mut self, ui: &mut egui::Ui, track: usize) {
        let Some(device) = self.session.project().tracks[track].instrument.clone() else {
            return;
        };
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "Choose a source and a target; depth is how far the source moves the target. \
                 Negative depths go the other way.",
            )
            .color(theme::text_dim()),
        );
        ui.add_space(4.0);
        egui::Grid::new("sg-matrix")
            .num_columns(4)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for header in ["", "Source", "Target", "Depth"] {
                    ui.label(egui::RichText::new(header).color(theme::text_dim()));
                }
                ui.end_row();
                for slot in 0..MOD_SLOTS {
                    ui.label(format!("{}", slot + 1));
                    for offset in 0..3 {
                        let p = &PARAMS[FIRST_MATRIX_PARAM + slot * 3 + offset];
                        // Each control shows its value; the label is the column's.
                        let mut value = device.param(p.name).unwrap_or(p.default);
                        let response = match p.kind {
                            audiozero_engine::effects::Kind::Choice(labels) => {
                                let mut index = value.round().max(0.0) as usize;
                                let before = index;
                                let mut response = egui::ComboBox::from_id_salt(("sg-mod", p.name))
                                    .selected_text(labels.get(index).copied().unwrap_or(""))
                                    .width(if offset == 0 { 120.0 } else { 190.0 })
                                    .show_ui(ui, |ui| {
                                        for (i, label) in labels.iter().enumerate() {
                                            ui.selectable_value(&mut index, i, *label);
                                        }
                                    })
                                    .response;
                                if index != before {
                                    value = index as f32;
                                    response.mark_changed();
                                }
                                response
                            }
                            _ => ui.add(
                                egui::Slider::new(&mut value, p.min..=p.max).fixed_decimals(2),
                            ),
                        };
                        if response.drag_started() {
                            self.begin_gesture();
                        }
                        if response.changed() {
                            let result = self.session.set_instrument_param(track, p.name, value);
                            self.finish(result);
                            if let Some(w) = &mut self.sg_window {
                                w.edited = true;
                            }
                        }
                        if response.drag_stopped() {
                            self.end_gesture();
                        }
                    }
                    ui.end_row();
                }
            });
    }

    /// An on-screen keyboard of four octaves. Press a key to play it; slide to play another.
    fn sg_keyboard(&mut self, ui: &mut egui::Ui, track: usize) {
        ui.horizontal(|ui| {
            let low = self.sg_window.as_ref().map_or(36, |w| w.low_c);
            if ui.small_button("Octave -").clicked()
                && let Some(w) = &mut self.sg_window
            {
                w.low_c = low.saturating_sub(12);
            }
            ui.label(format!(
                "C{} to C{}",
                i32::from(low / 12) - 1,
                i32::from(low / 12) + 3
            ));
            if ui.small_button("Octave +").clicked()
                && let Some(w) = &mut self.sg_window
            {
                w.low_c = (low + 12).min(84);
            }
        });
        let low_c = self.sg_window.as_ref().map_or(36, |w| w.low_c);
        let white_keys: Vec<u8> = (low_c..=low_c + 48)
            .filter(|p| !matches!(p % 12, 1 | 3 | 6 | 8 | 10))
            .collect();
        let size = Vec2::new(ui.available_width(), 96.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let key_w = rect.width() / white_keys.len() as f32;
        let painter = ui.painter_at(rect);

        let white_rect = |i: usize| {
            Rect::from_min_size(
                Pos2::new(rect.left() + i as f32 * key_w, rect.top()),
                Vec2::new(key_w - 1.0, rect.height()),
            )
        };
        let black_rect = |pitch: u8| {
            let i = white_keys.iter().position(|&w| w + 1 == pitch).unwrap_or(0);
            Rect::from_min_size(
                Pos2::new(
                    rect.left() + (i as f32 + 1.0) * key_w - key_w * 0.3,
                    rect.top(),
                ),
                Vec2::new(key_w * 0.6, rect.height() * 0.6),
            )
        };
        let blacks: Vec<u8> = (low_c..=low_c + 48)
            .filter(|p| matches!(p % 12, 1 | 3 | 6 | 8 | 10))
            .collect();
        let pitch_at = |pos: Pos2| -> Option<u8> {
            blacks
                .iter()
                .copied()
                .find(|&p| black_rect(p).contains(pos))
                .or_else(|| {
                    white_keys
                        .iter()
                        .enumerate()
                        .find(|(i, _)| white_rect(*i).contains(pos))
                        .map(|(_, &p)| p)
                })
        };

        // Playing with the mouse: press a key, slide to another, let go.
        let down = response.is_pointer_button_down_on();
        let target = if down {
            response.interact_pointer_pos().and_then(pitch_at)
        } else {
            None
        };
        let held = self.sg_window.as_ref().and_then(|w| w.mouse_note);
        if target != held {
            if let Some(pitch) = held {
                self.play_note(track, pitch, 0.0, false);
            }
            if let Some(pitch) = target {
                let velocity = self.keys.velocity;
                self.play_note(track, pitch, velocity, true);
            }
            if let Some(w) = &mut self.sg_window {
                w.mouse_note = target;
            }
        }

        let lit: Vec<u8> = self.keys.held_pitches().chain(target).collect();
        let key_colour = |pitch: u8, base: Color32| {
            if lit.contains(&pitch) {
                theme::accent()
            } else {
                base
            }
        };
        for (i, &pitch) in white_keys.iter().enumerate() {
            let r = white_rect(i);
            painter.rect_filled(
                r,
                2.0,
                key_colour(pitch, Color32::from_rgb(0xe6, 0xe8, 0xec)),
            );
            if pitch % 12 == 0 {
                painter.text(
                    Pos2::new(r.center().x, r.bottom() - 4.0),
                    Align2::CENTER_BOTTOM,
                    format!("C{}", i32::from(pitch / 12) - 1),
                    FontId::proportional(10.0),
                    Color32::from_rgb(0x30, 0x30, 0x30),
                );
            }
        }
        for &pitch in &blacks {
            let r = black_rect(pitch);
            painter.rect_filled(
                r,
                2.0,
                key_colour(pitch, Color32::from_rgb(0x1b, 0x1c, 0x20)),
            );
            painter.rect_stroke(
                r,
                2.0,
                Stroke::new(1.0, Color32::from_black_alpha(160)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// What the right-click menu on a control asked for.
enum ModAction {
    Add(usize),
    Depth(usize, f32),
    Remove(usize),
}

/// One route of the modulation matrix.
struct Route {
    slot: usize,
    source: usize,
    depth: f32,
}

/// The routes that modulate the parameter whose matrix target is `dest`.
fn routes_to(device: &Device, dest: usize) -> Vec<Route> {
    (0..MOD_SLOTS)
        .filter_map(|slot| {
            let get = |name: &str| {
                device
                    .param(&format!("mod{}.{name}", slot + 1))
                    .unwrap_or(0.0)
            };
            let (source, target, depth) = (
                get("source").round() as usize,
                get("dest").round() as usize,
                get("depth"),
            );
            (source > 0 && source < MOD_SOURCES.len() && target == dest && depth != 0.0).then_some(
                Route {
                    slot,
                    source,
                    depth,
                },
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------- the LFO editor

/// The shapes the LFO editor offers to start from.
const TEMPLATES: [&str; 7] = [
    "Triangle", "Sine", "Saw up", "Saw down", "Square", "Pulse", "Stairs",
];

/// A template as a curve.
fn template(name: &str) -> Shape {
    let mut shape = Shape::default();
    let mut set = |points: &[(f32, f32)]| {
        shape.count = points.len();
        for (i, &(x, y)) in points.iter().enumerate() {
            shape.x[i] = x;
            shape.y[i] = y;
            shape.bend[i] = 0.0;
        }
    };
    match name {
        "Sine" => {
            let points: Vec<(f32, f32)> = (0..SHAPE_POINTS)
                .map(|i| {
                    let x = i as f32 / (SHAPE_POINTS - 1) as f32;
                    (x, 0.5 - 0.5 * (x * std::f32::consts::TAU).cos())
                })
                .collect();
            set(&points);
        }
        "Saw up" => set(&[(0.0, 0.0), (1.0, 1.0)]),
        "Saw down" => set(&[(0.0, 1.0), (1.0, 0.0)]),
        "Square" => set(&[(0.0, 1.0), (0.499, 1.0), (0.5, 0.0), (1.0, 0.0)]),
        "Pulse" => set(&[(0.0, 1.0), (0.249, 1.0), (0.25, 0.0), (1.0, 0.0)]),
        "Stairs" => set(&[
            (0.0, 0.0),
            (0.249, 0.0),
            (0.25, 0.33),
            (0.499, 0.33),
            (0.5, 0.66),
            (0.749, 0.66),
            (0.75, 1.0),
            (1.0, 1.0),
        ]),
        _ => {}
    }
    shape
}

impl App {
    /// The curve of LFO `lfo`, as the instrument has it.
    fn sg_shape(device: &Device, lfo: usize) -> Shape {
        let base = shape_param(lfo);
        let get = |at: usize| device.param(PARAMS[at].name).unwrap_or(PARAMS[at].default);
        let mut shape = Shape {
            count: get(base).round().clamp(2.0, SHAPE_POINTS as f32) as usize,
            ..Shape::default()
        };
        for i in 0..SHAPE_POINTS {
            shape.x[i] = get(base + 1 + i * 3);
            shape.y[i] = get(base + 2 + i * 3);
            shape.bend[i] = get(base + 3 + i * 3);
        }
        shape.sort();
        shape
    }

    /// Changes LFO `lfo`'s curve from `old` to `new`, writing only what differs.
    fn sg_set_shape(&mut self, track: usize, lfo: usize, old: &Shape, new: &Shape) {
        let base = shape_param(lfo);
        let mut writes: Vec<(usize, f32)> = Vec::new();
        if old.count != new.count {
            writes.push((base, new.count as f32));
        }
        for i in 0..SHAPE_POINTS {
            for (k, (a, b)) in [
                (old.x[i], new.x[i]),
                (old.y[i], new.y[i]),
                (old.bend[i], new.bend[i]),
            ]
            .into_iter()
            .enumerate()
            {
                if a != b {
                    writes.push((base + 1 + i * 3 + k, b));
                }
            }
        }
        for (at, value) in writes {
            let result = self
                .session
                .set_instrument_param(track, PARAMS[at].name, value);
            self.finish(result);
        }
        if let Some(w) = &mut self.sg_window {
            w.edited = true;
        }
    }

    /// The LFO curve editor: click to add a point, drag a point to move it, drag the diamond on
    /// a segment to bend it, right-click a point to delete it, hold Ctrl to snap to a grid.
    fn sg_lfo_editor(&mut self, ui: &mut egui::Ui, track: usize, device: &Device) {
        let lfo = self.sg_window.as_ref().map_or(0, |w| w.lfo.min(7));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("LFO shape")
                    .strong()
                    .color(theme::accent()),
            );
            for n in 0..8 {
                if ui
                    .selectable_label(n == lfo, format!("{}", n + 1))
                    .clicked()
                    && let Some(w) = &mut self.sg_window
                {
                    w.lfo = n;
                }
            }
            ui.separator();
            ui.label("Start from");
            let mut chosen = None;
            egui::ComboBox::from_id_salt("sg-lfo-template")
                .selected_text("Choose…")
                .show_ui(ui, |ui| {
                    for name in TEMPLATES {
                        if ui.selectable_label(false, name).clicked() {
                            chosen = Some(name);
                        }
                    }
                });
            if let Some(name) = chosen {
                let old = Self::sg_shape(device, lfo);
                self.begin_gesture();
                self.sg_set_shape(track, lfo, &old, &template(name));
                self.end_gesture();
            }
        });
        let shape = Self::sg_shape(device, lfo);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().clamp(240.0, 760.0), 170.0),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, theme::bg());
        let inner = rect.shrink2(Vec2::new(10.0, 12.0));
        let to_screen = |x: f32, y: f32| {
            Pos2::new(
                inner.left() + x * inner.width(),
                inner.bottom() - y * inner.height(),
            )
        };
        let from_screen = |p: Pos2| {
            (
                ((p.x - inner.left()) / inner.width()).clamp(0.0, 1.0),
                ((inner.bottom() - p.y) / inner.height()).clamp(0.0, 1.0),
            )
        };
        for i in 0..=8 {
            let x = inner.left() + inner.width() * i as f32 / 8.0;
            painter.line_segment(
                [Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())],
                Stroke::new(
                    1.0,
                    if i % 4 == 0 {
                        theme::bar()
                    } else {
                        theme::grid()
                    },
                ),
            );
        }
        for i in 0..=4 {
            let y = inner.bottom() - inner.height() * i as f32 / 4.0;
            painter.line_segment(
                [Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)],
                Stroke::new(
                    1.0,
                    if i % 2 == 0 {
                        theme::bar()
                    } else {
                        theme::grid()
                    },
                ),
            );
        }
        let steps = (inner.width() as usize).max(2);
        let curve: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let x = i as f32 / steps as f32;
                to_screen(x, shape.eval(x))
            })
            .collect();
        painter.add(egui::Shape::line(curve, Stroke::new(2.0, theme::accent())));

        let snap = ui.input(|i| i.modifiers.command);
        let snapped = |v: f32| if snap { (v * 16.0).round() / 16.0 } else { v };
        let mut next = shape;
        let mut touched = false;
        let mut delete: Option<usize> = None;
        let mut on_handle = false;

        // The bend handles, one on each segment.
        for i in 0..shape.count - 1 {
            let (x0, x1) = (shape.x[i], shape.x[i + 1]);
            if x1 - x0 < 0.02 {
                continue;
            }
            let mid_x = (x0 + x1) / 2.0;
            let centre = to_screen(mid_x, shape.eval(mid_x));
            let handle = ui.interact(
                Rect::from_center_size(centre, Vec2::splat(14.0)),
                ui.scope_id().with(("sg-bend", lfo, i)),
                Sense::drag(),
            );
            on_handle |= handle.hovered() || handle.dragged();
            let active = handle.hovered() || handle.dragged();
            let size = if active { 5.0 } else { 3.5 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    centre + Vec2::new(0.0, -size),
                    centre + Vec2::new(size, 0.0),
                    centre + Vec2::new(0.0, size),
                    centre + Vec2::new(-size, 0.0),
                ],
                theme::text_dim(),
                Stroke::new(1.0, theme::bg()),
            ));
            if handle.drag_started() {
                self.begin_gesture();
            }
            if handle.dragged()
                && let Some(p) = handle.interact_pointer_pos()
            {
                // Choose the bend that puts the middle of the segment under the pointer.
                let (y0, y1) = (shape.y[i], shape.y[i + 1]);
                if (y1 - y0).abs() > 0.02 {
                    let wanted = (from_screen(p).1 - y0) / (y1 - y0);
                    let wanted = wanted.clamp(0.02, 0.98);
                    let exponent = wanted.ln() / 0.5f32.ln();
                    next.bend[i] = (exponent.log2() / 4.0).clamp(-1.0, 1.0);
                    touched = true;
                }
            }
            if handle.drag_stopped() {
                self.end_gesture();
            }
        }

        // The points.
        for i in 0..shape.count {
            let centre = to_screen(shape.x[i], shape.y[i]);
            let point = ui.interact(
                Rect::from_center_size(centre, Vec2::splat(16.0)),
                ui.scope_id().with(("sg-point", lfo, i)),
                Sense::click_and_drag(),
            );
            on_handle |= point.hovered() || point.dragged();
            let active = point.hovered() || point.dragged();
            painter.circle_filled(centre, if active { 6.5 } else { 5.0 }, theme::track(lfo));
            painter.circle_stroke(
                centre,
                if active { 6.5 } else { 5.0 },
                Stroke::new(1.5, theme::bg()),
            );
            if point.drag_started() {
                self.begin_gesture();
            }
            if point.dragged()
                && let Some(p) = point.interact_pointer_pos()
            {
                let (x, y) = from_screen(p);
                // The ends stay at the edges, so the curve loops cleanly; the others stay
                // between their neighbours.
                let (lo, hi) = (
                    if i == 0 { 0.0 } else { shape.x[i - 1] + 0.004 },
                    if i + 1 == shape.count {
                        1.0
                    } else {
                        shape.x[i + 1] - 0.004
                    },
                );
                next.x[i] = if i == 0 {
                    0.0
                } else if i + 1 == shape.count {
                    1.0
                } else {
                    snapped(x).clamp(lo, hi.max(lo))
                };
                next.y[i] = snapped(y).clamp(0.0, 1.0);
                touched = true;
            }
            if point.drag_stopped() {
                self.end_gesture();
            }
            if point.secondary_clicked() && i > 0 && i + 1 < shape.count && shape.count > 2 {
                delete = Some(i);
            }
            point.on_hover_text(format!(
                "x {:.2}, height {:.2}. Right-click to delete.",
                shape.x[i], shape.y[i]
            ));
        }

        // A click on empty space adds a point there.
        if response.clicked()
            && !on_handle
            && shape.count < SHAPE_POINTS
            && let Some(p) = response.interact_pointer_pos()
        {
            let (x, y) = from_screen(p);
            let (x, y) = (snapped(x).clamp(0.01, 0.99), snapped(y));
            let at = (0..shape.count)
                .find(|&i| shape.x[i] > x)
                .unwrap_or(shape.count);
            if at > 0 && at < shape.count {
                for j in (at..shape.count).rev() {
                    (next.x[j + 1], next.y[j + 1], next.bend[j + 1]) =
                        (shape.x[j], shape.y[j], shape.bend[j]);
                }
                (next.x[at], next.y[at], next.bend[at]) = (x, y, 0.0);
                next.count = shape.count + 1;
                self.begin_gesture();
                self.sg_set_shape(track, lfo, &shape, &next);
                self.end_gesture();
                return;
            }
        }
        if let Some(i) = delete {
            for j in i..shape.count - 1 {
                (next.x[j], next.y[j], next.bend[j]) =
                    (shape.x[j + 1], shape.y[j + 1], shape.bend[j + 1]);
            }
            next.count = shape.count - 1;
            self.begin_gesture();
            self.sg_set_shape(track, lfo, &shape, &next);
            self.end_gesture();
            return;
        }
        if touched {
            self.sg_set_shape(track, lfo, &shape, &next);
        }
        ui.label(
            egui::RichText::new(
                "Click to add a point · drag a point or the diamond on a segment (to bend it) · \
                 right-click a point to delete · Ctrl snaps. Right-click any control to send an LFO to it.",
            )
            .color(theme::text_dim())
            .size(11.0),
        );
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_a_page() {
        for (title, _) in SECTIONS.iter() {
            // The matrix has its own table; the rest are flowing groups on their page.
            let tab = Tab::of(title);
            assert!(Tab::ALL.contains(&tab), "{title}");
        }
        assert_eq!(Tab::of("Osc 2"), Tab::Sound);
        assert_eq!(Tab::of("Filter routing"), Tab::Filter);
        assert_eq!(Tab::of("LFO 8"), Tab::Modulators);
        assert_eq!(Tab::of("Macros"), Tab::Modulators);
        assert_eq!(Tab::of("Matrix"), Tab::Matrix);
        assert_eq!(Tab::of("FX distortion"), Tab::Effects);
        // Every page has something on it.
        for tab in Tab::ALL {
            assert!(
                SECTIONS.iter().any(|(title, _)| Tab::of(title) == tab),
                "{}",
                tab.label()
            );
        }
    }

    #[test]
    fn templates_are_sorted_curves_in_range() {
        for name in TEMPLATES {
            let shape = template(name);
            assert!((2..=SHAPE_POINTS).contains(&shape.count), "{name}");
            for i in 0..shape.count {
                assert!(
                    (0.0..=1.0).contains(&shape.x[i]) && (0.0..=1.0).contains(&shape.y[i]),
                    "{name}"
                );
            }
            assert!(
                shape.x[..shape.count].windows(2).all(|w| w[0] <= w[1]),
                "{name} is in order"
            );
            assert_eq!(
                (shape.x[0], shape.x[shape.count - 1]),
                (0.0, 1.0),
                "{name} spans the cycle"
            );
        }
        let square = template("Square");
        assert!(square.eval(0.25) > 0.99 && square.eval(0.75) < 0.01);
        let sine = template("Sine");
        assert!((sine.eval(0.5) - 1.0).abs() < 0.05 && sine.eval(0.0) < 0.01);
        assert!(
            template("Nope") == Shape::default(),
            "an unknown name is the triangle"
        );
    }

    #[test]
    fn routes_are_read_from_the_matrix() {
        let mut device = Device::builtin("soundgineer");
        let set = |device: &mut Device, name: &str, value: f32| {
            if let Device::Builtin { params, .. } = device {
                params.insert(name.into(), value);
            }
        };
        assert!(routes_to(&device, 5).is_empty());
        set(&mut device, "mod3.source", 8.0);
        set(&mut device, "mod3.dest", 5.0);
        set(&mut device, "mod3.depth", 0.4);
        set(&mut device, "mod4.source", 2.0);
        set(&mut device, "mod4.dest", 6.0);
        set(&mut device, "mod4.depth", 0.9);
        let routes = routes_to(&device, 5);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            (routes[0].slot, routes[0].source, routes[0].depth),
            (2, 8, 0.4)
        );
        set(&mut device, "mod3.depth", 0.0);
        assert!(routes_to(&device, 5).is_empty(), "no depth, no route");
        // A parameter that can be modulated has a target number; one that can't has none.
        let cutoff = PARAMS
            .iter()
            .position(|p| p.name == "filter1.cutoff")
            .unwrap();
        let table = PARAMS
            .iter()
            .position(|p| p.name == "osc1.wavetable")
            .unwrap();
        assert!(mod_dest_for(cutoff).is_some());
        assert!(mod_dest_for(table).is_none());
    }

    #[test]
    fn a_curve_round_trips_through_the_parameters() {
        let mut device = Device::builtin("soundgineer");
        // Default: a triangle.
        assert_eq!(App::sg_shape(&device, 0), Shape::default());
        // Write a new curve's parameters the way the editor does, and read it back.
        let new = template("Stairs");
        if let Device::Builtin { params, .. } = &mut device {
            let base = shape_param(3);
            params.insert(PARAMS[base].name.into(), new.count as f32);
            for i in 0..SHAPE_POINTS {
                params.insert(PARAMS[base + 1 + i * 3].name.into(), new.x[i]);
                params.insert(PARAMS[base + 2 + i * 3].name.into(), new.y[i]);
                params.insert(PARAMS[base + 3 + i * 3].name.into(), new.bend[i]);
            }
        }
        assert_eq!(App::sg_shape(&device, 3), new);
        assert_eq!(
            App::sg_shape(&device, 2),
            Shape::default(),
            "the other LFOs are untouched"
        );
    }
}
