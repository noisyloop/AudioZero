//! Renders a project to audio.

use crate::project::{Device, Project, ProjectError};
use crate::synth::{Instrument, create_builtin};

/// Note events one track can schedule in one block. More are dropped rather than allocating
/// on the audio thread.
const MAX_EVENTS_PER_BLOCK: usize = 4096;

#[derive(Clone, Copy)]
struct Event {
    offset: usize,
    on: bool,
    pitch: u8,
    velocity: f32,
}

struct TrackState {
    instrument: Option<Box<dyn Instrument>>,
    notes_on: [bool; 128],
    events: Vec<Event>,
}

impl TrackState {
    fn push(&mut self, event: Event) {
        if self.events.len() < MAX_EVENTS_PER_BLOCK {
            self.events.push(event);
        }
    }

    /// Schedules note-offs for every note this track's clips have playing.
    fn release_all(&mut self, offset: usize) {
        for pitch in 0..128u8 {
            if std::mem::take(&mut self.notes_on[usize::from(pitch)]) {
                self.push(Event {
                    offset,
                    on: false,
                    pitch,
                    velocity: 0.0,
                });
            }
        }
    }
}

/// Plays a project. `process` is meant to run on the audio thread: after `new`, nothing in it
/// allocates, locks or blocks.
pub struct Engine {
    project: Project,
    sample_rate: f64,
    max_block: usize,
    tracks: Vec<TrackState>,
    position: f64,
    playing: bool,
    warnings: Vec<String>,
    track_l: Vec<f32>,
    track_r: Vec<f32>,
}

impl Engine {
    /// Prepares `project` for playback at `sample_rate`, in blocks of up to `max_block` frames.
    /// The project is sanitized first, so the engine never sees out-of-range values.
    pub fn new(
        mut project: Project,
        sample_rate: f64,
        max_block: usize,
    ) -> Result<Self, ProjectError> {
        project.sanitize()?;
        let sample_rate = if sample_rate.is_finite() && (8_000.0..=768_000.0).contains(&sample_rate)
        {
            sample_rate
        } else {
            48_000.0
        };
        let max_block = max_block.clamp(16, 8192);
        let mut warnings = Vec::new();
        let tracks = project
            .tracks
            .iter()
            .map(|track| {
                let instrument = match &track.instrument {
                    None => None,
                    Some(Device::Builtin { id }) => {
                        let made = create_builtin(id);
                        if made.is_none() {
                            warnings.push(format!("Track \"{}\": unknown built-in instrument \"{id}\"", track.name));
                        }
                        made
                    }
                    Some(Device::Plugin(p)) => {
                        warnings.push(format!(
                            "Track \"{}\": plugin \"{}\" by {} isn't loaded (plugin hosting isn't implemented yet)",
                            track.name, p.name, p.vendor
                        ));
                        None
                    }
                };
                if !track.effects.is_empty() {
                    warnings.push(format!("Track \"{}\": effects aren't implemented yet", track.name));
                }
                let mut instrument = instrument;
                if let Some(i) = instrument.as_mut() {
                    i.prepare(sample_rate);
                }
                TrackState { instrument, notes_on: [false; 128], events: Vec::with_capacity(MAX_EVENTS_PER_BLOCK) }
            })
            .collect();
        Ok(Self {
            project,
            sample_rate,
            max_block,
            tracks,
            position: 0.0,
            playing: false,
            warnings,
            track_l: vec![0.0; max_block],
            track_r: vec![0.0; max_block],
        })
    }

    pub fn project(&self) -> &Project {
        &self.project
    }
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
    /// Problems that didn't stop the project loading, such as missing plugins.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    /// The playhead, in beats.
    pub fn position(&self) -> f64 {
        self.position
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.release_everything();
    }

    pub fn set_position(&mut self, beats: f64) {
        self.position = if beats.is_finite() {
            beats.clamp(0.0, crate::project::limits::MAX_BEATS)
        } else {
            0.0
        };
        self.release_everything();
    }

    fn release_everything(&mut self) {
        for track in &mut self.tracks {
            for pitch in 0..128u8 {
                if std::mem::take(&mut track.notes_on[usize::from(pitch)]) {
                    if let Some(i) = track.instrument.as_mut() {
                        i.note_off(pitch);
                    }
                }
            }
        }
    }

    /// Fills `left` and `right` (the same length, any size) with the next block of audio.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.max_block);
            self.process_block(&mut left[done..done + n], &mut right[done..done + n]);
            done += n;
        }
    }

    /// Renders `frames` frames offline, for export and tests.
    pub fn render(&mut self, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        self.process(&mut left, &mut right);
        (left, right)
    }

    /// Seconds from the playhead to the end of the last clip.
    pub fn seconds_to_end(&self) -> f64 {
        (self.project.end_beats() - self.position).max(0.0) * 60.0 / self.project.bpm
    }

    fn process_block(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = out_l.len();
        out_l.fill(0.0);
        out_r.fill(0.0);
        for track in &mut self.tracks {
            track.events.clear();
        }

        if self.playing {
            self.schedule(n);
        }

        let any_solo = self.project.tracks.iter().any(|t| t.solo);
        for (state, track) in self.tracks.iter_mut().zip(&self.project.tracks) {
            let (l, r) = (&mut self.track_l[..n], &mut self.track_r[..n]);
            l.fill(0.0);
            r.fill(0.0);
            state.events.sort_unstable_by_key(|e| (e.offset, e.on));
            if let Some(instrument) = state.instrument.as_mut() {
                let mut cursor = 0;
                for event in &state.events {
                    let at = event.offset.min(n);
                    if at > cursor {
                        instrument.render(&mut l[cursor..at], &mut r[cursor..at]);
                        cursor = at;
                    }
                    if event.on {
                        instrument.note_on(event.pitch, event.velocity);
                    } else {
                        instrument.note_off(event.pitch);
                    }
                }
                if cursor < n {
                    instrument.render(&mut l[cursor..], &mut r[cursor..]);
                }
            }

            let audible = !track.mute && (!any_solo || track.solo);
            if !audible {
                continue;
            }
            let gain_l = track.volume * (1.0 - track.pan).min(1.0);
            let gain_r = track.volume * (1.0 + track.pan).min(1.0);
            for i in 0..n {
                out_l[i] += l[i] * gain_l;
                out_r[i] += r[i] * gain_r;
            }
        }

        let master = self.project.master_volume;
        for x in out_l.iter_mut().chain(out_r.iter_mut()) {
            *x = safety_clip(*x * master);
        }
    }

    /// Turns the clips under the playhead into note events for this block, wrapping at the loop.
    fn schedule(&mut self, n: usize) {
        let p = &self.project;
        let samples_per_beat = self.sample_rate * 60.0 / p.bpm;
        let looping = p.loop_enabled && p.loop_end > p.loop_start;
        if looping && self.position >= p.loop_end {
            self.position = p.loop_start;
            for track in &mut self.tracks {
                track.release_all(0);
            }
        }

        let mut offset = 0;
        while offset < n {
            let mut len = n - offset;
            if looping {
                let to_end = ((p.loop_end - self.position) * samples_per_beat).ceil();
                if to_end < len as f64 {
                    len = (to_end as usize).max(1);
                }
            }
            let b0 = self.position;
            let b1 = b0 + len as f64 / samples_per_beat;
            for (state, track) in self.tracks.iter_mut().zip(&p.tracks) {
                schedule_track(state, track, b0, b1, offset, len, samples_per_beat);
            }
            self.position = b1;
            offset += len;
            if looping && self.position >= p.loop_end - 1e-9 {
                self.position = p.loop_start;
                for track in &mut self.tracks {
                    track.release_all(offset.min(n - 1));
                }
            }
        }
    }
}

fn schedule_track(
    state: &mut TrackState,
    track: &crate::project::Track,
    b0: f64,
    b1: f64,
    offset: usize,
    len: usize,
    samples_per_beat: f64,
) {
    // Positions are compared as f64 and clamped before converting, so no out-of-range cast
    // can happen (security review, finding 4).
    let to_offset =
        |beat: f64| offset + (((beat - b0) * samples_per_beat).max(0.0) as usize).min(len - 1);
    // Note-offs first, so a pitch re-triggered on the same sample isn't cut off.
    for pass_on in [false, true] {
        for clip in &track.clips {
            if clip.muted || clip.end() < b0 || clip.start >= b1 {
                continue;
            }
            for note in &clip.notes {
                let on = clip.start + note.start;
                let off = (on + note.length).min(clip.end());
                let pitch = usize::from(note.pitch.min(127));
                if !pass_on && off >= b0 && off < b1 && state.notes_on[pitch] {
                    state.notes_on[pitch] = false;
                    state.push(Event {
                        offset: to_offset(off),
                        on: false,
                        pitch: note.pitch,
                        velocity: 0.0,
                    });
                }
                if pass_on && on >= b0 && on < b1 {
                    state.notes_on[pitch] = true;
                    state.push(Event {
                        offset: to_offset(on),
                        on: true,
                        pitch: note.pitch,
                        velocity: note.velocity,
                    });
                }
            }
        }
    }
}

/// Transparent below 0.9, never goes past full scale, and turns NaN or infinity into silence
/// so they can't reach the audio device (security review, finding 5).
fn safety_clip(x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        (0.9 + 0.1 * ((a - 0.9) * 10.0).tanh()).copysign(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Clip, Note, PluginFormat, PluginRef, Track};

    fn one_note_project() -> Project {
        Project {
            tracks: vec![Track {
                instrument: Some(Device::Builtin { id: "sine".into() }),
                clips: vec![Clip {
                    notes: vec![Note {
                        pitch: 69,
                        start: 0.0,
                        length: 1.0,
                        velocity: 1.0,
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|x| x * x).sum::<f32>() / buf.len().max(1) as f32).sqrt()
    }

    #[test]
    fn plays_a_note_and_advances_the_playhead() {
        let mut engine = Engine::new(one_note_project(), 48_000.0, 512).unwrap();
        engine.play();
        let (l, r) = engine.render(24_000); // one beat at 120 BPM
        assert!(rms(&l) > 0.05 && rms(&r) > 0.05);
        assert!((engine.position() - 1.0).abs() < 1e-9);
        assert!(engine.warnings().is_empty());
    }

    #[test]
    fn silent_when_stopped_muted_or_soloed_away() {
        let mut engine = Engine::new(one_note_project(), 48_000.0, 512).unwrap();
        assert_eq!(rms(&engine.render(4800).0), 0.0, "not playing");

        let mut muted = one_note_project();
        muted.tracks[0].mute = true;
        let mut engine = Engine::new(muted, 48_000.0, 512).unwrap();
        engine.play();
        assert_eq!(rms(&engine.render(4800).0), 0.0, "muted");

        let mut soloed = one_note_project();
        soloed.tracks.push(Track {
            solo: true,
            ..Default::default()
        });
        let mut engine = Engine::new(soloed, 48_000.0, 512).unwrap();
        engine.play();
        assert_eq!(rms(&engine.render(4800).0), 0.0, "another track is soloed");
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let render = |block| {
            let mut engine = Engine::new(crate::demo::project(), 44_100.0, block).unwrap();
            engine.play();
            engine.render(44_100)
        };
        let (a, _) = render(64);
        let (b, _) = render(4096);
        let diff = a
            .iter()
            .zip(&b)
            .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(diff < 1e-5, "max difference {diff}");
    }

    #[test]
    fn loops_back_to_the_loop_start() {
        let mut project = one_note_project();
        project.loop_enabled = true;
        project.loop_start = 0.0;
        project.loop_end = 2.0;
        let mut engine = Engine::new(project, 48_000.0, 512).unwrap();
        engine.play();
        engine.render(48_000 * 3 / 2); // three beats
        assert!(
            (engine.position() - 1.0).abs() < 1e-6,
            "position {}",
            engine.position()
        );
    }

    #[test]
    fn output_never_exceeds_full_scale_or_goes_non_finite() {
        let mut project = crate::demo::project();
        project.master_volume = 1.5;
        for track in &mut project.tracks {
            track.volume = 2.0;
        }
        let mut engine = Engine::new(project, 48_000.0, 256).unwrap();
        engine.play();
        let (l, r) = engine.render(96_000);
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() <= 1.0));
        assert_eq!(safety_clip(f32::NAN), 0.0);
        assert_eq!(safety_clip(f32::INFINITY), 0.0);
    }

    #[test]
    fn stop_releases_held_notes() {
        let mut project = one_note_project();
        project.tracks[0].clips[0].notes[0].length = 100.0;
        project.tracks[0].clips[0].length = 100.0;
        let mut engine = Engine::new(project, 48_000.0, 512).unwrap();
        engine.play();
        engine.render(4800);
        engine.stop();
        let (l, _) = engine.render(48_000);
        assert!(rms(&l[40_000..]) < 1e-4, "release tail has finished");
    }

    #[test]
    fn missing_plugins_are_warnings_not_errors() {
        let mut project = one_note_project();
        project.tracks[0].instrument = Some(Device::Plugin(PluginRef {
            format: PluginFormat::Vst3,
            id: "ABCDEF".into(),
            name: "Some Synth".into(),
            vendor: "Someone".into(),
        }));
        let engine = Engine::new(project, 48_000.0, 512).unwrap();
        assert_eq!(engine.warnings().len(), 1);
        assert!(engine.warnings()[0].contains("Some Synth"));
    }
}
