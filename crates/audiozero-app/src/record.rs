//! Recording takes: notes or audio captured while the transport runs, turned into a clip when
//! the take ends. Kept free of devices, so it can be tested without any.

use audiozero_engine::sample::{MAX_SAMPLE_SECONDS, Sample};
use audiozero_engine::{Clip, Note};

/// Shortest note a take keeps, in beats.
const MIN_NOTE: f64 = 1.0 / 128.0;

/// A MIDI take: notes in beats on the timeline.
#[derive(Debug)]
pub struct MidiTake {
    pub track: usize,
    pub start: f64,
    last: f64,
    /// (on beat, off beat once released, pitch, velocity)
    notes: Vec<(f64, Option<f64>, u8, f32)>,
    /// Pitches held down, with their index into `notes`.
    held: Vec<(u8, usize)>,
}

impl MidiTake {
    pub fn new(track: usize, start: f64) -> Self {
        Self {
            track,
            start,
            last: start,
            notes: Vec::new(),
            held: Vec::new(),
        }
    }

    /// Records a note on or off at `beat`. Returns `false` if the playhead went backwards (the
    /// loop wrapped or someone moved it), which ends the take.
    pub fn note(&mut self, beat: f64, pitch: u8, velocity: f32, on: bool) -> bool {
        if !self.advance(beat) {
            return false;
        }
        let pitch = pitch.min(127);
        if let Some(at) = self.held.iter().position(|&(p, _)| p == pitch) {
            let (_, i) = self.held.swap_remove(at);
            self.notes[i].1 = Some(beat);
        }
        if on {
            self.held.push((pitch, self.notes.len()));
            self.notes.push((beat, None, pitch, velocity));
        }
        true
    }

    /// Follows the playhead. Returns `false` if it went backwards.
    pub fn advance(&mut self, beat: f64) -> bool {
        if beat + 1e-6 < self.last {
            return false;
        }
        self.last = beat;
        true
    }

    /// The clip for this take, from the bar the take started in to the bar it ended in. Notes
    /// still held end with the take. `None` if nothing was played.
    pub fn finish(self, end: f64, beats_per_bar: u32) -> Option<Clip> {
        if self.notes.is_empty() {
            return None;
        }
        let end = end.max(self.last);
        let bar = f64::from(beats_per_bar.max(1));
        let start = (self.start / bar).floor() * bar;
        let clip_end = ((end / bar).ceil() * bar).max(start + bar);
        let notes = self
            .notes
            .into_iter()
            .filter(|(on, ..)| *on >= start && *on < clip_end)
            .map(|(on, off, pitch, velocity)| Note {
                pitch,
                start: on - start,
                length: (off.unwrap_or(end) - on).max(MIN_NOTE),
                velocity,
            })
            .collect();
        Some(Clip {
            name: "Recording".into(),
            start,
            length: clip_end - start,
            notes,
            ..Default::default()
        })
    }
}

/// An audio take: what the input captured, from `start` on the timeline.
#[derive(Debug)]
pub struct AudioTake {
    pub track: usize,
    pub start: f64,
    last: f64,
    pub rate: f64,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl AudioTake {
    pub fn new(track: usize, start: f64, rate: f64) -> Self {
        Self {
            track,
            start,
            last: start,
            rate,
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    pub fn seconds(&self) -> f64 {
        self.left.len().min(self.right.len()) as f64 / self.rate
    }

    /// Whether the take has reached the longest audio a clip can hold.
    pub fn is_full(&self) -> bool {
        self.seconds() >= MAX_SAMPLE_SECONDS
    }

    /// Follows the playhead. Returns `false` if it went backwards, which ends the take.
    pub fn advance(&mut self, beat: f64) -> bool {
        if beat + 1e-6 < self.last {
            return false;
        }
        self.last = beat;
        true
    }

    /// The recorded audio, or `None` if there's less than a few milliseconds.
    pub fn finish(self) -> Option<(usize, f64, Sample)> {
        if self.seconds() < 0.01 {
            return None;
        }
        Some((
            self.track,
            self.start,
            Sample::new(self.left, self.right, self.rate),
        ))
    }
}

/// A take in progress.
#[derive(Debug)]
pub enum Take {
    Midi(MidiTake),
    Audio(AudioTake),
}

impl Take {
    pub fn track(&self) -> usize {
        match self {
            Take::Midi(t) => t.track,
            Take::Audio(t) => t.track,
        }
    }
}

/// A file name for a recording on `track_name`: plain characters only, and not one that exists.
pub fn take_file_name(folder: &std::path::Path, track_name: &str) -> std::path::PathBuf {
    let clean: String = track_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let clean = clean.trim();
    let base = if clean.is_empty() { "Track" } else { clean };
    (1..)
        .map(|n| folder.join(format!("{base} take {n}.wav")))
        .find(|p| !p.exists())
        .expect("some take number is free")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_midi_take_becomes_a_bar_aligned_clip() {
        let mut take = MidiTake::new(2, 5.5);
        assert!(take.note(6.0, 60, 0.9, true));
        assert!(take.note(6.5, 64, 0.5, true));
        assert!(take.note(7.0, 60, 0.0, false));
        assert!(take.note(8.25, 60, 0.7, true)); // still held at the end
        let clip = take.finish(9.0, 4).unwrap();
        assert_eq!((clip.start, clip.length), (4.0, 8.0), "bars 2 and 3");
        let got: Vec<(u8, f64, f64)> = clip
            .notes
            .iter()
            .map(|n| (n.pitch, n.start, n.length))
            .collect();
        assert_eq!(
            got,
            vec![(60, 2.0, 1.0), (64, 2.5, 2.5), (60, 4.25, 0.75)],
            "held notes end with the take"
        );
        assert!(!clip.is_audio());
    }

    #[test]
    fn retriggering_a_held_pitch_ends_the_first_note() {
        let mut take = MidiTake::new(0, 0.0);
        take.note(0.0, 60, 1.0, true);
        take.note(1.0, 60, 1.0, true);
        let clip = take.finish(2.0, 4).unwrap();
        assert_eq!(clip.notes.len(), 2);
        assert_eq!(clip.notes[0].length, 1.0);
    }

    #[test]
    fn going_backwards_ends_a_take() {
        let mut take = MidiTake::new(0, 4.0);
        assert!(take.note(5.0, 60, 1.0, true));
        assert!(!take.note(0.5, 62, 1.0, true), "the loop wrapped");
        let mut audio = AudioTake::new(0, 3.0, 48_000.0);
        assert!(audio.advance(3.5));
        assert!(!audio.advance(1.0));
    }

    #[test]
    fn empty_takes_make_nothing() {
        assert!(MidiTake::new(0, 0.0).finish(8.0, 4).is_none());
        assert!(AudioTake::new(0, 0.0, 48_000.0).finish().is_none());
    }

    #[test]
    fn an_audio_take_keeps_its_start_and_rate() {
        let mut take = AudioTake::new(1, 2.5, 44_100.0);
        take.left = vec![0.25; 44_100];
        take.right = vec![-0.25; 44_100];
        assert!((take.seconds() - 1.0).abs() < 1e-9 && !take.is_full());
        let (track, start, sample) = take.finish().unwrap();
        assert_eq!((track, start, sample.rate()), (1, 2.5, 44_100.0));
        assert_eq!(sample.frames(), 44_100);
    }

    #[test]
    fn take_names_are_plain_and_new() {
        let dir = std::env::temp_dir().join(format!("audiozero-takes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = take_file_name(&dir, "Vox/../../etc");
        assert_eq!(first.file_name().unwrap(), "Vox_______etc take 1.wav");
        std::fs::write(&first, b"").unwrap();
        let second = take_file_name(&dir, "Vox/../../etc");
        assert_eq!(second.file_name().unwrap(), "Vox_______etc take 2.wav");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
