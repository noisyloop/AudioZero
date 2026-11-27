//! The MIDI messages an [`Action`] stands for. AudioZero plays controller notes directly rather
//! than through a MIDI port; this is for the monitor and the diagnostic tool, and for any future
//! output to a real MIDI port.

use crate::mapper::Action;

/// A three-byte MIDI message on `channel` (1 to 16), if the action is one.
pub fn message(action: &Action, channel: u8) -> Option<[u8; 3]> {
    let ch = channel.clamp(1, 16) - 1;
    let v7 = |v: f32| (v.clamp(0.0, 1.0) * 127.0).round() as u8;
    Some(match *action {
        Action::NoteOn { pitch, velocity } => [0x90 | ch, pitch & 0x7f, v7(velocity).max(1)],
        Action::NoteOff { pitch } => [0x80 | ch, pitch & 0x7f, 0],
        Action::Cc { controller, value } => [0xB0 | ch, controller & 0x7f, value & 0x7f],
        Action::PitchBend { value } => {
            let v = value.min(0x3fff);
            [0xE0 | ch, (v & 0x7f) as u8, (v >> 7) as u8]
        }
        _ => return None,
    })
}

/// "90 3C 66  note on C4 vel 102", for a log.
pub fn describe(bytes: [u8; 3]) -> String {
    let [s, a, b] = bytes;
    let ch = (s & 0x0f) + 1;
    let what = match s & 0xf0 {
        0x90 => format!("note on {} vel {b}", note_name(a)),
        0x80 => format!("note off {}", note_name(a)),
        0xB0 => format!("CC {a} = {b}"),
        0xE0 => format!("pitch bend {}", i32::from(a) | (i32::from(b) << 7)),
        _ => "?".into(),
    };
    format!("{s:02X} {a:02X} {b:02X}  ch{ch} {what}")
}

/// "C4" for 60.
pub fn note_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(pitch % 12)],
        i32::from(pitch / 12) - 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_have_the_right_bytes() {
        let on = Action::NoteOn {
            pitch: 60,
            velocity: 1.0,
        };
        assert_eq!(message(&on, 1), Some([0x90, 60, 127]));
        assert_eq!(message(&on, 16), Some([0x9F, 60, 127]));
        assert_eq!(
            message(&Action::NoteOff { pitch: 60 }, 2),
            Some([0x81, 60, 0])
        );
        assert_eq!(
            message(
                &Action::Cc {
                    controller: 1,
                    value: 64
                },
                1
            ),
            Some([0xB0, 1, 64])
        );
        // Centre is 8192 = 0x2000: LSB 0, MSB 0x40.
        assert_eq!(
            message(&Action::PitchBend { value: 8192 }, 1),
            Some([0xE0, 0x00, 0x40])
        );
        assert_eq!(message(&Action::TogglePlay, 1), None);
    }

    #[test]
    fn a_zero_velocity_note_on_never_means_note_off() {
        let quiet = Action::NoteOn {
            pitch: 60,
            velocity: 0.0,
        };
        assert_eq!(message(&quiet, 1).unwrap()[2], 1);
    }

    #[test]
    fn names() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert!(describe([0x90, 60, 100]).contains("note on C4 vel 100"));
    }
}
