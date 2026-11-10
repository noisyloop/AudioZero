//! Playing and recording: arming a track, the computer keyboard as a MIDI keyboard, MIDI input,
//! and turning takes into clips.
//!
//! Which take a track records depends on the track: a track with an instrument records MIDI, a
//! track without one records audio from the default input.

use super::App;
use crate::input::AudioIn;
use crate::record::{AudioTake, MidiTake, Take, take_file_name};
use audiozero_engine::{AudioRegion, Clip, Command, LiveNote};
use eframe::egui::{self, Key};

/// The computer keyboard's note keys, as semitones above the octave's C: the home row is the
/// white keys, the row above the black ones (as in most DAWs).
const NOTE_KEYS: [(Key, u8); 15] = [
    (Key::A, 0),
    (Key::W, 1),
    (Key::S, 2),
    (Key::E, 3),
    (Key::D, 4),
    (Key::F, 5),
    (Key::T, 6),
    (Key::G, 7),
    (Key::Y, 8),
    (Key::H, 9),
    (Key::U, 10),
    (Key::J, 11),
    (Key::K, 12),
    (Key::O, 13),
    (Key::L, 14),
];

/// The computer keyboard's state.
pub(super) struct Keys {
    /// The octave of the A key's C, as in MIDI (4 = middle C, 60).
    pub octave: i32,
    pub velocity: f32,
    /// Keys held down, with the pitch each one started.
    held: Vec<(Key, u8)>,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            octave: 4,
            velocity: 0.8,
            held: Vec::new(),
        }
    }
}

impl App {
    /// The track live notes play: the armed one, else the one being edited or selected, if it
    /// has an instrument.
    pub(super) fn live_target(&self) -> Option<usize> {
        let tracks = &self.session.project().tracks;
        self.armed
            .or(self.roll.as_ref().map(|r| r.clip.track))
            .or(self.selected_clip.map(|c| c.track))
            .filter(|&t| tracks.get(t).is_some_and(|t| t.instrument.is_some()))
    }

    pub(super) fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Arms a track for recording, or disarms it. Only one track is armed at a time.
    pub(super) fn toggle_arm(&mut self, track: usize) {
        if self.recording.as_ref().is_some_and(|t| t.track() == track) {
            self.stop_recording();
        }
        if self.armed == Some(track) {
            self.armed = None;
            self.audio_in = None;
            return;
        }
        let Some(t) = self.session.project().tracks.get(track) else {
            return;
        };
        self.armed = Some(track);
        if t.instrument.is_some() {
            self.audio_in = None;
            self.status = format!("\"{}\" is armed: play a MIDI keyboard or A–K.", t.name);
        } else {
            let name = t.name.clone();
            match AudioIn::open() {
                Ok(input) => {
                    self.status =
                        format!("\"{name}\" is armed to record from {}.", input.description);
                    self.audio_in = Some(input);
                }
                Err(e) => {
                    self.armed = None;
                    self.status = format!("Can't record audio: {e}");
                }
            }
        }
    }

    pub(super) fn toggle_record(&mut self) {
        if self.recording.is_some() {
            self.stop_recording();
            return;
        }
        let Some(track) = self.armed else {
            self.status = "Arm a track (●) to record on it.".into();
            return;
        };
        let Some(t) = self.session.project().tracks.get(track) else {
            self.armed = None;
            return;
        };
        let position = self.audio.controller.position();
        let take = if t.instrument.is_some() {
            Take::Midi(MidiTake::new(track, position))
        } else {
            let Some(input) = &mut self.audio_in else {
                self.status = "The audio input isn't open; arm the track again.".into();
                return;
            };
            input.discard();
            Take::Audio(AudioTake::new(track, position, input.rate))
        };
        self.status = format!("Recording on \"{}\"…", t.name);
        self.recording = Some(take);
        self.record_rolling = false;
        if !self.audio.controller.is_playing() {
            self.send(Command::Play);
        }
    }

    /// Ends the take and adds what it recorded as a clip, as one undo step.
    pub(super) fn stop_recording(&mut self) {
        let Some(take) = self.recording.take() else {
            return;
        };
        let end = self.audio.controller.position();
        match take {
            Take::Midi(take) => {
                let track = take.track;
                let bar = self.session.project().beats_per_bar;
                match take.finish(end, bar) {
                    Some(clip) => self.add_take(track, clip),
                    None => self.status = "Nothing was played, so nothing was recorded.".into(),
                }
            }
            Take::Audio(mut take) => {
                if let Some(input) = &mut self.audio_in {
                    input.drain(&mut take.left, &mut take.right);
                }
                match take.finish() {
                    Some((track, start, sample)) => self.save_audio_take(track, start, &sample),
                    None => self.status = "Nothing was recorded.".into(),
                }
            }
        }
    }

    fn add_take(&mut self, track: usize, clip: Clip) {
        let result = self.session.add_clip(track, clip);
        if let Some(at) = self.done(result) {
            self.selected_clip = Some(at);
            self.status = "Recorded. Undo removes the take.".into();
        }
    }

    fn save_audio_take(
        &mut self,
        track: usize,
        start: f64,
        sample: &audiozero_engine::sample::Sample,
    ) {
        let folder = crate::project_folder(self.file.as_deref())
            .unwrap_or_else(crate::default_folder)
            .join("Recordings");
        if let Err(e) = std::fs::create_dir_all(&folder) {
            self.status = format!("Can't save the recording in {}: {e}", folder.display());
            return;
        }
        let name = self
            .session
            .project()
            .tracks
            .get(track)
            .map_or("Track".into(), |t| t.name.clone());
        let path = take_file_name(&folder, &name);
        let written = std::fs::File::create(&path).and_then(|f| {
            audiozero_engine::wav::write_stereo_24(
                std::io::BufWriter::new(f),
                sample.rate().round() as u32,
                sample.left(),
                sample.right(),
            )
        });
        if let Err(e) = written {
            self.status = format!("Can't save the recording {}: {e}", path.display());
            return;
        }
        let file = self.audio_file_name(&path);
        self.audio.controller.samples().insert(&file, sample);
        let bpm = self.session.project().bpm;
        let clip = Clip {
            name: path
                .file_stem()
                .map_or("Recording".into(), |s| s.to_string_lossy().into_owned()),
            start,
            length: (sample.seconds() * bpm / 60.0).max(1.0 / 16.0),
            audio: Some(AudioRegion {
                file,
                ..Default::default()
            }),
            ..Default::default()
        };
        self.add_take(track, clip);
        if self.file.is_none() {
            self.status = format!(
                "Recorded to {}. Save the project to keep it with the take.",
                path.display()
            );
        }
    }

    /// Runs every frame: routes MIDI input, feeds takes, and ends them if the playhead jumps.
    pub(super) fn live_input(&mut self, ctx: &egui::Context) {
        if self
            .armed
            .is_some_and(|t| t >= self.session.project().tracks.len())
        {
            self.armed = None;
            self.audio_in = None;
        }
        let target = self.live_target();
        if let Some(midi) = &self.midi {
            midi.set_target(target);
        }
        let events: Vec<_> = self
            .midi
            .as_ref()
            .map_or(Vec::new(), |m| m.events().collect());
        for event in events {
            if let Some(Take::Midi(take)) = &mut self.recording
                && event.track == take.track
                && event.playing
                && !take.note(event.beat, event.pitch, event.velocity, event.on)
            {
                self.stop_recording();
            }
        }

        let position = self.audio.controller.position();
        let playing = self.audio.controller.is_playing();
        // The audio thread starts the transport a moment after Record is pressed; only a stop
        // after it has been seen running ends the take.
        self.record_rolling |= playing && self.recording.is_some();
        let mut stop = self.record_rolling && !playing && self.recording.is_some();
        match (&mut self.recording, &mut self.audio_in) {
            (Some(Take::Audio(take)), Some(input)) => {
                if input.drain(&mut take.left, &mut take.right) {
                    self.status = "Some input audio was lost (the window was too busy).".into();
                }
                stop |= !take.advance(position) || take.is_full();
            }
            (Some(Take::Midi(take)), input) => {
                stop |= !take.advance(position);
                if let Some(input) = input {
                    input.discard();
                }
            }
            (None, Some(input)) => input.discard(),
            _ => {}
        }
        if stop {
            self.stop_recording();
        }
        if self.recording.is_some() || self.audio_in.is_some() {
            ctx.request_repaint();
        }
    }

    /// The computer keyboard as a MIDI keyboard. Called when no text field has focus.
    pub(super) fn computer_keys(&mut self, ctx: &egui::Context) {
        let Some(track) = self.live_target() else {
            self.release_keys();
            return;
        };
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if repeat || (pressed && (modifiers.command || modifiers.alt || modifiers.ctrl)) {
                continue;
            }
            if pressed {
                match key {
                    Key::Z => self.keys.octave = (self.keys.octave - 1).max(-1),
                    Key::X => self.keys.octave = (self.keys.octave + 1).min(8),
                    Key::C => self.keys.velocity = (self.keys.velocity - 0.1).max(0.1),
                    Key::V => self.keys.velocity = (self.keys.velocity + 0.1).min(1.0),
                    _ => {}
                }
            }
            let Some(&(_, semitone)) = NOTE_KEYS.iter().find(|(k, _)| *k == key) else {
                continue;
            };
            if pressed {
                let pitch = (self.keys.octave + 1) * 12 + i32::from(semitone);
                let Ok(pitch) = u8::try_from(pitch) else {
                    continue;
                };
                if pitch > 127 || self.keys.held.iter().any(|(k, _)| *k == key) {
                    continue;
                }
                self.keys.held.push((key, pitch));
                self.play_note(track, pitch, self.keys.velocity, true);
            } else if let Some(i) = self.keys.held.iter().position(|(k, _)| *k == key) {
                let (_, pitch) = self.keys.held.remove(i);
                self.play_note(track, pitch, 0.0, false);
            }
        }
    }

    fn play_note(&mut self, track: usize, pitch: u8, velocity: f32, on: bool) {
        self.send(Command::LiveNote(LiveNote {
            track,
            pitch,
            velocity,
            on,
        }));
        let position = self.audio.controller.position();
        let playing = self.audio.controller.is_playing();
        if let Some(Take::Midi(take)) = &mut self.recording
            && take.track == track
            && playing
            && !take.note(position, pitch, velocity, on)
        {
            self.stop_recording();
        }
    }

    /// Lets go of every computer-keyboard note (when focus moves or the target changes).
    pub(super) fn release_keys(&mut self) {
        if !self.keys.held.is_empty() {
            self.keys.held.clear();
            self.send(Command::ReleaseLive);
        }
    }

    /// "C4 · 80%": the computer keyboard's octave and velocity, for the transport bar.
    pub(super) fn keys_label(&self) -> String {
        format!(
            "Keys C{} · {:.0}%",
            self.keys.octave,
            self.keys.velocity * 100.0
        )
    }
}
