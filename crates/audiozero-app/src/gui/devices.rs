//! The device panel: the clicked track's instrument and effect chain, with their parameters and
//! the instrument's presets. Every change is an undo step; dragging a slider is one step for
//! the whole drag.

use super::{App, theme};
use audiozero_engine::Device;
use audiozero_engine::effects::{EFFECTS, Kind, Param, effect_name};
use audiozero_engine::synth::{BUILTINS, instrument_presets, instrument_sections};
use eframe::egui;

/// Draws the control for one parameter: a slider, a tick box or a list, depending on its kind.
/// `value` is changed if the user changed it (see the response's `changed`).
fn param_control(
    ui: &mut egui::Ui,
    salt: (usize, &str),
    p: &Param,
    value: &mut f32,
) -> egui::Response {
    match p.kind {
        Kind::Toggle => {
            let mut on = *value > 0.5;
            let response = ui.checkbox(&mut on, p.label);
            *value = if on { 1.0 } else { 0.0 };
            response
        }
        Kind::Choice(labels) => {
            let mut chosen = (value.round().max(0.0) as usize).min(labels.len().saturating_sub(1));
            let before = chosen;
            let mut response = ui
                .horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("param", salt))
                        .selected_text(labels.get(chosen).copied().unwrap_or(""))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (i, label) in labels.iter().enumerate() {
                                ui.selectable_value(&mut chosen, i, *label);
                            }
                        });
                    ui.label(p.label);
                })
                .response;
            if chosen != before {
                *value = chosen as f32;
                response.mark_changed();
            }
            response
        }
        Kind::Linear | Kind::Log | Kind::Whole => {
            let unit = if p.unit.is_empty() {
                String::new()
            } else {
                format!(" {}", p.unit)
            };
            let mut slider = egui::Slider::new(value, p.min..=p.max)
                .text(p.label)
                .suffix(unit);
            if p.kind == Kind::Log {
                slider = slider.logarithmic(true);
            }
            if p.kind == Kind::Whole {
                slider = slider.integer();
            }
            ui.add(slider)
        }
    }
}

enum Action {
    Beat(&'static str),
    Close,
    Instrument(Option<&'static str>),
    Preset(&'static str),
    Add(&'static str),
    Remove(usize),
}

impl App {
    pub(super) fn devices(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self
            .device_track
            .filter(|&t| t < self.session.project().tracks.len())
        else {
            self.device_track = None;
            return;
        };
        let track = self.session.project().tracks[index].clone();
        let mut action = None;

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading(&track.name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Close").clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        ui.separator();

        ui.label(egui::RichText::new("Instrument").color(theme::text_dim()));
        let current = match &track.instrument {
            None => "None".to_string(),
            Some(device) => device
                .builtin_id()
                .and_then(|id| BUILTINS.iter().find(|(i, _)| *i == id))
                .map_or("Plugin (not loaded)".into(), |(_, name)| name.to_string()),
        };
        egui::ComboBox::from_id_salt(("instrument", index))
            .selected_text(current)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(track.instrument.is_none(), "None (audio track)")
                    .clicked()
                {
                    action = Some(Action::Instrument(None));
                }
                for (id, name) in BUILTINS {
                    let on = track.instrument.as_ref().and_then(Device::builtin_id) == Some(id);
                    if ui.selectable_label(on, name).clicked() {
                        action = Some(Action::Instrument(Some(id)));
                    }
                }
            });
        let instrument_id = track.instrument.as_ref().and_then(Device::builtin_id);
        let presets = instrument_id.map_or(&[][..], instrument_presets);
        let drums = instrument_id == Some("drums");
        if !presets.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.menu_button(if drums { "Kits" } else { "Presets" }, |ui| {
                    for name in presets {
                        if ui.button(*name).clicked() {
                            action = Some(Action::Preset(name));
                        }
                    }
                })
                .response
                .on_hover_text(if drums {
                    "Switch the whole kit. Undo brings the old one back."
                } else {
                    "Replaces every setting below. Undo brings yours back."
                });
                if drums {
                    self.beats_menu(ui, "Beats", |id| action = Some(Action::Beat(id)));
                }
            });
        }
        if drums {
            self.pad_grid(ui, index);
        }
        ui.add_space(4.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let (Some(device), Some(id)) = (&track.instrument, instrument_id)
                && let Some(params) = device.params_info()
            {
                for (i, (title, range)) in instrument_sections(id).iter().enumerate() {
                    egui::CollapsingHeader::new(*title)
                        .id_salt(("instrument-section", index, i))
                        .default_open(i < 2)
                        .show(ui, |ui| {
                            for p in &params[range.clone()] {
                                let mut value = device.param(p.name).unwrap_or(p.default);
                                let response = param_control(ui, (index, p.name), p, &mut value);
                                if response.drag_started() {
                                    self.begin_gesture();
                                }
                                if response.changed() {
                                    let result =
                                        self.session.set_instrument_param(index, p.name, value);
                                    self.finish(result);
                                }
                                if response.drag_stopped() {
                                    self.end_gesture();
                                }
                            }
                        });
                }
            }

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Effects").color(theme::text_dim()));
            if track.effects.is_empty() {
                ui.label(egui::RichText::new("No effects yet.").color(theme::text_dim()));
            }
            for (e, device) in track.effects.iter().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let title = match device {
                            Device::Builtin { id, .. } => effect_name(id).to_string(),
                            Device::Plugin(p) => format!("{} (not loaded)", p.name),
                        };
                        ui.strong(title);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                action = Some(Action::Remove(e));
                            }
                        });
                    });
                    let Some(params) = device.params_info() else {
                        return;
                    };
                    for p in params {
                        let mut value = device.param(p.name).unwrap_or(p.default);
                        let response = param_control(ui, (e, p.name), p, &mut value);
                        if response.drag_started() {
                            self.begin_gesture();
                        }
                        if response.changed() {
                            let result = self.session.set_effect_param(index, e, p.name, value);
                            self.finish(result);
                        }
                        if response.drag_stopped() {
                            self.end_gesture();
                        }
                    }
                });
            }
            ui.add_space(4.0);
            ui.menu_button("+ Add effect", |ui| {
                for (id, name) in EFFECTS {
                    if ui.button(name).clicked() {
                        action = Some(Action::Add(id));
                    }
                }
            });
        });

        match action {
            Some(Action::Close) => self.device_track = None,
            Some(Action::Instrument(id)) => {
                let result = self.session.set_instrument(index, id.map(Device::builtin));
                self.finish(result);
            }
            Some(Action::Beat(id)) => self.insert_beat(Some(index), id),
            Some(Action::Preset(name)) => {
                let result = self.session.set_instrument_preset(index, name);
                if self.done(result.map(|changes| ((), changes))).is_some() {
                    self.status = format!("\"{}\" now plays the {name} preset.", track.name);
                }
            }
            Some(Action::Add(id)) => {
                let result = self.session.add_effect(index, id);
                if self.done(result).is_some() {
                    self.status = format!("Added {} to \"{}\".", effect_name(id), track.name);
                }
            }
            Some(Action::Remove(e)) => {
                let result = self.session.remove_effect(index, e);
                self.finish(result);
            }
            None => {}
        }
    }
}

impl App {
    /// The **Beats** menu: how many repeats, then every ready-made beat by genre.
    pub(super) fn beats_menu(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        mut chosen: impl FnMut(&'static str),
    ) {
        ui.menu_button(label, |ui| {
            ui.horizontal(|ui| {
                ui.label("Bars of each:");
                for repeats in [1, 2, 4, 8] {
                    ui.selectable_value(&mut self.beat_repeats, repeats, repeats.to_string());
                }
            });
            ui.separator();
            for kit in &audiozero_engine::drums::KITS {
                ui.menu_button(kit.name, |ui| {
                    for beat in audiozero_engine::drums::beats_for(kit.id) {
                        let label = format!("{}  ·  {:.0} BPM", beat.name, beat.bpm);
                        if ui.button(label).clicked() {
                            chosen(beat.id);
                            ui.close();
                        }
                    }
                });
            }
        })
        .response
        .on_hover_text("Ready-made beats: each is added as a clip you can edit");
    }

    /// Adds a beat as a clip on `track`, or on a new drum track.
    pub(super) fn insert_beat(&mut self, track: Option<usize>, id: &str) {
        let repeats = self.beat_repeats.max(1);
        let result = self.session.add_drum_beat(track, id, repeats, None);
        if let Some((at, tempo_set)) = self.done(result) {
            let beat = audiozero_engine::drums::beat_by_id(id);
            let name = beat.map_or("beat", |b| b.name);
            self.selected_clip = Some(at);
            self.device_track = Some(at.track);
            self.status = if tempo_set {
                format!(
                    "Added \"{name}\" and set the tempo to {:.0} BPM. Press Space to play.",
                    beat.map_or(120.0, |b| b.bpm)
                )
            } else {
                format!(
                    "Added \"{name}\". Double-click the clip to edit its notes. ({:.0} BPM is its usual tempo.)",
                    beat.map_or(120.0, |b| b.bpm)
                )
            };
        }
    }

    /// The drum pads as a 4 by 4 grid to play with the mouse (the bottom left is the kick, as
    /// on a drum machine). Pressing plays the pad on this track.
    fn pad_grid(&mut self, ui: &mut egui::Ui, track: usize) {
        use audiozero_engine::drums::{FIRST_PAD, PAD_NAMES};
        use audiozero_engine::{Command, LiveNote};
        ui.add_space(6.0);
        let gap = 4.0;
        let width = (ui.available_width() - 8.0 - 3.0 * gap) / 4.0;
        let size = egui::vec2(width.max(40.0), 34.0);
        let mut down: Option<u8> = None;
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        for row in (0..4).rev() {
            ui.horizontal(|ui| {
                for col in 0..4 {
                    let pad = row * 4 + col;
                    let pitch = FIRST_PAD + pad as u8;
                    let lit = self.pad_held == Some(pitch);
                    let button = egui::Button::new(egui::RichText::new(PAD_NAMES[pad]).size(11.0))
                        .min_size(size)
                        .fill(if lit {
                            theme::accent()
                        } else {
                            theme::track(pad / 4).gamma_multiply(0.25)
                        });
                    let response = ui.add(button).on_hover_text(format!(
                        "{} (note {pitch}). Hold the mouse down to hit it.",
                        PAD_NAMES[pad]
                    ));
                    if response.is_pointer_button_down_on() {
                        down = Some(pitch);
                    }
                }
            });
        }
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
        // Hit a pad when it goes down, and let go when it comes up.
        if down != self.pad_held {
            if let Some(previous) = self.pad_held {
                self.send(Command::LiveNote(LiveNote {
                    track,
                    pitch: previous,
                    velocity: 0.0,
                    on: false,
                }));
            }
            if let Some(pitch) = down {
                self.send(Command::LiveNote(LiveNote {
                    track,
                    pitch,
                    velocity: 0.9,
                    on: true,
                }));
            }
            self.pad_held = down;
        }
        ui.add_space(4.0);
    }
}
