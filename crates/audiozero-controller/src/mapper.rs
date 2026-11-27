//! Controller events in, musical actions out. No device, no audio, no UI: just state, so it is
//! tested without a controller.
//!
//! Default layout (see [`Profile::note_buttons`]): Cross, Circle, Square, Triangle and the D-pad
//! play the scale from its root up (C D E F G A B C in C major, and the song's key and scale
//! otherwise). L1 and R1 shift the octave, L2 sets the velocity of the next notes, Options is
//! play/stop and Share is record. The sticks and R2 are continuous controls.

use crate::input::{Ax, Btn, RawEvent};
use crate::profile::Profile;

/// CC numbers for the analog controls (standard MIDI assignments).
pub const CC_MODULATION: u8 = 1;
pub const CC_EXPRESSION: u8 = 11;
pub const CC_RESONANCE: u8 = 71;
pub const CC_CUTOFF: u8 = 74;
/// Pitch bend's resting value.
pub const BEND_CENTRE: u16 = 8192;

/// Trigger travel below this counts as not pulled.
const TRIGGER_DEAD_ZONE: f32 = 0.05;
/// The quietest note L2 can ask for.
const MIN_VELOCITY: f32 = 0.1;
const OCTAVE_RANGE: std::ops::RangeInclusive<i8> = -1..=8;

/// What the note buttons play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout<'a> {
    /// The notes of a scale: the key's root (0 = C) and the scale's steps above it.
    Scale { key: u8, steps: &'a [u8] },
    /// A drum rack: the first pad's note, one pad per button.
    Drums { first_pad: u8 },
}

/// What the controller asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    NoteOn {
        pitch: u8,
        velocity: f32,
    },
    NoteOff {
        pitch: u8,
    },
    /// The octave changed (held notes were released first).
    OctaveChanged(i8),
    TogglePlay,
    ToggleRecord,
    /// A continuous control, 0 to 127 (modulation, expression, filter cutoff, resonance).
    Cc {
        controller: u8,
        value: u8,
    },
    /// 0 to 16383, [`BEND_CENTRE`] at rest.
    PitchBend {
        value: u16,
    },
}

/// Indexes into the continuous controls.
const BEND: usize = 0;
const EXPRESSION: usize = 1;
const CUTOFF: usize = 2;
const RESONANCE: usize = 3;
const MODULATION: usize = 4;
/// Where each control rests: bend at its centre, the sticks mid-way, modulation at nothing.
const REST: [f32; 5] = [0.0, 0.0, 0.0, 0.0, 0.0];

pub struct Mapper {
    pub profile: Profile,
    /// The notes held, by the button that started them (so a later octave change can't leave
    /// one stuck).
    held: Vec<(Btn, u8)>,
    l2: f32,
    /// Dead-zoned control values the controls are heading for: sticks -1 to 1, R2 0 to 1.
    target: [f32; 5],
    /// The smoothed values.
    value: [f32; 5],
    /// What was last sent for each control, to send changes only.
    sent: [u16; 5],
}

impl Mapper {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile: profile.sanitized(),
            held: Vec::new(),
            l2: 0.0,
            target: REST,
            value: REST,
            sent: [BEND_CENTRE, 64, 64, 64, 0],
        }
    }

    pub fn octave(&self) -> i8 {
        self.profile.octave
    }

    /// The pitches held now.
    pub fn held_pitches(&self) -> impl Iterator<Item = u8> + '_ {
        self.held.iter().map(|&(_, p)| p)
    }

    /// Whether a control is still moving towards its target (the app should keep calling
    /// [`Mapper::tick`]).
    pub fn is_settling(&self) -> bool {
        self.value
            .iter()
            .zip(&self.target)
            .any(|(v, t)| (v - t).abs() > f32::EPSILON)
    }

    /// Releases every held note and recentres the controls. Call it when the controller goes
    /// away, input is switched off, or the target changes.
    pub fn release_all(&mut self, out: &mut Vec<Action>) {
        for (_, pitch) in self.held.drain(..) {
            out.push(Action::NoteOff { pitch });
        }
        self.l2 = 0.0;
        self.target = REST;
        self.value = REST;
        self.emit_controls(out);
    }

    /// Turns the controller on or off; off releases everything held.
    pub fn set_enabled(&mut self, enabled: bool, out: &mut Vec<Action>) {
        if self.profile.enabled && !enabled {
            self.release_all(out);
        }
        self.profile.enabled = enabled;
    }

    /// Handles one event.
    pub fn handle(&mut self, event: &RawEvent, layout: &Layout, out: &mut Vec<Action>) {
        match *event {
            RawEvent::Disconnected { .. } => self.release_all(out),
            RawEvent::Connected { .. } => {}
            _ if !self.profile.enabled => {}
            RawEvent::Button {
                button, pressed, ..
            } => self.button(button, pressed, layout, out),
            RawEvent::Trigger { button, value, .. } => {
                let v = dead_zone(value.clamp(0.0, 1.0), TRIGGER_DEAD_ZONE);
                match button {
                    Btn::L2 => self.l2 = v,
                    Btn::R2 => self.set_target(MODULATION, v, out),
                    _ => {}
                }
            }
            RawEvent::Axis { axis, value, .. } => {
                let v = dead_zone(value.clamp(-1.0, 1.0), self.profile.dead_zone);
                let index = match axis {
                    Ax::LeftX => BEND,
                    Ax::LeftY => EXPRESSION,
                    Ax::RightX => CUTOFF,
                    Ax::RightY => RESONANCE,
                };
                self.set_target(index, v, out);
            }
        }
    }

    /// Moves the smoothed controls on by `dt` seconds. Call it every frame.
    pub fn tick(&mut self, dt: f32, out: &mut Vec<Action>) {
        let s = self.profile.smoothing;
        if s <= 0.0 {
            return;
        }
        // The same smoothness at any frame rate.
        let alpha = 1.0 - s.powf((dt * 60.0).clamp(0.0, 10.0));
        for i in 0..5 {
            let v = self.value[i] + (self.target[i] - self.value[i]) * alpha;
            self.value[i] = if (v - self.target[i]).abs() < 0.002 {
                self.target[i]
            } else {
                v
            };
        }
        self.emit_controls(out);
    }

    fn set_target(&mut self, index: usize, v: f32, out: &mut Vec<Action>) {
        self.target[index] = v;
        if self.profile.smoothing <= 0.0 {
            self.value[index] = v;
            self.emit_controls(out);
        }
    }

    fn emit_controls(&mut self, out: &mut Vec<Action>) {
        let bipolar = |v: f32| (((v + 1.0) / 2.0) * 127.0).round().clamp(0.0, 127.0) as u16;
        let now = [
            (BEND_CENTRE as f32 + self.value[BEND] * 8191.0)
                .round()
                .clamp(0.0, 16383.0) as u16,
            bipolar(self.value[EXPRESSION]),
            bipolar(self.value[CUTOFF]),
            bipolar(self.value[RESONANCE]),
            (self.value[MODULATION] * 127.0).round().clamp(0.0, 127.0) as u16,
        ];
        for (i, &v) in now.iter().enumerate() {
            if v == self.sent[i] {
                continue;
            }
            self.sent[i] = v;
            out.push(match i {
                BEND => Action::PitchBend { value: v },
                EXPRESSION => Action::Cc {
                    controller: CC_EXPRESSION,
                    value: v as u8,
                },
                CUTOFF => Action::Cc {
                    controller: CC_CUTOFF,
                    value: v as u8,
                },
                RESONANCE => Action::Cc {
                    controller: CC_RESONANCE,
                    value: v as u8,
                },
                _ => Action::Cc {
                    controller: CC_MODULATION,
                    value: v as u8,
                },
            });
        }
    }

    fn velocity(&self) -> f32 {
        if self.l2 > 0.0 {
            MIN_VELOCITY + (1.0 - MIN_VELOCITY) * self.l2
        } else {
            self.profile.velocity
        }
    }

    fn button(&mut self, button: Btn, pressed: bool, layout: &Layout, out: &mut Vec<Action>) {
        if let Some(index) = self.profile.note_buttons.iter().position(|&b| b == button) {
            if pressed {
                if self.held.iter().any(|&(b, _)| b == button) {
                    return;
                }
                if let Some(pitch) = self.pitch(index, layout) {
                    self.held.push((button, pitch));
                    out.push(Action::NoteOn {
                        pitch,
                        velocity: self.velocity(),
                    });
                }
            } else if let Some(at) = self.held.iter().position(|&(b, _)| b == button) {
                let (_, pitch) = self.held.remove(at);
                out.push(Action::NoteOff { pitch });
            }
            return;
        }
        if !pressed {
            return;
        }
        match button {
            Btn::L1 => self.shift_octave(-1, out),
            Btn::R1 => self.shift_octave(1, out),
            Btn::Options => out.push(Action::TogglePlay),
            Btn::Share => out.push(Action::ToggleRecord),
            _ => {}
        }
    }

    fn shift_octave(&mut self, by: i8, out: &mut Vec<Action>) {
        let octave = (self.profile.octave + by).clamp(*OCTAVE_RANGE.start(), *OCTAVE_RANGE.end());
        if octave == self.profile.octave {
            return;
        }
        // Notes started in the old octave end now, so none is left hanging.
        for (_, pitch) in self.held.drain(..) {
            out.push(Action::NoteOff { pitch });
        }
        self.profile.octave = octave;
        out.push(Action::OctaveChanged(octave));
    }

    /// The pitch of the `index`th note button, if it fits in MIDI's range.
    fn pitch(&self, index: usize, layout: &Layout) -> Option<u8> {
        let pitch = match *layout {
            Layout::Scale { key, steps } => {
                let n = steps.len();
                if n == 0 {
                    return None;
                }
                // Past the scale's last note the buttons continue an octave up.
                let semitones = i32::from(steps[index % n]) + 12 * (index / n) as i32;
                (i32::from(self.profile.octave) + 1) * 12 + i32::from(key % 12) + semitones
            }
            Layout::Drums { first_pad } => i32::from(first_pad) + index as i32,
        };
        u8::try_from(pitch).ok().filter(|&p| p <= 127)
    }
}

/// Stick travel inside `zone` is nothing; outside it the rest is stretched over 0 to 1, so
/// there's no jump at the edge.
pub fn dead_zone(v: f32, zone: f32) -> f32 {
    let mag = v.abs();
    if mag <= zone {
        0.0
    } else {
        v.signum() * ((mag - zone) / (1.0 - zone)).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
    const MINOR_PENTATONIC: [u8; 5] = [0, 3, 5, 7, 10];

    fn c_major() -> Layout<'static> {
        Layout::Scale {
            key: 0,
            steps: &MAJOR,
        }
    }

    fn press(m: &mut Mapper, layout: &Layout, button: Btn, pressed: bool) -> Vec<Action> {
        let mut out = Vec::new();
        m.handle(
            &RawEvent::Button {
                pad: 0,
                button,
                pressed,
                value: 1.0,
            },
            layout,
            &mut out,
        );
        out
    }

    fn axis(m: &mut Mapper, axis: Ax, value: f32) -> Vec<Action> {
        let mut out = Vec::new();
        m.handle(
            &RawEvent::Axis {
                pad: 0,
                axis,
                value,
            },
            &c_major(),
            &mut out,
        );
        out
    }

    fn trigger(m: &mut Mapper, button: Btn, value: f32) {
        m.handle(
            &RawEvent::Trigger {
                pad: 0,
                button,
                value,
            },
            &c_major(),
            &mut Vec::new(),
        );
    }

    fn mapper() -> Mapper {
        Mapper::new(Profile {
            smoothing: 0.0,
            ..Profile::default()
        })
    }

    fn pitch_of(actions: &[Action]) -> Vec<u8> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::NoteOn { pitch, .. } | Action::NoteOff { pitch } => Some(*pitch),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_buttons_play_c_major_from_middle_c() {
        let mut m = mapper();
        let l = c_major();
        let played: Vec<u8> = DEFAULT_ORDER
            .iter()
            .map(|&b| pitch_of(&press(&mut m, &l, b, true))[0])
            .collect();
        assert_eq!(played, [60, 62, 64, 65, 67, 69, 71, 72]);
    }

    const DEFAULT_ORDER: [Btn; 8] = crate::profile::DEFAULT_NOTE_BUTTONS;

    #[test]
    fn notes_follow_the_songs_key_and_scale() {
        let mut m = mapper();
        // A minor pentatonic: the 6th button wraps to the next octave.
        let l = Layout::Scale {
            key: 9,
            steps: &MINOR_PENTATONIC,
        };
        let played: Vec<u8> = DEFAULT_ORDER
            .iter()
            .map(|&b| pitch_of(&press(&mut m, &l, b, true))[0])
            .collect();
        assert_eq!(played, [69, 72, 74, 76, 79, 81, 84, 86]);
    }

    #[test]
    fn button_up_ends_the_note_and_chords_hold_together() {
        let mut m = mapper();
        let l = c_major();
        press(&mut m, &l, Btn::Cross, true);
        press(&mut m, &l, Btn::Square, true);
        press(&mut m, &l, Btn::DpadLeft, true);
        let mut held: Vec<u8> = m.held_pitches().collect();
        held.sort_unstable();
        assert_eq!(held, [60, 64, 67]);
        assert_eq!(
            press(&mut m, &l, Btn::Square, false),
            [Action::NoteOff { pitch: 64 }]
        );
        assert_eq!(m.held_pitches().count(), 2);
    }

    #[test]
    fn a_repeated_press_or_stray_release_does_nothing() {
        let mut m = mapper();
        let l = c_major();
        assert_eq!(press(&mut m, &l, Btn::Cross, true).len(), 1);
        assert!(press(&mut m, &l, Btn::Cross, true).is_empty());
        assert_eq!(press(&mut m, &l, Btn::Cross, false).len(), 1);
        assert!(press(&mut m, &l, Btn::Cross, false).is_empty());
    }

    #[test]
    fn octave_buttons_shift_and_release_held_notes() {
        let mut m = mapper();
        let l = c_major();
        press(&mut m, &l, Btn::Cross, true);
        let out = press(&mut m, &l, Btn::R1, true);
        assert_eq!(
            out,
            [Action::NoteOff { pitch: 60 }, Action::OctaveChanged(5)]
        );
        // The button is still down, but letting go is harmless.
        assert!(press(&mut m, &l, Btn::Cross, false).is_empty());
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::Cross, true)), [72]);
        press(&mut m, &l, Btn::L1, true);
        press(&mut m, &l, Btn::L1, true);
        assert_eq!(m.octave(), 3);
    }

    #[test]
    fn octave_stops_at_the_ends_and_unplayable_notes_are_silent() {
        let mut m = mapper();
        let l = c_major();
        for _ in 0..20 {
            press(&mut m, &l, Btn::R1, true);
        }
        assert_eq!(m.octave(), 8);
        // C8 is 108 and the top button 120: both fit.
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::DpadDown, true)), [120]);
        press(&mut m, &l, Btn::DpadDown, false);
        // In B, the top button would be 131: past MIDI's 127, so it stays silent.
        let b_major = Layout::Scale {
            key: 11,
            steps: &MAJOR,
        };
        assert!(press(&mut m, &b_major, Btn::DpadDown, true).is_empty());
        for _ in 0..20 {
            press(&mut m, &l, Btn::L1, true);
        }
        assert_eq!(m.octave(), -1);
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::Cross, true)), [0]);
    }

    #[test]
    fn l2_sets_velocity_and_released_l2_uses_the_default() {
        let mut m = mapper();
        let l = c_major();
        let velocity = |actions: &[Action]| match actions[0] {
            Action::NoteOn { velocity, .. } => velocity,
            _ => panic!("not a note"),
        };
        // Not pulled: the profile's velocity, never silence.
        assert_eq!(velocity(&press(&mut m, &l, Btn::Cross, true)), 0.8);
        press(&mut m, &l, Btn::Cross, false);
        trigger(&mut m, Btn::L2, 1.0);
        assert_eq!(velocity(&press(&mut m, &l, Btn::Cross, true)), 1.0);
        press(&mut m, &l, Btn::Cross, false);
        trigger(&mut m, Btn::L2, 0.5);
        let half = velocity(&press(&mut m, &l, Btn::Cross, true));
        assert!(half > 0.4 && half < 0.7, "{half}");
        press(&mut m, &l, Btn::Cross, false);
        trigger(&mut m, Btn::L2, 0.0);
        assert_eq!(velocity(&press(&mut m, &l, Btn::Cross, true)), 0.8);
    }

    #[test]
    fn options_and_share_are_transport() {
        let mut m = mapper();
        let l = c_major();
        assert_eq!(press(&mut m, &l, Btn::Options, true), [Action::TogglePlay]);
        assert!(press(&mut m, &l, Btn::Options, false).is_empty());
        assert_eq!(press(&mut m, &l, Btn::Share, true), [Action::ToggleRecord]);
    }

    #[test]
    fn dead_zone_hides_drift_and_has_no_jump_at_the_edge() {
        assert_eq!(dead_zone(0.1, 0.12), 0.0);
        assert_eq!(dead_zone(-0.12, 0.12), 0.0);
        assert!(dead_zone(0.13, 0.12) < 0.02);
        assert_eq!(dead_zone(1.0, 0.12), 1.0);
        assert_eq!(dead_zone(-1.0, 0.12), -1.0);
        let mut m = mapper();
        assert!(axis(&mut m, Ax::LeftX, 0.08).is_empty(), "drift is ignored");
        assert_eq!(
            axis(&mut m, Ax::LeftX, 1.0),
            [Action::PitchBend { value: 16383 }]
        );
        assert_eq!(
            axis(&mut m, Ax::LeftX, -1.0),
            [Action::PitchBend { value: 1 }]
        );
        assert_eq!(
            axis(&mut m, Ax::LeftX, 0.0),
            [Action::PitchBend { value: BEND_CENTRE }]
        );
    }

    #[test]
    fn the_analog_controls_map_to_their_ccs() {
        let mut m = mapper();
        let cc = |controller, value| Action::Cc { controller, value };
        assert_eq!(axis(&mut m, Ax::LeftY, 1.0), [cc(CC_EXPRESSION, 127)]);
        assert_eq!(axis(&mut m, Ax::RightX, 1.0), [cc(CC_CUTOFF, 127)]);
        assert_eq!(axis(&mut m, Ax::RightY, -1.0), [cc(CC_RESONANCE, 0)]);
        let mut out = Vec::new();
        m.handle(
            &RawEvent::Trigger {
                pad: 0,
                button: Btn::R2,
                value: 1.0,
            },
            &c_major(),
            &mut out,
        );
        assert_eq!(out, [cc(CC_MODULATION, 127)]);
    }

    #[test]
    fn only_changes_are_sent() {
        let mut m = mapper();
        assert_eq!(axis(&mut m, Ax::RightX, 0.5).len(), 1);
        assert!(axis(&mut m, Ax::RightX, 0.5).is_empty());
    }

    #[test]
    fn smoothing_glides_to_the_target_and_settles_at_rest() {
        let mut m = Mapper::new(Profile {
            smoothing: 0.8,
            ..Profile::default()
        });
        assert!(
            axis(&mut m, Ax::LeftX, 1.0).is_empty(),
            "moves on the next tick"
        );
        let mut out = Vec::new();
        m.tick(1.0 / 60.0, &mut out);
        let Some(Action::PitchBend { value: first }) = out.first().copied() else {
            panic!("no bend after a tick");
        };
        assert!(first > BEND_CENTRE && first < 16383, "{first}");
        for _ in 0..600 {
            m.tick(1.0 / 60.0, &mut out);
        }
        assert_eq!(out.last(), Some(&Action::PitchBend { value: 16383 }));
        assert!(!m.is_settling());
        // Letting go returns to exactly the centre, not "nearly".
        axis(&mut m, Ax::LeftX, 0.0);
        out.clear();
        for _ in 0..600 {
            m.tick(1.0 / 60.0, &mut out);
        }
        assert_eq!(out.last(), Some(&Action::PitchBend { value: BEND_CENTRE }));
    }

    #[test]
    fn disconnecting_releases_every_note_and_recentres() {
        let mut m = mapper();
        let l = c_major();
        press(&mut m, &l, Btn::Cross, true);
        press(&mut m, &l, Btn::Circle, true);
        axis(&mut m, Ax::LeftX, 1.0);
        trigger(&mut m, Btn::L2, 1.0);
        let mut out = Vec::new();
        m.handle(&RawEvent::Disconnected { pad: 0 }, &l, &mut out);
        assert_eq!(
            out,
            [
                Action::NoteOff { pitch: 60 },
                Action::NoteOff { pitch: 62 },
                Action::PitchBend { value: BEND_CENTRE },
            ]
        );
        assert_eq!(m.held_pitches().count(), 0);
        // And after reconnecting, a velocity left over from before is gone.
        let out = press(&mut m, &l, Btn::Cross, true);
        assert_eq!(
            out,
            [Action::NoteOn {
                pitch: 60,
                velocity: 0.8
            }]
        );
    }

    #[test]
    fn disabling_releases_notes_and_ignores_input() {
        let mut m = mapper();
        let l = c_major();
        press(&mut m, &l, Btn::Cross, true);
        let mut out = Vec::new();
        m.set_enabled(false, &mut out);
        assert_eq!(out, [Action::NoteOff { pitch: 60 }]);
        assert!(press(&mut m, &l, Btn::Cross, true).is_empty());
        assert!(axis(&mut m, Ax::LeftX, 1.0).is_empty());
        m.set_enabled(true, &mut Vec::new());
        assert_eq!(press(&mut m, &l, Btn::Cross, true).len(), 1);
    }

    #[test]
    fn drum_racks_get_one_pad_per_button_whatever_the_octave() {
        let mut m = mapper();
        let l = Layout::Drums { first_pad: 36 };
        press(&mut m, &l, Btn::R1, true);
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::Cross, true)), [36]);
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::Circle, true)), [37]);
    }

    #[test]
    fn remapped_buttons_play_in_the_order_given() {
        let mut m = Mapper::new(Profile {
            note_buttons: vec![Btn::Triangle, Btn::Cross],
            smoothing: 0.0,
            ..Profile::default()
        });
        let l = c_major();
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::Triangle, true)), [60]);
        assert_eq!(pitch_of(&press(&mut m, &l, Btn::Cross, true)), [62]);
    }
}
