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
const WINDOW: usize = 2048;
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

/// Walks a [`Song`] frame by frame.
pub struct Analyzer<'a> {
    song: &'a Song,
    fps: f64,
    frame: usize,
    last: Features,
    low_average: f32,
    next_note: usize,
    window: Vec<f32>,
    hann: Vec<f32>,
    bands: Vec<f64>,
}

impl<'a> Analyzer<'a> {
    pub fn new(song: &'a Song, fps: u32) -> Self {
        let hann = (0..WINDOW)
            .map(|i| {
                let x = i as f32 / (WINDOW - 1) as f32;
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * x).cos()
            })
            .collect();
        // Log-spaced band centres from 40 Hz to 12 kHz.
        let bands = (0..SPECTRUM_BANDS)
            .map(|i| 40.0 * (12_000.0f64 / 40.0).powf(i as f64 / (SPECTRUM_BANDS - 1) as f64))
            .collect();
        Self {
            song,
            fps: f64::from(fps.clamp(1, 240)),
            frame: 0,
            last: Features::default(),
            low_average: 0.0,
            next_note: 0,
            window: vec![0.0; WINDOW],
            hann,
            bands,
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
        let rms = |signal: &dyn Fn(usize) -> f32| {
            let from = centre.saturating_sub(WINDOW / 2);
            let to = (from + WINDOW).min(song.left.len());
            if to <= from {
                return 0.0;
            }
            ((from..to).map(|i| signal(i).powi(2)).sum::<f32>() / (to - from) as f32).sqrt()
        };
        let mono = |i: usize| 0.5 * (song.left[i] + song.right[i]);
        let level = rms(&mono);
        let low = rms(&|i| song.low[i]);
        let mid = rms(&|i| song.below_high[i] - song.low[i]);
        let high = rms(&|i| mono(i) - song.below_high[i]);
        let shape = |x: f32, gain: f32| 1.0 - (-x * gain).exp();

        // Onsets: the low end jumping above its recent average.
        let onset = ((low - self.low_average * 1.25) * 6.0).clamp(0.0, 1.0);
        self.low_average = self.low_average * 0.92 + low * 0.08;

        let spectrum = self.spectrum(centre);

        let beat = song.start_beat + time * song.bpm / 60.0;
        let per_bar = f64::from(song.beats_per_bar.max(1));
        let previous = song.start_beat + (time - 1.0 / self.fps) * song.bpm / 60.0;
        let new_beat = self.frame == 0 || beat.floor() != previous.floor();
        let new_bar = self.frame == 0 || (beat / per_bar).floor() != (previous / per_bar).floor();

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

        self.frame += 1;
        let features = Features {
            time,
            progress: (time / song.seconds().max(1e-9)).clamp(0.0, 1.0) as f32,
            beat,
            beat_phase: beat.fract() as f32,
            bar_phase: ((beat / per_bar).fract()) as f32,
            new_beat,
            new_bar,
            level: shape(level, 4.0),
            low: shape(low, 5.0),
            mid: shape(mid, 6.0),
            high: shape(high, 12.0),
            onset,
            spectrum,
            hits,
            held,
            tracks: song.tracks,
        };
        self.last = features.clone();
        features
    }

    /// Goertzel magnitudes at the band centres over a Hann window, in rough dB, smoothed:
    /// quick to rise, slow to fall.
    fn spectrum(&mut self, centre: usize) -> [f32; SPECTRUM_BANDS] {
        let song = self.song;
        let from = centre.saturating_sub(WINDOW / 2);
        for (i, w) in self.window.iter_mut().enumerate() {
            let at = from + i;
            *w = if at < song.left.len() {
                0.5 * (song.left[at] + song.right[at]) * self.hann[i]
            } else {
                0.0
            };
        }
        let mut out = [0.0f32; SPECTRUM_BANDS];
        for (band, &freq) in out.iter_mut().zip(&self.bands) {
            let k = (2.0 * std::f64::consts::PI * freq / RATE).cos() * 2.0;
            let (mut s1, mut s2) = (0.0f64, 0.0f64);
            for &x in &self.window {
                let s = f64::from(x) + k * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            let power = (s1 * s1 + s2 * s2 - k * s1 * s2).max(0.0);
            let magnitude = power.sqrt() / (WINDOW as f64 / 4.0);
            let db = 20.0 * magnitude.max(1e-9).log10();
            *band = (((db + 60.0) / 60.0).clamp(0.0, 1.0)) as f32;
        }
        for (now, before) in out.iter_mut().zip(self.last.spectrum) {
            *now = if *now > before {
                before + (*now - before) * 0.7
            } else {
                before * 0.86 + *now * 0.14
            };
        }
        out
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
}
