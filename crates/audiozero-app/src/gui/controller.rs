//! Game controller input: a DualShock 4 (or another gamepad) plays the live target track.
//!
//! The controller is read on its own thread (`audiozero-controller`). Each frame this turns
//! what it sent into notes through the same [`App::play_note`] the computer keyboard uses, so
//! the armed track, the song's key and scale, recording and the audio thread's lock-free queue
//! all behave exactly as they do for a MIDI keyboard. Nothing here runs on the audio thread.

use super::{App, theme};
use audiozero_controller::{Action, Ax, Btn, Controllers, Layout, Mapper, Profile, RawEvent, midi};
use audiozero_engine::{Device, drums, music};
use eframe::egui;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

const LOG_LINES: usize = 200;

pub(super) struct ControllerUi {
    input: Controllers,
    mapper: Mapper,
    /// Whether the settings window is open.
    pub(super) window_open: bool,
    /// The controller that plays. One at a time for now.
    active: Option<usize>,
    /// The track the mapper's notes go to, so a note always ends where it started.
    last_target: Option<usize>,
    last_tick: Instant,
    // What's shown in the live activity view.
    down: HashSet<Btn>,
    sticks: [f32; 4],
    triggers: [f32; 2],
    log: VecDeque<String>,
    path: Option<PathBuf>,
}

impl ControllerUi {
    pub(super) fn new(ctx: &egui::Context) -> Self {
        let path = crate::settings::file().map(|f| f.with_file_name("controller.json"));
        let profile = path.as_deref().map_or_else(Profile::default, Profile::load);
        let ctx = ctx.clone();
        Self {
            input: Controllers::start(move || ctx.request_repaint()),
            mapper: Mapper::new(profile),
            window_open: false,
            active: None,
            last_target: None,
            last_tick: Instant::now(),
            down: HashSet::new(),
            sticks: [0.0; 4],
            triggers: [0.0; 2],
            log: VecDeque::new(),
            path,
        }
    }

    fn log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn save(&self) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or("there's no home folder to keep it in")?;
        self.mapper.profile.save(path)
    }

    /// Updates the activity view and picks the active controller. Whether the mapper should see
    /// the event.
    fn observe(&mut self, event: &RawEvent) -> bool {
        match event {
            RawEvent::Connected { pad, name } => {
                if self.active.is_none() {
                    self.active = Some(*pad);
                    self.log(format!("connected: {name}"));
                } else {
                    self.log(format!(
                        "connected, not used (one controller plays): {name}"
                    ));
                }
                return false;
            }
            RawEvent::Disconnected { pad } => {
                if self.active != Some(*pad) {
                    return false;
                }
                self.log("disconnected: notes released".into());
                self.down.clear();
                self.sticks = [0.0; 4];
                self.triggers = [0.0; 2];
                self.active = self.input.pads().first().map(|p| p.id);
                return true;
            }
            _ if self.active != Some(event.pad()) => return false,
            RawEvent::Button {
                button, pressed, ..
            } => {
                if *pressed {
                    self.down.insert(*button);
                } else {
                    self.down.remove(button);
                }
                self.log(event.to_string());
            }
            RawEvent::Trigger { button, value, .. } => {
                self.triggers[usize::from(*button == Btn::R2)] = *value;
            }
            RawEvent::Axis { axis, value, .. } => {
                self.sticks[match axis {
                    Ax::LeftX => 0,
                    Ax::LeftY => 1,
                    Ax::RightX => 2,
                    Ax::RightY => 3,
                }] = *value;
            }
        }
        true
    }
}

impl App {
    /// Runs every frame: feeds the controller's events to the mapper and plays the result.
    pub(super) fn controller_frame(&mut self, ctx: &egui::Context) {
        let target = self.live_target();
        let drum_track = target.is_some_and(|t| {
            self.session.project().tracks.get(t).is_some_and(|t| {
                t.instrument.as_ref().and_then(Device::builtin_id) == Some("drums")
            })
        });
        let (key, scale) = {
            let p = self.session.project();
            (p.key % 12, music::scale_by_id(&p.scale))
        };
        let layout = if drum_track {
            Layout::Drums {
                first_pad: drums::FIRST_PAD,
            }
        } else {
            Layout::Scale {
                key,
                steps: scale.steps,
            }
        };

        // A different track (or kind of track) is a different instrument: let go first.
        if self.controller.last_target != target {
            let mut out = Vec::new();
            self.controller.mapper.release_all(&mut out);
            let old = self.controller.last_target;
            self.run_controller_actions(old, out);
            self.controller.last_target = target;
        }

        for event in self.controller.input.events() {
            if self.controller.observe(&event) {
                let mut out = Vec::new();
                self.controller.mapper.handle(&event, &layout, &mut out);
                self.run_controller_actions(target, out);
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(self.controller.last_tick).as_secs_f32();
        self.controller.last_tick = now;
        let mut out = Vec::new();
        self.controller.mapper.tick(dt, &mut out);
        self.run_controller_actions(target, out);
        if self.controller.mapper.is_settling() {
            ctx.request_repaint();
        }
    }

    fn run_controller_actions(&mut self, track: Option<usize>, actions: Vec<Action>) {
        let channel = self.controller.mapper.profile.channel;
        for action in actions {
            if let Some(bytes) = midi::message(&action, channel) {
                let applied = matches!(action, Action::NoteOn { .. } | Action::NoteOff { .. });
                let note = if applied {
                    ""
                } else {
                    "   (shown only: the engine has no live control for this yet)"
                };
                self.controller
                    .log(format!("{}{note}", midi::describe(bytes)));
            }
            match action {
                Action::NoteOn { pitch, velocity } => match track {
                    Some(t) => self.play_note(t, pitch, velocity, true),
                    None => {
                        self.status =
                            "The controller needs a track with an instrument: select or arm one."
                                .into();
                    }
                },
                Action::NoteOff { pitch } => {
                    if let Some(t) = track {
                        self.play_note(t, pitch, 0.0, false);
                    }
                }
                Action::OctaveChanged(octave) => {
                    self.controller.log(format!("octave: root is C{octave}"));
                    self.save_controller_profile();
                }
                Action::TogglePlay => self.toggle_play(),
                Action::ToggleRecord => self.toggle_record(),
                Action::Cc { .. } | Action::PitchBend { .. } => {}
            }
        }
    }

    fn save_controller_profile(&mut self) {
        if let Err(e) = self.controller.save() {
            self.status = format!("Can't save the controller settings: {e}");
        }
    }

    /// The controller settings window.
    pub(super) fn controller_window(&mut self, ctx: &egui::Context) {
        if !self.controller.window_open {
            return;
        }
        let target_name = self
            .live_target()
            .and_then(|t| self.session.project().tracks.get(t).map(|t| t.name.clone()));
        let pads = self.controller.input.pads();
        let error = self.controller.input.error();
        let mut open = true;
        let mut changed = false;
        let mut out = Vec::new();
        let c = &mut self.controller;
        egui::Window::new("Controller")
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                // Status.
                if let Some(e) = &error {
                    ui.colored_label(theme::accent(), format!("Can't read controllers: {e}"));
                    ui.label(
                        egui::RichText::new(
                            "On Linux your user needs read access to /dev/input/event* (see docs/CONTROLLER.md).",
                        )
                        .color(theme::text_dim()),
                    );
                } else if pads.is_empty() {
                    ui.label("No controller. Plug the DualShock 4 in with a USB data cable.");
                } else {
                    for p in &pads {
                        let mark = if c.active == Some(p.id) { "●" } else { "○" };
                        ui.label(format!("{mark} {}", p.label()));
                    }
                }
                match &target_name {
                    Some(n) => ui.label(
                        egui::RichText::new(format!("Plays: {n}")).color(theme::text_dim()),
                    ),
                    None => ui.label(
                        egui::RichText::new("Select or arm a track with an instrument to hear it.")
                            .color(theme::accent()),
                    ),
                };
                ui.add_space(4.0);

                let mut enabled = c.mapper.profile.enabled;
                if ui.checkbox(&mut enabled, "Controller plays").changed() {
                    c.mapper.set_enabled(enabled, &mut out);
                    changed = true;
                }
                egui::Grid::new("controller-settings")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        let p = &mut c.mapper.profile;
                        ui.label("MIDI channel").on_hover_text(
                            "The channel shown on the monitor's messages. Notes go straight to the track.",
                        );
                        changed |= ui
                            .add(egui::DragValue::new(&mut p.channel).range(1..=16))
                            .changed();
                        ui.end_row();
                        ui.label("Octave (root)");
                        changed |= ui.add(egui::Slider::new(&mut p.octave, -1..=8)).changed();
                        ui.end_row();
                        ui.label("Velocity").on_hover_text("Used when L2 isn't pulled");
                        changed |= ui.add(egui::Slider::new(&mut p.velocity, 0.05..=1.0)).changed();
                        ui.end_row();
                        ui.label("Stick dead zone");
                        changed |= ui.add(egui::Slider::new(&mut p.dead_zone, 0.0..=0.5)).changed();
                        ui.end_row();
                        ui.label("Smoothing");
                        changed |= ui.add(egui::Slider::new(&mut p.smoothing, 0.0..=0.95)).changed();
                        ui.end_row();
                    });
                ui.add_space(6.0);

                // Live activity.
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for (button, label) in BUTTON_LABELS {
                        let lit = c.down.contains(&button);
                        let text = egui::RichText::new(label).monospace();
                        let text = if lit {
                            text.color(theme::bg()).background_color(theme::accent())
                        } else {
                            text.color(theme::text_dim())
                        };
                        ui.label(text);
                    }
                });
                ui.horizontal(|ui| {
                    stick(ui, c.sticks[0], c.sticks[1], "L");
                    stick(ui, c.sticks[2], c.sticks[3], "R");
                    ui.vertical(|ui| {
                        ui.add(egui::ProgressBar::new(c.triggers[0]).desired_width(120.0).text("L2"));
                        ui.add(egui::ProgressBar::new(c.triggers[1]).desired_width(120.0).text("R2"));
                    });
                });
                ui.add_space(4.0);

                // Mapping.
                ui.separator();
                ui.label(
                    egui::RichText::new(
                        "Notes: the buttons below play the song's scale from its root. \
                         L1/R1 octave, L2 velocity, Options play/stop, Share record.",
                    )
                    .color(theme::text_dim()),
                );
                ui.label(
                    egui::RichText::new(
                        c.mapper
                            .profile
                            .note_buttons
                            .iter()
                            .enumerate()
                            .map(|(i, b)| format!("{b:?}={}", i + 1))
                            .collect::<Vec<_>>()
                            .join("  "),
                    )
                    .monospace(),
                );
                ui.label(
                    egui::RichText::new(
                        "Sticks: left X pitch bend, left Y expression, right X cutoff, right Y resonance, R2 modulation (shown in the monitor; not applied to sound yet).",
                    )
                    .color(theme::text_dim()),
                );
                ui.horizontal(|ui| {
                    if ui.button("Reset to defaults").clicked() {
                        c.mapper.release_all(&mut out);
                        c.mapper.profile = Profile::default();
                        changed = true;
                    }
                    if ui.button("Clear monitor").clicked() {
                        c.log.clear();
                    }
                });

                // Monitor.
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &c.log {
                            ui.label(egui::RichText::new(line).monospace().size(11.0));
                        }
                    });
            });
        self.controller.window_open = open;
        let target = self.controller.last_target;
        self.run_controller_actions(target, out);
        if changed {
            self.save_controller_profile();
        }
    }
}

const BUTTON_LABELS: [(Btn, &str); 17] = [
    (Btn::Cross, "✕"),
    (Btn::Circle, "○"),
    (Btn::Square, "□"),
    (Btn::Triangle, "△"),
    (Btn::DpadLeft, "←"),
    (Btn::DpadUp, "↑"),
    (Btn::DpadRight, "→"),
    (Btn::DpadDown, "↓"),
    (Btn::L1, "L1"),
    (Btn::R1, "R1"),
    (Btn::L2, "L2"),
    (Btn::R2, "R2"),
    (Btn::L3, "L3"),
    (Btn::R3, "R3"),
    (Btn::Options, "OPT"),
    (Btn::Share, "SHR"),
    (Btn::Ps, "PS"),
];

/// A stick's position as a dot in a square.
fn stick(ui: &mut egui::Ui, x: f32, y: f32, label: &str) {
    let size = 56.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::lane());
    painter.line_segment(
        [rect.center_top(), rect.center_bottom()],
        egui::Stroke::new(1.0, theme::grid()),
    );
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        egui::Stroke::new(1.0, theme::grid()),
    );
    let dot = rect.center() + egui::vec2(x, -y) * (size / 2.0 - 4.0);
    painter.circle_filled(dot, 4.0, theme::accent());
    painter.text(
        rect.left_top() + egui::vec2(3.0, 1.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::monospace(10.0),
        theme::text_dim(),
    );
}
