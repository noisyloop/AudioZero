//! What the visuals react to, worked out exactly for every video frame.
//!
//! The song is rendered offline with the engine first, so analysis sees exactly what export will
//! play. Besides the audio (loudness, bands, spectrum, onsets), the arrangement gives what a VJ
//! app listening to audio can't know: every note, on which track, and where the beat and bar are.

use audiozero_engine::sample::SamplePool;
use audiozero_engine::{Engine, Project};

/// Sample rate the song is rendered at for analysis and export.
pub const RATE: f64 = 48_000.0;
/// Bands in [`Features::spectrum`].
pub const SPECTRUM_BANDS: usize = 32;
/// Samples measured per frame.
pub const WINDOW: usize = 2048;
/// Longest stretch that can be rendered, in seconds (an hour).
pub const MAX_SECONDS: f64 = 3600.0;

/// Which part of the arrangement to render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Range {
    /// From the start to the end of the last clip, plus a two-second tail.
    Song,
    /// The loop brace, once.
    Loop,
    /// From one beat to another.
    Beats(f64, f64),
}

/// A note starting or sounding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub track: usize,
    pub pitch: u8,
    pub velocity: f32,
}

#[derive(Debug, Clone, Copy)]
struct NoteSpan {
    on: f64,
    off: f64,
    hit: Hit,
}

/// The rendered song and its notes.
pub struct Song {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub bpm: f64,
    pub beats_per_bar: u32,
    /// The beat the range starts at.
    pub start_beat: f64,
    pub tracks: usize,
    /// Mono, low-passed at 200 Hz and at 4 kHz, for the bands.
    low: Vec<f32>,
    below_high: Vec<f32>,
    notes: Vec<NoteSpan>,
}

impl Song {
    pub fn seconds(&self) -> f64 {
        self.left.len() as f64 / RATE
    }
}

/// Renders `range` of `project` and collects its notes.
pub fn render_song(project: &Project, pool: &mut SamplePool, range: Range) -> Result<Song, String> {
    let mut project = project.clone();
    project.sanitize().map_err(|e| e.to_string())?;
    let (start, end, tail) = match range {
        Range::Song => (0.0, project.end_beats(), 2.0),
        Range::Loop => (project.loop_start, project.loop_end, 0.0),
        Range::Beats(a, b) => (a, b, 0.0),
    };
    if !(start.is_finite() && end.is_finite() && end > start) {
        return Err("there's nothing to render in that range".into());
    }
    let seconds = (end - start) * 60.0 / project.bpm + tail;
    if seconds > MAX_SECONDS {
        return Err(format!("that's over {} minutes", MAX_SECONDS / 60.0));
    }
    // Render straight through: the range decides what plays, not the loop brace.
    project.loop_enabled = false;
    let mut engine =
        Engine::with_samples(project.clone(), RATE, 1024, pool).map_err(|e| e.to_string())?;
    engine.set_position(start);
    engine.play();
    let (left, right) = engine.render((seconds * RATE).round() as usize);

    let any_solo = project.tracks.iter().any(|t| t.solo);
    let mut notes = Vec::new();
    for (track, t) in project.tracks.iter().enumerate() {
        if t.mute || (any_solo && !t.solo) || t.instrument.is_none() {
            continue;
        }
        for clip in t.clips.iter().filter(|c| !c.muted) {
            for n in &clip.notes {
                let on = clip.start + n.start;
                let off = (on + n.length).min(clip.end());
                if on >= start && on < end {
                    let to_seconds = |beat: f64| (beat - start) * 60.0 / project.bpm;
                    notes.push(NoteSpan {
                        on: to_seconds(on),
                        off: to_seconds(off),
                        hit: Hit {
                            track,
                            pitch: n.pitch,
                            velocity: n.velocity,
                        },
                    });
                }
            }
        }
    }
    notes.sort_by(|a, b| a.on.total_cmp(&b.on));

    let mono: Vec<f32> = left
        .iter()
        .zip(&right)
        .map(|(l, r)| 0.5 * (l + r))
        .collect();
    Ok(Song {
        low: one_pole(&mono, 200.0),
        below_high: one_pole(&mono, 4000.0),
        left,
        right,
        bpm: project.bpm,
        beats_per_bar: project.beats_per_bar,
        start_beat: start,
        tracks: project.tracks.len(),
        notes,
    })
}

fn one_pole(input: &[f32], cutoff: f64) -> Vec<f32> {
    let a = (-2.0 * std::f64::consts::PI * cutoff / RATE).exp() as f32;
    let mut y = 0.0;
    input
        .iter()
        .map(|&x| {
            y = x + a * (y - x);
            y
        })
        .collect()
}

/// Everything a look can react to, for one video frame. Levels are 0 to 1.
#[derive(Debug, Clone, Default)]
pub struct Features {
    /// Seconds since the start of the range.
    pub time: f64,
    /// 0 to 1 through the range.
    pub progress: f32,
    /// The beat on the timeline.
    pub beat: f64,
    /// 0 to 1 through the current beat and bar.
    pub beat_phase: f32,
    pub bar_phase: f32,
    /// A beat or a bar started since the last frame.
    pub new_beat: bool,
    pub new_bar: bool,
    pub level: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    /// A sudden rise in the low end: kicks and hits.
    pub onset: f32,
    /// Low to high, smoothed so it falls back gently.
    pub spectrum: [f32; SPECTRUM_BANDS],
    /// Notes that started since the last frame.
    pub hits: Vec<Hit>,
    /// Notes sounding now.
    pub held: Vec<Hit>,
    pub tracks: usize,
}

/// Measures a window of audio: levels, bands, onsets and the spectrum, with the smoothing that
/// carries from one frame to the next. Shared by the offline [`Analyzer`] and [`LiveAnalyzer`],
/// so export and the live window react the same way.
pub struct Meter {
    hann: Vec<f32>,
    /// Goertzel coefficients for the band centres.
    coefficients: Vec<f64>,
    low_average: f32,
    spectrum: [f32; SPECTRUM_BANDS],
    scratch: Vec<f32>,
}

/// What [`Meter::measure`] gives.
#[derive(Debug, Clone, Copy, Default)]
pub struct Levels {
    pub level: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub onset: f32,
    pub spectrum: [f32; SPECTRUM_BANDS],
}

impl Default for Meter {
    fn default() -> Self {
        let hann = (0..WINDOW)
            .map(|i| {
                let x = i as f32 / (WINDOW - 1) as f32;
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * x).cos()
            })
            .collect();
        // Log-spaced band centres from 40 Hz to 12 kHz.
        let coefficients = (0..SPECTRUM_BANDS)
            .map(|i| {
                let freq = 40.0 * (12_000.0f64 / 40.0).powf(i as f64 / (SPECTRUM_BANDS - 1) as f64);
                (2.0 * std::f64::consts::PI * freq / RATE).cos() * 2.0
            })
            .collect();
        Self {
            hann,
            coefficients,
            low_average: 0.0,
            spectrum: [0.0; SPECTRUM_BANDS],
            scratch: vec![0.0; WINDOW],
        }
    }
}

impl Meter {
    /// Measures one window (up to [`WINDOW`] samples): the mono mix, and the same low-passed at
    /// 200 Hz and at 4 kHz.
    pub fn measure(&mut self, mono: &[f32], low: &[f32], below_high: &[f32]) -> Levels {
        let rms = |x: &mut dyn Iterator<Item = f32>| {
            let (sum, n) = x.fold((0.0f32, 0usize), |(s, n), v| (s + v * v, n + 1));
            if n == 0 { 0.0 } else { (sum / n as f32).sqrt() }
        };
        let level = rms(&mut mono.iter().copied());
        let low_rms = rms(&mut low.iter().copied());
        let mid = rms(&mut below_high.iter().zip(low).map(|(h, l)| h - l));
        let high = rms(&mut mono.iter().zip(below_high).map(|(m, h)| m - h));
        let shape = |x: f32, gain: f32| 1.0 - (-x * gain).exp();

        // Onsets: the low end jumping above its recent average.
        let onset = ((low_rms - self.low_average * 1.25) * 6.0).clamp(0.0, 1.0);
        self.low_average = self.low_average * 0.92 + low_rms * 0.08;

        // Goertzel magnitudes over a Hann window, in rough dB; quick to rise, slow to fall.
        let n = mono.len().min(WINDOW);
        self.scratch.fill(0.0);
        for i in 0..n {
            self.scratch[i] = mono[mono.len() - n + i] * self.hann[i];
        }
        for (band, &k) in self.spectrum.iter_mut().zip(&self.coefficients) {
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for &x in &self.scratch {
                let s = f64::from(x) + k * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            let power = (s1 * s1 + s2 * s2 - k * s1 * s2).max(0.0);
            let magnitude = power.sqrt() / (WINDOW as f64 / 4.0);
            let db = 20.0 * magnitude.max(1e-9).log10();
            let now = (((db + 60.0) / 60.0).clamp(0.0, 1.0)) as f32;
            *band = if now > *band {
                *band + (now - *band) * 0.7
            } else {
                *band * 0.86 + now * 0.14
            };
        }
        Levels {
            level: shape(level, 4.0),
            low: shape(low_rms, 5.0),
            mid: shape(mid, 6.0),
            high: shape(high, 12.0),
            onset,
            spectrum: self.spectrum,
        }
    }
}

/// Builds [`Features`] from levels and where the song is.
#[allow(clippy::too_many_arguments)]
fn features(
    levels: Levels,
    time: f64,
    progress: f32,
    beat: f64,
    previous_beat: Option<f64>,
    beats_per_bar: u32,
    hits: Vec<Hit>,
    held: Vec<Hit>,
    tracks: usize,
) -> Features {
    let per_bar = f64::from(beats_per_bar.max(1));
    let (new_beat, new_bar) = match previous_beat {
        None => (true, true),
        Some(p) => (
            beat.floor() != p.floor(),
            (beat / per_bar).floor() != (p / per_bar).floor(),
        ),
    };
    Features {
        time,
        progress,
        beat,
        beat_phase: beat.rem_euclid(1.0) as f32,
        bar_phase: (beat / per_bar).rem_euclid(1.0) as f32,
        new_beat,
        new_bar,
        level: levels.level,
        low: levels.low,
        mid: levels.mid,
        high: levels.high,
        onset: levels.onset,
        spectrum: levels.spectrum,
        hits,
        held,
        tracks,
    }
}

/// Walks a [`Song`] frame by frame.
pub struct Analyzer<'a> {
    song: &'a Song,
    fps: f64,
    frame: usize,
    meter: Meter,
    next_note: usize,
}

impl<'a> Analyzer<'a> {
    pub fn new(song: &'a Song, fps: u32) -> Self {
        Self {
            song,
            fps: f64::from(fps.clamp(1, 240)),
            frame: 0,
            meter: Meter::default(),
            next_note: 0,
        }
    }

    /// How many frames the song has at this frame rate.
    pub fn frames(&self) -> usize {
        (self.song.seconds() * self.fps).ceil() as usize
    }

    /// The next frame's features. Call once per frame, in order.
    pub fn next_frame(&mut self) -> Features {
        let song = self.song;
        let time = self.frame as f64 / self.fps;
        let centre = (time * RATE) as usize;
        let to = (centre + WINDOW / 2).min(song.left.len());
        let from = to.saturating_sub(WINDOW);
        let mono: Vec<f32> = (from..to)
            .map(|i| 0.5 * (song.left[i] + song.right[i]))
            .collect();
        let levels = self
            .meter
            .measure(&mono, &song.low[from..to], &song.below_high[from..to]);

        // Notes that started since the last frame, and those sounding now.
        let mut hits = Vec::new();
        while self.next_note < song.notes.len() && song.notes[self.next_note].on <= time {
            hits.push(song.notes[self.next_note].hit);
            self.next_note += 1;
        }
        let held = song.notes[..self.next_note]
            .iter()
            .filter(|n| n.off > time)
            .map(|n| n.hit)
            .collect();

        let beat = song.start_beat + time * song.bpm / 60.0;
        let previous =
            (self.frame > 0).then(|| song.start_beat + (time - 1.0 / self.fps) * song.bpm / 60.0);
        self.frame += 1;
        features(
            levels,
            time,
            (time / song.seconds().max(1e-9)).clamp(0.0, 1.0) as f32,
            beat,
            previous,
            song.beats_per_bar,
            hits,
            held,
            song.tracks,
        )
    }
}

/// Analyses audio as it plays, for the live visuals window: feed it the output with
/// [`LiveAnalyzer::push`], then ask for a frame with the playhead and the project.
pub struct LiveAnalyzer {
    mono: Vec<f32>,
    low: Vec<f32>,
    below_high: Vec<f32>,
    low_state: f32,
    high_state: f32,
    meter: Meter,
    last_beat: Option<f64>,
    rate: f64,
}

impl LiveAnalyzer {
    /// `rate` is the output's sample rate.
    pub fn new(rate: f64) -> Self {
        Self {
            mono: Vec::with_capacity(WINDOW * 2),
            low: Vec::with_capacity(WINDOW * 2),
            below_high: Vec::with_capacity(WINDOW * 2),
            low_state: 0.0,
            high_state: 0.0,
            meter: Meter::default(),
            last_beat: None,
            rate: if rate.is_finite() && rate > 1000.0 {
                rate
            } else {
                RATE
            },
        }
    }

    /// Adds mono output samples, keeping the last window.
    pub fn push(&mut self, samples: &[f32]) {
        let a_low = (-2.0 * std::f64::consts::PI * 200.0 / self.rate).exp() as f32;
        let a_high = (-2.0 * std::f64::consts::PI * 4000.0 / self.rate).exp() as f32;
        for &x in samples {
            let x = if x.is_finite() { x } else { 0.0 };
            self.low_state = x + a_low * (self.low_state - x);
            self.high_state = x + a_high * (self.high_state - x);
            self.mono.push(x);
            self.low.push(self.low_state);
            self.below_high.push(self.high_state);
        }
        for buf in [&mut self.mono, &mut self.low, &mut self.below_high] {
            if buf.len() > WINDOW {
                let extra = buf.len() - WINDOW;
                buf.drain(..extra);
            }
        }
    }

    /// The features now: the playhead at `beat`, and the notes in `project` that started since
    /// the last frame or are sounding (only while `playing`).
    pub fn frame(&mut self, project: &Project, beat: f64, playing: bool) -> Features {
        let levels = self.meter.measure(&self.mono, &self.low, &self.below_high);
        let previous = self.last_beat.filter(|p| beat >= *p && beat - *p < 4.0);
        let (mut hits, mut held) = (Vec::new(), Vec::new());
        if playing {
            let from = previous.unwrap_or(beat);
            let any_solo = project.tracks.iter().any(|t| t.solo);
            for (track, t) in project.tracks.iter().enumerate() {
                if t.mute || (any_solo && !t.solo) || t.instrument.is_none() {
                    continue;
                }
                for clip in t
                    .clips
                    .iter()
                    .filter(|c| !c.muted && c.start <= beat && c.end() > from)
                {
                    for n in &clip.notes {
                        let on = clip.start + n.start;
                        let off = (on + n.length).min(clip.end());
                        let hit = Hit {
                            track,
                            pitch: n.pitch,
                            velocity: n.velocity,
                        };
                        if on > from && on <= beat {
                            hits.push(hit);
                        }
                        if on <= beat && off > beat {
                            held.push(hit);
                        }
                    }
                }
            }
        }
        self.last_beat = Some(beat);
        features(
            levels,
            beat * 60.0 / project.bpm,
            0.0,
            beat,
            previous,
            project.beats_per_bar,
            hits,
            held,
            project.tracks.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audiozero_engine::demo;

    #[test]
    fn analyses_the_demo_song() {
        let project = demo::project();
        let mut pool = SamplePool::new(RATE);
        let song = render_song(&project, &mut pool, Range::Song).unwrap();
        let expected = project.end_beats() * 60.0 / project.bpm + 2.0;
        assert!((song.seconds() - expected).abs() < 0.01);
        let mut analyzer = Analyzer::new(&song, 30);
        let frames = analyzer.frames();
        let all: Vec<Features> = (0..frames).map(|_| analyzer.next_frame()).collect();
        assert!(
            all.iter()
                .all(|f| f.level.is_finite() && f.spectrum.iter().all(|x| x.is_finite()))
        );
        assert!(all.iter().any(|f| f.level > 0.1), "the song is audible");
        assert!(
            all.iter().any(|f| f.onset > 0.0),
            "the kick shows up as onsets"
        );
        let hits: usize = all.iter().map(|f| f.hits.len()).sum();
        let notes: usize = project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.notes.len())
            .sum();
        assert_eq!(hits, notes, "every note is reported once");
        let beats = all.iter().filter(|f| f.new_beat).count() as f64;
        assert!((beats - (song.seconds() * project.bpm / 60.0)).abs() <= 1.5);
        assert!(all.last().unwrap().progress > 0.95);
    }

    #[test]
    fn ranges_and_mutes_are_respected() {
        let mut project = demo::project();
        project.loop_start = 4.0;
        project.loop_end = 8.0;
        let mut pool = SamplePool::new(RATE);
        let song = render_song(&project, &mut pool, Range::Loop).unwrap();
        assert!((song.seconds() - 4.0 * 60.0 / project.bpm).abs() < 0.01);
        assert_eq!(song.start_beat, 4.0);

        project.tracks[0].mute = true;
        let song = render_song(&project, &mut pool, Range::Song).unwrap();
        assert!(
            song.notes.iter().all(|n| n.hit.track != 0),
            "muted tracks send no notes"
        );
        assert!(render_song(&project, &mut pool, Range::Beats(8.0, 8.0)).is_err());
    }

    #[test]
    fn live_analysis_reacts_to_audio_and_notes() {
        let project = demo::project();
        let mut live = LiveAnalyzer::new(48_000.0);
        let quiet = live.frame(&project, 0.0, true);
        assert_eq!(quiet.level, 0.0);
        let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.02).sin() * 0.5).collect();
        live.push(&tone);
        let loud = live.frame(&project, 0.5, true);
        assert!(loud.level > 0.1 && loud.low > 0.05);
        // The demo's first notes are at beat 0: a frame from before to after them sees hits.
        let mut live = LiveAnalyzer::new(48_000.0);
        live.frame(&project, -0.01, true);
        let f = live.frame(&project, 0.05, true);
        assert!(!f.hits.is_empty() && !f.held.is_empty());
        let stopped = live.frame(&project, 0.1, false);
        assert!(stopped.hits.is_empty(), "no notes while stopped");
    }
}
