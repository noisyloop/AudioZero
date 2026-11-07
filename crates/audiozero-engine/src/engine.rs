//! Renders a project to audio.

use crate::project::limits::MAX_BEATS;
use crate::project::{Clip, Device, Project, ProjectError, finite, finite32};

/// A track's mixer settings, for [`Engine::set_track_mix`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackMix {
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

/// How fast meters fall: to about 1% in 0.6 s.
fn meter_decay(frames: usize, sample_rate: f64) -> f32 {
    (-(frames as f64) / (0.13 * sample_rate)).exp() as f32
}

fn block_peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}
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
    peak: (f32, f32),
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
    master_peak: (f32, f32),
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
                TrackState {
                    instrument,
                    notes_on: [false; 128],
                    events: Vec::with_capacity(MAX_EVENTS_PER_BLOCK),
                    peak: (0.0, 0.0),
                }
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
            master_peak: (0.0, 0.0),
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

    /// A track's output level after its volume and pan, as decaying peaks (left, right).
    pub fn track_peak(&self, index: usize) -> (f32, f32) {
        self.tracks.get(index).map_or((0.0, 0.0), |t| t.peak)
    }

    /// The master output level, as decaying peaks (left, right).
    pub fn master_peak(&self) -> (f32, f32) {
        self.master_peak
    }

    // Live changes. Each goes through the same clamping as `Project::sanitize`, so a value from
    // the UI (or later an MCP client) can't put the engine somewhere a project file couldn't.

    pub fn set_bpm(&mut self, bpm: f64) {
        self.project.bpm = finite(bpm, 20.0, 999.0, self.project.bpm);
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.project.master_volume = finite32(volume, 0.0, 1.5, self.project.master_volume);
    }

    pub fn set_loop(&mut self, enabled: bool, start: f64, end: f64) {
        let start = finite(start, 0.0, MAX_BEATS, self.project.loop_start);
        let end = finite(end, 0.0, MAX_BEATS, self.project.loop_end);
        if end >= start + 1.0 / 16.0 {
            self.project.loop_start = start;
            self.project.loop_end = end;
            self.project.loop_enabled = enabled;
        } else {
            self.project.loop_enabled = false;
        }
    }

    /// Changes one track's mixer settings. Out-of-range track indexes are ignored.
    pub fn set_track_mix(&mut self, index: usize, mix: TrackMix) {
        if let Some(track) = self.project.tracks.get_mut(index) {
            track.volume = finite32(mix.volume, 0.0, 2.0, track.volume);
            track.pan = finite32(mix.pan, -1.0, 1.0, track.pan);
            track.mute = mix.mute;
            track.solo = mix.solo;
        }
    }

    /// Swaps in new clips for one track and returns the old ones, so they can be freed off the
    /// audio thread. Notes the old clips had playing are released. The clips must already be
    /// sanitized (see [`Clip::sanitize`]); [`crate::Controller::set_clips`] does that.
    /// Returns `clips` unchanged if the track doesn't exist.
    pub fn replace_clips(&mut self, index: usize, mut clips: Vec<Clip>) -> Vec<Clip> {
        if let (Some(track), Some(state)) = (
            self.project.tracks.get_mut(index),
            self.tracks.get_mut(index),
        ) {
            std::mem::swap(&mut track.clips, &mut clips);
            for pitch in 0..128u8 {
                if std::mem::take(&mut state.notes_on[usize::from(pitch)])
                    && let Some(i) = state.instrument.as_mut()
                {
                    i.note_off(pitch);
                }
            }
        }
        clips
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
            beats.clamp(0.0, MAX_BEATS)
        } else {
            0.0
        };
        self.release_everything();
    }

    fn release_everything(&mut self) {
        for track in &mut self.tracks {
            for pitch in 0..128u8 {
                if std::mem::take(&mut track.notes_on[usize::from(pitch)])
                    && let Some(i) = track.instrument.as_mut()
                {
                    i.note_off(pitch);
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
        let decay = meter_decay(n, self.sample_rate);
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

            let gain_l = track.volume * (1.0 - track.pan).min(1.0);
            let gain_r = track.volume * (1.0 + track.pan).min(1.0);
            state.peak = (
                (block_peak(l) * gain_l).max(state.peak.0 * decay),
                (block_peak(r) * gain_r).max(state.peak.1 * decay),
            );
            let audible = !track.mute && (!any_solo || track.solo);
            if !audible {
                continue;
            }
            for i in 0..n {
                out_l[i] += l[i] * gain_l;
                out_r[i] += r[i] * gain_r;
            }
        }

        let master = self.project.master_volume;
        for x in out_l.iter_mut().chain(out_r.iter_mut()) {
            *x = safety_clip(*x * master);
        }
        self.master_peak = (
            block_peak(out_l).max(self.master_peak.0 * decay),
            block_peak(out_r).max(self.master_peak.1 * decay),
        );
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
    fn live_changes_are_clamped_like_project_files() {
        let mut engine = Engine::new(one_note_project(), 48_000.0, 512).unwrap();
        engine.set_bpm(f64::NAN);
        assert_eq!(engine.project().bpm, 120.0, "NaN keeps the old value");
        engine.set_bpm(5000.0);
        assert_eq!(engine.project().bpm, 999.0);
        engine.set_master_volume(f32::INFINITY);
        assert_eq!(engine.project().master_volume, 0.85);
        engine.set_track_mix(
            0,
            TrackMix {
                volume: 9.0,
                pan: -3.0,
                mute: true,
                solo: false,
            },
        );
        let t = &engine.project().tracks[0];
        assert_eq!((t.volume, t.pan, t.mute), (2.0, -1.0, true));
        engine.set_track_mix(
            99,
            TrackMix {
                volume: 1.0,
                pan: 0.0,
                mute: false,
                solo: false,
            },
        );
        engine.set_loop(true, 8.0, 4.0);
        assert!(
            !engine.project().loop_enabled,
            "an inverted loop is turned off"
        );
        engine.set_loop(true, 4.0, 8.0);
        assert!(engine.project().loop_enabled);
    }

    #[test]
    fn meters_follow_the_output() {
        let mut engine = Engine::new(one_note_project(), 48_000.0, 512).unwrap();
        engine.play();
        engine.render(4800);
        assert!(engine.track_peak(0).0 > 0.05);
        assert!(engine.master_peak().0 > 0.05);
        assert_eq!(engine.track_peak(5), (0.0, 0.0));
        engine.stop();
        engine.render(96_000);
        assert!(
            engine.master_peak().0 < 1e-3,
            "meters fall back after stopping"
        );
    }

    #[test]
    fn replacing_clips_swaps_them_and_releases_notes() {
        let mut engine = Engine::new(one_note_project(), 48_000.0, 512).unwrap();
        engine.play();
        engine.render(4800);
        let old = engine.replace_clips(0, Vec::new());
        assert_eq!(old.len(), 1, "the old clips come back");
        assert!(engine.project().tracks[0].clips.is_empty());
        let (l, _) = engine.render(48_000);
        assert!(rms(&l[40_000..]) < 1e-4, "the held note was released");
        let back = engine.replace_clips(7, vec![Clip::default()]);
        assert_eq!(
            back.len(),
            1,
            "a missing track hands the clips straight back"
        );
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
