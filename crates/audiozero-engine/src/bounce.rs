//! Bouncing a track to audio: playing one track (its clips, instrument, effects, volume and pan)
//! offline and capturing the result, so it can sit on an audio track of its own.

use crate::engine::Engine;
use crate::project::{Clip, Project};
use crate::sample::{Sample, SamplePool};

/// Seconds added after the last clip, for reverb and delay tails.
pub const TAIL_SECONDS: f64 = 2.0;
/// The longest bounce, in seconds.
pub const MAX_SECONDS: f64 = 1800.0;
const BLOCK: usize = 4096;

/// A bounced track and where it goes on the timeline.
pub struct Bounced {
    pub sample: Sample,
    /// Where the audio starts, in beats: the start of the track's first clip.
    pub start_beat: f64,
}

/// Renders track `index` of `project` (a group bounces everything inside it, with the group's own
/// effects) at `rate`. `progress(done, total)` is told how many frames are done and returns
/// `false` to cancel.
pub fn bounce_track(
    project: &Project,
    pool: &mut SamplePool,
    index: usize,
    rate: f64,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Bounced, String> {
    let track = project
        .tracks
        .get(index)
        .ok_or_else(|| format!("there's no track {index}"))?;
    let mut tracks: Vec<_> = project.tracks[project.block(index)].to_vec();
    // Alone: not muted or soloed, and not inside a group that's being left behind.
    for t in &mut tracks {
        t.mute = false;
        t.solo = false;
    }
    tracks[0].parent = None;

    let mut clips = tracks.iter().flat_map(|t| &t.clips).filter(|c| !c.muted);
    let first = clips.next().map(|c: &Clip| (c.start, c.end()));
    let (start, end) = clips.fold(
        first.ok_or_else(|| format!("\"{}\" has no clips to bounce", track.name))?,
        |(s, e), c| (s.min(c.start), e.max(c.end())),
    );
    let seconds_per_beat = 60.0 / project.bpm;
    let seconds = (end - start) * seconds_per_beat + TAIL_SECONDS;
    if seconds > MAX_SECONDS {
        return Err(format!(
            "that's {:.0} minutes of audio; the limit is {:.0}",
            seconds / 60.0,
            MAX_SECONDS / 60.0
        ));
    }
    let frames = (seconds * rate).ceil() as usize;

    // The track plays through the master later, so the bounce leaves the master gain out.
    let alone = Project {
        tracks,
        master_volume: 1.0,
        loop_enabled: false,
        ..project.clone()
    };
    let mut engine = Engine::with_samples(alone, rate, BLOCK, pool)
        .map_err(|e| format!("can't render the track: {e}"))?;
    engine.set_loop(false, 0.0, 1.0);
    engine.set_position(start);
    engine.play();

    let (mut left, mut right) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    let (mut block_l, mut block_r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    while left.len() < frames {
        let n = BLOCK.min(frames - left.len());
        engine.process(&mut block_l[..n], &mut block_r[..n]);
        left.extend_from_slice(&block_l[..n]);
        right.extend_from_slice(&block_r[..n]);
        if !progress(left.len(), frames) {
            return Err(crate::export::CANCELLED.into());
        }
    }
    Ok(Bounced {
        sample: Sample::new(left, right, rate),
        start_beat: start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn rms(s: &Sample) -> f32 {
        (s.left().iter().map(|x| x * x).sum::<f32>() / s.frames().max(1) as f32).sqrt()
    }

    #[test]
    fn a_track_bounces_to_its_own_audio() {
        let project = demo::project();
        let mut pool = SamplePool::new(48_000.0);
        for (i, track) in project.tracks.iter().enumerate() {
            if track.clips.is_empty() {
                continue;
            }
            let b = bounce_track(&project, &mut pool, i, 48_000.0, |_, _| true).unwrap();
            assert!(
                rms(&b.sample) > 0.001,
                "\"{}\" bounced to silence",
                track.name
            );
            let first = track.clips.iter().map(|c| c.start).fold(f64::MAX, f64::min);
            assert_eq!(b.start_beat, first);
            let beats = track.clips.iter().map(Clip::end).fold(0.0, f64::max) - first;
            let want = beats * 60.0 / project.bpm + TAIL_SECONDS;
            assert!((b.sample.seconds() - want).abs() < 0.01);
        }
    }

    #[test]
    fn the_bounces_add_up_to_the_song() {
        // Every track bounced alone and summed is what the whole song renders to (the master
        // gain is the only difference, and the bounce leaves it out).
        let project = demo::project();
        let rate = 48_000.0;
        let mut pool = SamplePool::new(rate);
        let mut sum: Vec<f32> = Vec::new();
        for i in 0..project.tracks.len() {
            let b = bounce_track(&project, &mut pool, i, rate, |_, _| true).unwrap();
            let offset = (b.start_beat * 60.0 / project.bpm * rate).round() as usize;
            if sum.len() < offset + b.sample.frames() {
                sum.resize(offset + b.sample.frames(), 0.0);
            }
            for (k, x) in b.sample.left().iter().enumerate() {
                sum[offset + k] += x;
            }
        }
        let mut whole = project.clone();
        whole.master_volume = 1.0;
        let mut engine = Engine::with_samples(whole, rate, 4096, &mut pool).unwrap();
        engine.set_loop(false, 0.0, 1.0);
        engine.set_position(0.0);
        engine.play();
        let n = sum.len().min(48_000 * 10);
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        for (cl, cr) in l.chunks_mut(4096).zip(r.chunks_mut(4096)) {
            engine.process(cl, cr);
        }
        let (mut err, mut tot) = (0.0f64, 0.0f64);
        for i in 0..n {
            err += f64::from(l[i] - sum[i]).powi(2);
            tot += f64::from(l[i]).powi(2);
        }
        let snr = 10.0 * (tot / err.max(1e-20)).log10();
        assert!(
            snr > 40.0,
            "the tracks bounced alone add up to the song ({snr:.1} dB)"
        );
    }

    #[test]
    fn nothing_to_bounce_and_cancel_are_errors() {
        let mut project = demo::project();
        let mut pool = SamplePool::new(48_000.0);
        assert!(bounce_track(&project, &mut pool, 99, 48_000.0, |_, _| true).is_err());
        project.tracks[0].clips.clear();
        let e = bounce_track(&project, &mut pool, 0, 48_000.0, |_, _| true)
            .err()
            .unwrap();
        assert!(e.contains("no clips"), "{e}");
        let project = demo::project();
        let e = bounce_track(&project, &mut pool, 0, 48_000.0, |_, _| false)
            .err()
            .unwrap();
        assert_eq!(e, crate::export::CANCELLED);
    }
}
