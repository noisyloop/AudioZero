//! Soundgineer's own window: every control of the synth, its presets, and a keyboard to play it.
//!
//! It's a second operating-system window, so it can sit beside the arrangement. While it's
//! open, the computer keyboard and the on-screen keyboard play its track, and the Record button
//! puts what you play into a clip.

use super::devices::param_control;
use super::{App, theme};
use audiozero_engine::effects::Param;
use audiozero_engine::soundgineer::{
    FIRST_MATRIX_PARAM, MOD_SLOTS, PARAMS, PRESET_NAMES, SECTIONS,
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
                });
            })
            .response
            .on_hover_text("59 factory presets. Undo brings back what you had.");
            if ui.small_button(">").on_hover_text("Next preset").clicked() {
                chosen = Some(current.map_or(0, |(p, _)| (p + 1) % count));
            }
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
        for (title, range) in SECTIONS.iter().filter(|(title, _)| Tab::of(title) == tab) {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(*title).strong().color(theme::accent()));
            let params = &PARAMS[range.clone()];
            flow(ui, params.len(), |ui, i| {
                self.sg_param(ui, track, &device, &params[i])
            });
        }
    }

    /// One parameter's control, as an undo step (a drag is one step).
    fn sg_param(&mut self, ui: &mut egui::Ui, track: usize, device: &Device, p: &Param) {
        let mut value = device.param(p.name).unwrap_or(p.default);
        let response = param_control(ui, (track, p.name), p, &mut value);
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
}
