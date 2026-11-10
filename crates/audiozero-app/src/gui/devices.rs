//! The device panel: the clicked track's instrument and effect chain, with each effect's
//! parameters. Every change is an undo step; dragging a slider is one step for the whole drag.

use super::{App, theme};
use audiozero_engine::Device;
use audiozero_engine::effects::{EFFECTS, effect_name, params_for};
use audiozero_engine::synth::BUILTINS;
use eframe::egui;

enum Action {
    Close,
    Instrument(Option<&'static str>),
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
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Effects").color(theme::TEXT_DIM));

        egui::ScrollArea::vertical().show(ui, |ui| {
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
                    let Some(params) = device.builtin_id().and_then(params_for) else {
                        return;
                    };
                    for p in params {
                        let mut value = device.param(p.name).unwrap_or(p.default);
                        let response = ui.add(
                            egui::Slider::new(&mut value, p.min..=p.max)
                                .text(p.label)
                                .suffix(p.unit),
                        );
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
