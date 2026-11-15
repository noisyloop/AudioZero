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
pub(super) fn param_control(
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
    OpenEditor,
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

        let instrument_id = track.instrument.as_ref().and_then(Device::builtin_id);
        if track.group {
            ui.label(
                egui::RichText::new(
                    "A group: the tracks inside it are mixed together here, and these effects process that mix.",
                )
                .color(theme::text_dim()),
            );
        } else {
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
                    if instrument_id == Some("soundgineer")
                    && ui
                        .button("Open editor")
                        .on_hover_text(
                            "Soundgineer's own window: every control, the presets and a keyboard",
                        )
                        .clicked()
                {
                    action = Some(Action::OpenEditor);
                }
                });
            }
            if drums {
                self.pad_grid(ui, index);
            }
        }
        ui.add_space(4.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if track.instrument.is_some() {
                self.arp_panel(ui, index, &track);
            }
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
                    if device.builtin_id() == Some("eq") {
                        self.eq_editor(ui, index, e, device);
                    }
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
            Some(Action::OpenEditor) => self.open_soundgineer(index),
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

impl App {
    /// The EQ's curve, with a node for each band: drag a node to set its frequency and gain,
    /// scroll over a bell to make it wider or narrower.
    fn eq_editor(&mut self, ui: &mut egui::Ui, track: usize, effect: usize, device: &Device) {
        use audiozero_engine::fx::{EQ_PARAMS, eq_response_db};
        use egui::{Pos2, Rect, Sense, Stroke, Vec2};
        const F_LOW: f32 = 20.0;
        const F_HIGH: f32 = 20_000.0;
        const RANGE_DB: f32 = 18.0;
        // The curve is drawn for 48 kHz; it differs from the real one only near 20 kHz.
        const RATE: f32 = 48_000.0;

        let values: Vec<f32> = EQ_PARAMS
            .iter()
            .map(|p| device.param(p.name).unwrap_or(p.default))
            .collect();
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(160.0), 150.0),
            Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, theme::bg());
        let half = rect.height() / 2.0 - 6.0;
        let x_of = |f: f32| rect.left() + (f / F_LOW).ln() / (F_HIGH / F_LOW).ln() * rect.width();
        let f_of = |x: f32| {
            F_LOW * (F_HIGH / F_LOW).powf(((x - rect.left()) / rect.width()).clamp(0.0, 1.0))
        };
        let y_of = |db: f32| rect.center().y - db / RANGE_DB * half;
        let db_of = |y: f32| ((rect.center().y - y) / half * RANGE_DB).clamp(-RANGE_DB, RANGE_DB);

        // Grid: octave-ish lines, labelled at 100 Hz, 1 kHz and 10 kHz; 6 dB steps.
        for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0] {
            let x = x_of(f);
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, theme::grid()),
            );
            if matches!(f as u32, 100 | 1000 | 10_000) {
                painter.text(
                    Pos2::new(x + 2.0, rect.bottom() - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    if f >= 1000.0 {
                        format!("{}k", f / 1000.0)
                    } else {
                        format!("{f}")
                    },
                    egui::FontId::proportional(9.0),
                    theme::text_dim(),
                );
            }
        }
        for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
            let y = y_of(db);
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(
                    if db == 0.0 { 1.5 } else { 1.0 },
                    if db == 0.0 {
                        theme::bar()
                    } else {
                        theme::grid()
                    },
                ),
            );
        }
        // The curve.
        let steps = (rect.width() / 2.0) as usize;
        let curve: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let x = rect.left() + rect.width() * i as f32 / steps as f32;
                let db = eq_response_db(&values, RATE, f_of(x)).clamp(-RANGE_DB, RANGE_DB);
                Pos2::new(x, y_of(db))
            })
            .collect();
        painter.add(egui::Shape::line(curve, Stroke::new(2.0, theme::accent())));

        // The four nodes: (frequency, gain, Q) parameter indices.
        let bands = [(0, 1, None), (2, 3, Some(4)), (5, 6, Some(7)), (8, 9, None)];
        for (band, &(fi, gi, qi)) in bands.iter().enumerate() {
            let centre = Pos2::new(x_of(values[fi]), y_of(values[gi]));
            let response = ui.interact(
                Rect::from_center_size(centre, Vec2::splat(18.0)),
                ui.scope_id().with(("eq-node", track, effect, band)),
                Sense::drag(),
            );
            let active = response.hovered() || response.dragged();
            painter.circle_filled(centre, if active { 7.0 } else { 5.5 }, theme::track(band));
            painter.circle_stroke(
                centre,
                if active { 7.0 } else { 5.5 },
                Stroke::new(1.5, theme::bg()),
            );
            let mut hover = format!("{:.0} Hz, {:+.1} dB", values[fi], values[gi]);
            if let Some(q) = qi {
                hover.push_str(&format!(", Q {:.2} (scroll to change)", values[q]));
            }
            let response = response.on_hover_text(hover);
            if response.drag_started() {
                self.begin_gesture();
            }
            if response.dragged()
                && let Some(p) = response.interact_pointer_pos()
            {
                let frequency = f_of(p.x).clamp(EQ_PARAMS[fi].min, EQ_PARAMS[fi].max);
                self.set_effect_param(track, effect, EQ_PARAMS[fi].name, frequency);
                self.set_effect_param(track, effect, EQ_PARAMS[gi].name, db_of(p.y));
            }
            if response.drag_stopped() {
                self.end_gesture();
            }
            if let Some(q) = qi
                && response.hovered()
            {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let widened = (values[q] * (1.0 - scroll * 0.004))
                        .clamp(EQ_PARAMS[q].min, EQ_PARAMS[q].max);
                    self.set_effect_param(track, effect, EQ_PARAMS[q].name, widened);
                    // Keep the panel from scrolling while the wheel is changing a band.
                    ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                }
            }
        }
    }

    fn set_effect_param(&mut self, track: usize, effect: usize, name: &str, value: f32) {
        let result = self.session.set_effect_param(track, effect, name, value);
        self.finish(result);
    }
}

impl App {
    /// The arpeggiator: it turns the notes the track plays into a pattern before the instrument.
    fn arp_panel(&mut self, ui: &mut egui::Ui, index: usize, track: &audiozero_engine::Track) {
        use audiozero_engine::arp::{Mode, Rate};
        /// What the controls reported this frame.
        #[derive(Default)]
        struct Flags {
            changed: bool,
            started: bool,
            stopped: bool,
        }
        impl Flags {
            fn note(&mut self, r: &egui::Response) {
                self.changed |= r.changed();
                self.started |= r.drag_started();
                self.stopped |= r.drag_stopped();
            }
        }
        let mut arp = track.arp;
        let mut flags = Flags::default();
        egui::CollapsingHeader::new(egui::RichText::new("Arpeggiator").strong())
            .id_salt(("arp", index))
            .default_open(arp.on)
            .show(ui, |ui| {
                let on = ui
                    .checkbox(&mut arp.on, "On")
                    .on_hover_text("Plays the notes of this track one at a time, as a pattern");
                flags.note(&on);
                ui.horizontal(|ui| {
                    let mut chosen = arp.mode;
                    egui::ComboBox::from_id_salt(("arp-mode", index))
                        .selected_text(arp.mode.name())
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for mode in Mode::ALL {
                                ui.selectable_value(&mut chosen, mode, mode.name());
                            }
                        });
                    flags.changed |= chosen != arp.mode;
                    arp.mode = chosen;
                    ui.label("Order");
                });
                ui.horizontal(|ui| {
                    let mut chosen = arp.rate;
                    egui::ComboBox::from_id_salt(("arp-rate", index))
                        .selected_text(arp.rate.name())
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for rate in Rate::ALL {
                                ui.selectable_value(&mut chosen, rate, rate.name());
                            }
                        });
                    flags.changed |= chosen != arp.rate;
                    arp.rate = chosen;
                    ui.label("Rate");
                });
                let octaves = ui.add(egui::Slider::new(&mut arp.octaves, 1..=4).text("Octaves"));
                flags.note(&octaves);
                let gate = ui
                    .add(egui::Slider::new(&mut arp.gate, 0.05..=1.0).text("Gate"))
                    .on_hover_text("How much of each step the note lasts");
                flags.note(&gate);
                let swing = ui
                    .add(egui::Slider::new(&mut arp.swing, 0.0..=0.5).text("Swing"))
                    .on_hover_text("How far off-beat steps are pushed late");
                flags.note(&swing);
            });
        if flags.started {
            self.begin_gesture();
        }
        if flags.changed {
            let result = self.session.set_arp(index, arp);
            self.finish(result);
        }
        if flags.stopped {
            self.end_gesture();
        }
    }
}
