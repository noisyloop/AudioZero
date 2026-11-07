//! A short built-in song, so there's something to hear before any project exists.

use crate::project::{Clip, Device, Note, Project, Track};

pub fn project() -> Project {
    // Am - F - C - G, two bars each, over eight bars.
    let chords: [[u8; 3]; 4] = [[57, 60, 64], [53, 57, 60], [55, 60, 64], [55, 59, 62]];
    let roots: [u8; 4] = [33, 29, 36, 31];
    let sine = || Some(Device::Builtin { id: "sine".into() });

    let kick_notes = (0..32)
        .map(|beat| Note {
            pitch: 36,
            start: f64::from(beat),
            length: 0.25,
            velocity: 0.9,
        })
        .collect();

    let mut chord_notes = Vec::new();
    let mut bass_notes = Vec::new();
    for bar in 0..8u32 {
        let index = (bar / 2) as usize % 4;
        let at = f64::from(bar) * 4.0;
        for (position, length, velocity) in [(0.0, 1.4, 0.85), (1.5, 0.45, 0.6), (2.5, 1.2, 0.75)] {
            for &pitch in &chords[index] {
                chord_notes.push(Note {
                    pitch,
                    start: at + position,
                    length,
                    velocity,
                });
            }
        }
        for beat in 0..4 {
            let pitch = if beat == 3 {
                roots[index] + 12
            } else {
                roots[index]
            };
            bass_notes.push(Note {
                pitch,
                start: at + f64::from(beat) + 0.5,
                length: 0.4,
                velocity: 0.9,
            });
        }
    }

    Project {
        bpm: 110.0,
        loop_enabled: false,
        loop_start: 0.0,
        loop_end: 32.0,
        master_volume: 0.8,
        tracks: vec![
            Track {
                name: "Chords".into(),
                volume: 0.6,
                pan: -0.2,
                instrument: sine(),
                clips: vec![Clip {
                    name: "Chords".into(),
                    start: 0.0,
                    length: 32.0,
                    muted: false,
                    notes: chord_notes,
                }],
                ..Default::default()
            },
            Track {
                name: "Bass".into(),
                volume: 0.8,
                pan: 0.1,
                instrument: sine(),
                clips: vec![Clip {
                    name: "Bass".into(),
                    start: 0.0,
                    length: 32.0,
                    muted: false,
                    notes: bass_notes,
                }],
                ..Default::default()
            },
            // Nusick's Kick Me, built in.
            Track {
                name: "Kick".into(),
                volume: 0.8,
                instrument: Some(Device::Builtin { id: "kick".into() }),
                clips: vec![Clip {
                    name: "Kick".into(),
                    start: 0.0,
                    length: 32.0,
                    muted: false,
                    notes: kick_notes,
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
