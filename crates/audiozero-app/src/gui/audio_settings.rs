//! The sound card: the **Audio settings** window (output and input device, sample rate, buffer
//! size), switching devices while the song stays where it is, and carrying on when a device is
//! unplugged.

use super::{App, connect_midi, theme};
use crate::audio::{self, Capabilities, DeviceInfo, Direction, Output};
use crate::input::AudioIn;
use crate::settings::{AudioSettings, COMMON_RATES, milliseconds, offered_buffers, offered_rates};
use audiozero_engine::Command;
use eframe::egui;
use std::path::Path;
use std::time::{Duration, Instant};

/// A device that goes away twice within this long isn't tried a third time.
const RETRY_WINDOW: Duration = Duration::from_secs(5);

/// The Audio settings window's state.
pub(super) struct AudioDialog {
    /// What's chosen in the window, before Apply.
    draft: AudioSettings,
    outputs: Vec<DeviceInfo>,
    inputs: Vec<DeviceInfo>,
    /// What the chosen output can do, and the output it was asked about.
    capabilities: Option<Capabilities>,
    asked: Option<String>,
}

impl AudioDialog {
    fn new(settings: &AudioSettings) -> Self {
        let mut dialog = Self {
            draft: settings.clone(),
            outputs: Vec::new(),
            inputs: Vec::new(),
            capabilities: None,
            asked: None,
        };
        dialog.refresh();
        dialog
    }

    /// Looks for devices again (after plugging one in).
    fn refresh(&mut self) {
        self.outputs = audio::devices(Direction::Output);
        self.inputs = audio::devices(Direction::Input);
        self.ask();
    }

    /// Finds out what the chosen output supports.
    fn ask(&mut self) {
        self.capabilities = audio::capabilities(Direction::Output, self.draft.output.as_deref());
        self.asked = self.draft.output.clone();
    }

    /// The sample rates to offer: what the device supports, or the usual ones if it won't say
    /// (a sound card that's in use often doesn't).
    fn rates(&self) -> Vec<u32> {
        let offered = self
            .capabilities
            .as_ref()
            .map(|c| offered_rates(&c.rates))
            .unwrap_or_default();
        if offered.is_empty() {
            COMMON_RATES[2..6].to_vec()
        } else {
            offered
        }
    }
}

/// The text for a device choice: the device's name, or that it isn't connected.
fn chosen_name(list: &[DeviceInfo], id: Option<&str>) -> String {
    match id {
        None => default_label(list),
        Some(id) => list
            .iter()
            .find(|d| d.id == id)
            .map_or_else(|| format!("Not connected ({id})"), |d| d.name.clone()),
    }
}

fn default_label(list: &[DeviceInfo]) -> String {
    match list.iter().find(|d| d.is_default) {
        Some(device) => format!("System default ({})", device.name),
        None if list.is_empty() => "None found".into(),
        None => "System default".into(),
    }
}

fn device_choice(ui: &mut egui::Ui, salt: &str, list: &[DeviceInfo], chosen: &mut Option<String>) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(chosen_name(list, chosen.as_deref()))
        .width(380.0)
        .truncate()
        .show_ui(ui, |ui| {
            ui.selectable_value(chosen, None, default_label(list));
            for device in list {
                ui.selectable_value(chosen, Some(device.id.clone()), &device.name)
                    .on_hover_text(&device.id);
            }
        });
}

impl App {
    pub(super) fn open_audio_dialog(&mut self) {
        self.audio_dialog = Some(AudioDialog::new(&self.audio_settings));
    }

    /// Draws the Audio settings window, if it's open.
    pub(super) fn audio_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.audio_dialog.take() else {
            return;
        };
        let (mut open, mut apply, mut close) = (true, false, false);
        egui::Window::new("Audio settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("audio-settings")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Output");
                        device_choice(ui, "audio-output", &dialog.outputs, &mut dialog.draft.output);
                        ui.end_row();

                        ui.label("Input");
                        device_choice(ui, "audio-input", &dialog.inputs, &mut dialog.draft.input);
                        ui.end_row();

                        if dialog.asked != dialog.draft.output {
                            dialog.ask();
                        }
                        let device_rate = dialog.capabilities.as_ref().and_then(|c| c.default_rate);
                        let rate_default = device_rate.map_or("Device default".to_string(), |r| {
                            format!("Device default ({r} Hz)")
                        });
                        ui.label("Sample rate");
                        egui::ComboBox::from_id_salt("audio-rate")
                            .selected_text(
                                dialog
                                    .draft
                                    .sample_rate
                                    .map_or(rate_default.clone(), |r| format!("{r} Hz")),
                            )
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut dialog.draft.sample_rate, None, rate_default);
                                for rate in dialog.rates() {
                                    ui.selectable_value(
                                        &mut dialog.draft.sample_rate,
                                        Some(rate),
                                        format!("{rate} Hz"),
                                    );
                                }
                            });
                        ui.end_row();

                        // Delay is shown for the rate that will be used.
                        let rate = dialog
                            .draft
                            .sample_rate
                            .or(device_rate)
                            .unwrap_or(48_000);
                        let frames = |n: u32| format!("{n} frames ({:.1} ms)", milliseconds(n, rate));
                        ui.label("Buffer size");
                        egui::ComboBox::from_id_salt("audio-buffer")
                            .selected_text(
                                dialog
                                    .draft
                                    .buffer
                                    .map_or("Device default".to_string(), frames),
                            )
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut dialog.draft.buffer, None, "Device default");
                                let range = dialog.capabilities.as_ref().and_then(|c| c.buffer);
                                for size in offered_buffers(range) {
                                    ui.selectable_value(
                                        &mut dialog.draft.buffer,
                                        Some(size),
                                        frames(size),
                                    );
                                }
                            })
                            .response
                            .on_hover_text(
                                "A smaller buffer means less delay when you play.\n\
                                 If the sound crackles, choose a larger one.",
                            );
                        ui.end_row();
                    });

                ui.add_space(6.0);
                ui.separator();
                let colour = if self.audio.has_device() {
                    theme::TEXT_DIM
                } else {
                    theme::ACCENT
                };
                ui.label(
                    egui::RichText::new(format!("Now: {}", self.audio.description)).color(colour),
                );
                if let Some(note) = &self.audio.note {
                    ui.label(egui::RichText::new(note).color(theme::ACCENT));
                }
                match self.audio.xruns() {
                    0 => {}
                    n => {
                        ui.label(
                            egui::RichText::new(format!(
                                "The sound card ran dry {n} times (crackles). A larger buffer helps."
                            ))
                            .color(theme::ACCENT),
                        );
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let changed = dialog.draft != self.audio_settings || !self.audio.has_device();
                    apply = ui
                        .add_enabled(changed, egui::Button::new("Apply"))
                        .on_hover_text("Switch to these settings and remember them")
                        .clicked();
                    if ui
                        .button("Look for devices")
                        .on_hover_text("Find a sound card you've just plugged in")
                        .clicked()
                    {
                        dialog.refresh();
                    }
                    close = ui.button("Close").clicked();
                });
            });
        if apply {
            self.apply_audio_settings(dialog.draft.clone());
            dialog.refresh();
        }
        if open && !close {
            self.audio_dialog = Some(dialog);
        }
    }

    /// Switches to `settings` and saves them for next time.
    fn apply_audio_settings(&mut self, settings: AudioSettings) {
        self.audio_settings = settings.sanitized();
        let settings = self.audio_settings.clone();
        self.switch_output(|| Output::open(&settings));
        self.status = match (&self.audio.note, self.audio.has_device()) {
            (Some(note), _) => format!("{note}."),
            (None, true) => format!("Now playing through {}.", self.audio.description),
            (None, false) => self.audio.description.clone(),
        };
        if let Err(e) = self.audio_settings.save() {
            self.status = format!("Can't save the audio settings: {e}");
        }
    }

    /// Replaces the audio output with the one `open` makes, keeping the song, the playhead and
    /// whether it's playing. The old device is let go first, since some allow only one stream.
    fn switch_output(&mut self, open: impl FnOnce() -> Output) {
        self.stop_recording();
        self.release_keys();
        let position = self.audio.controller.position();
        let playing = self.audio.controller.is_playing();
        let folder = self
            .audio
            .controller
            .samples()
            .base()
            .map(Path::to_path_buf);
        let recording_audio = self.audio_in.take().is_some();
        self.midi = None;
        self.audio.close();

        let mut output = open();
        output.controller.samples().set_base(folder);
        let loaded = output.controller.load(self.session.project().clone());
        output.controller.send(Command::Seek(position));
        if playing {
            output.controller.send(Command::Play);
        }
        self.midi = connect_midi(&mut output, &self.ctx).0;
        self.tap = output.controller.take_output_tap();
        self.audio = output;
        self.visuals_rate_changed();
        if let Err(e) = loaded {
            self.status = format!("Can't load the project on the new device: {e}");
        }
        // An armed audio track keeps recording from the (possibly new) input.
        if recording_audio {
            match AudioIn::open(&self.audio_settings) {
                Ok(input) => self.audio_in = Some(input),
                Err(_) => self.armed = None,
            }
        }
    }

    /// Runs every frame: if a device was unplugged, carries on with what's left.
    pub(super) fn watch_devices(&mut self) {
        if self.audio_in.as_ref().is_some_and(AudioIn::is_lost) {
            self.stop_recording();
            self.audio_in = None;
            self.armed = None;
            self.status = "The audio input was disconnected.".into();
        }
        if !self.audio.take_lost() {
            return;
        }
        let now = Instant::now();
        let again = self
            .output_lost_at
            .is_some_and(|before| now.duration_since(before) < RETRY_WINDOW);
        self.output_lost_at = Some(now);
        if again {
            self.switch_output(|| Output::silent("the device keeps disconnecting"));
            self.status =
                "The audio output keeps disconnecting. Choose one in Audio > Audio settings."
                    .into();
        } else {
            // The chosen device if it's still there, else the system's default, else silence.
            let settings = self.audio_settings.clone();
            self.switch_output(|| Output::open(&settings));
            self.status = format!(
                "The audio output was disconnected. Now: {}.",
                self.audio.description
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            name: name.into(),
            is_default,
        }
    }

    #[test]
    fn device_choices_are_named_for_people() {
        let list = [
            device("alsa:default", "Built-in Audio", true),
            device("alsa:hw:1", "Scarlett 2i2", false),
        ];
        assert_eq!(chosen_name(&list, None), "System default (Built-in Audio)");
        assert_eq!(chosen_name(&list, Some("alsa:hw:1")), "Scarlett 2i2");
        assert_eq!(
            chosen_name(&list, Some("alsa:hw:9")),
            "Not connected (alsa:hw:9)"
        );
        assert_eq!(chosen_name(&[], None), "None found");
        assert_eq!(chosen_name(&list[1..], None), "System default");
    }
}
