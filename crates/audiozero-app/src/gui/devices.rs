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

        ui.label(egui::RichText::new("Instrument").color(theme::TEXT_DIM));
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
        if !presets.is_empty() {
            ui.add_space(4.0);
            ui.menu_button("Presets", |ui| {
                for name in presets {
                    if ui.button(*name).clicked() {
                        action = Some(Action::Preset(name));
                    }
                }
            })
            .response
            .on_hover_text("Replaces every setting below. Undo brings yours back.");
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
            ui.label(egui::RichText::new("Effects").color(theme::TEXT_DIM));
            if track.effects.is_empty() {
                ui.label(egui::RichText::new("No effects yet.").color(theme::TEXT_DIM));
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
