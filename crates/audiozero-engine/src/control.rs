//! Driving an [`Engine`] that lives on the audio thread, from the UI thread.
//!
//! [`channel`] returns two halves:
//!
//! - a [`Player`], which owns the engine and goes to the audio callback;
//! - a [`Controller`], which stays on the UI thread.
//!
//! Commands go to the audio thread through a lock-free queue, and the playhead and meters come
//! back through atomics, so the audio thread never waits on the UI. A new project is built into
//! a complete engine on the UI thread and swapped in whole. The old engine is sent back to be
//! freed on the UI thread, so the audio thread never allocates or frees memory.

use crate::engine::{Engine, TrackMix};
use crate::project::{
    Clip, Project, ProjectError,
    limits::{MAX_CLIPS_PER_TRACK, MAX_TRACKS},
};
use crate::sample::{Sample, SamplePool};
use crate::session::{Changes, mix_of};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

/// Commands the UI can send. Values are clamped by the engine, the same way project files are.
pub enum Command {
    Play,
    Stop,
    Seek(f64),
    SetBpm(f64),
    SetMasterVolume(f32),
    SetLoop {
        enabled: bool,
        start: f64,
        end: f64,
    },
    SetTrackMix {
        track: usize,
        mix: TrackMix,
    },
    /// Swap in one track's clips, built and sanitized on the UI thread
    /// (see [`Controller::set_clips`]).
    SetClips {
        track: usize,
        clips: Vec<Clip>,
        /// Each clip's decoded audio, for audio clips.
        samples: Vec<Option<Arc<Sample>>>,
    },
    /// Swap in an engine built on the UI thread.
    Replace(Box<Engine>),
    /// Play or release a note now (computer keyboard). MIDI input uses [`LiveInput`] instead.
    LiveNote(LiveNote),
    /// Release every note held live.
    ReleaseLive,
    /// Change one effect's parameters, in [`crate::effects::params_for`] order.
    SetEffectParams {
        track: usize,
        effect: usize,
        values: [f32; crate::effects::MAX_PARAMS],
    },
}

/// A note played live on a track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveNote {
    pub track: usize,
    pub pitch: u8,
    pub velocity: f32,
    pub on: bool,
}

const LIVE_CAPACITY: usize = 1024;

/// Sends live notes straight to the audio thread from another thread (a MIDI input callback),
/// without going through the UI. Get it once from [`Controller::take_live_input`].
pub struct LiveInput {
    notes: Producer<LiveNote>,
    status: Arc<Status>,
}

impl LiveInput {
    /// Queues a note. Returns `false` if the queue is full (the audio thread has stopped).
    pub fn send(&mut self, note: LiveNote) -> bool {
        self.notes.push(note).is_ok()
    }
    /// The playhead in beats, for timestamping recorded notes.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.status.position.load(Relaxed))
    }
    pub fn is_playing(&self) -> bool {
        self.status.playing.load(Relaxed)
    }
}

/// Things the audio thread has finished with, sent back to be freed on the UI thread.
enum Garbage {
    Engine(#[allow(dead_code)] Box<Engine>),
    Clips(
        #[allow(dead_code)] Vec<Clip>,
        #[allow(dead_code)] Vec<Option<Arc<Sample>>>,
    ),
}

const COMMAND_CAPACITY: usize = 1024;
const GARBAGE_CAPACITY: usize = 64;
/// Commands handled per audio callback, so a flood of them can't overrun the callback.
const MAX_COMMANDS_PER_CALLBACK: usize = 256;

/// What the audio thread reports back.
struct Status {
    position: AtomicU64,
    playing: AtomicBool,
    master_peak: [AtomicU32; 2],
    track_peaks: Vec<[AtomicU32; 2]>,
    callbacks: AtomicU64,
}

fn store(cell: &AtomicU32, value: f32) {
    cell.store(value.to_bits(), Relaxed);
}

fn load(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.load(Relaxed))
}

fn peak_pair(cells: &[AtomicU32; 2]) -> (f32, f32) {
    (load(&cells[0]), load(&cells[1]))
}

/// Creates a connected [`Controller`] and [`Player`] for an audio device running at
/// `sample_rate` with callbacks of up to `max_block` frames. The player starts with an empty
/// project.
pub fn channel(sample_rate: f64, max_block: usize) -> (Controller, Player) {
    let (commands, command_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (garbage_tx, garbage) = RingBuffer::new(GARBAGE_CAPACITY);
    let (live_tx, live_rx) = RingBuffer::new(LIVE_CAPACITY);
    let status = Arc::new(Status {
        position: AtomicU64::new(0f64.to_bits()),
        playing: AtomicBool::new(false),
        master_peak: Default::default(),
        track_peaks: (0..MAX_TRACKS).map(|_| Default::default()).collect(),
        callbacks: AtomicU64::new(0),
    });
    let engine =
        Engine::new(Project::default(), sample_rate, max_block).expect("an empty project is valid");
    let controller = Controller {
        commands,
        garbage,
        status: Arc::clone(&status),
        sample_rate: engine.sample_rate(),
        max_block,
        pool: SamplePool::new(engine.sample_rate()),
        live_input: Some(LiveInput {
            notes: live_tx,
            status: Arc::clone(&status),
        }),
    };
    let player = Player {
        engine: Box::new(engine),
        live: live_rx,
        commands: command_rx,
        garbage: garbage_tx,
        status,
    };
    (controller, player)
}

/// The audio thread's half. Call [`Player::process`] from the audio callback.
pub struct Player {
    engine: Box<Engine>,
    live: Consumer<LiveNote>,
    commands: Consumer<Command>,
    garbage: Producer<Garbage>,
    status: Arc<Status>,
}

impl Player {
    /// Applies pending commands, renders the next block into `left` and `right`, and publishes
    /// the playhead and meters. Doesn't allocate, free, lock or block.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for _ in 0..MAX_COMMANDS_PER_CALLBACK {
            let Ok(command) = self.commands.pop() else {
                break;
            };
            self.apply(command);
        }
        while let Ok(note) = self.live.pop() {
            self.engine
                .live_note(note.track, note.pitch, note.velocity, note.on);
        }
        self.engine.process(left, right);
        self.publish();
    }

    fn apply(&mut self, command: Command) {
        let engine = &mut self.engine;
        match command {
            Command::Play => engine.play(),
            Command::Stop => engine.stop(),
            Command::Seek(beats) => engine.set_position(beats),
            Command::SetBpm(bpm) => engine.set_bpm(bpm),
            Command::SetMasterVolume(volume) => engine.set_master_volume(volume),
            Command::SetLoop {
                enabled,
                start,
                end,
            } => engine.set_loop(enabled, start, end),
            Command::SetTrackMix { track, mix } => engine.set_track_mix(track, mix),
            Command::SetClips {
                track,
                clips,
                samples,
            } => {
                let (clips, samples) = engine.replace_clips(track, clips, samples);
                self.throw_away(Garbage::Clips(clips, samples));
            }
            Command::LiveNote(n) => engine.live_note(n.track, n.pitch, n.velocity, n.on),
            Command::ReleaseLive => engine.release_live(),
            Command::SetEffectParams {
                track,
                effect,
                values,
            } => engine.set_effect_params(track, effect, values),
            Command::Replace(new) => {
                let old = std::mem::replace(&mut self.engine, new);
                self.throw_away(Garbage::Engine(old));
            }
        }
    }

    fn throw_away(&mut self, garbage: Garbage) {
        if let Err(rtrb::PushError::Full(garbage)) = self.garbage.push(garbage) {
            // The UI hasn't collected garbage for a long time. Leaking this is better than
            // freeing memory on the audio thread.
            std::mem::forget(garbage);
        }
    }

    fn publish(&self) {
        let s = &self.status;
        s.position.store(self.engine.position().to_bits(), Relaxed);
        s.playing.store(self.engine.is_playing(), Relaxed);
        let (l, r) = self.engine.master_peak();
        store(&s.master_peak[0], l);
        store(&s.master_peak[1], r);
        for (i, cells) in s
            .track_peaks
            .iter()
            .enumerate()
            .take(self.engine.project().tracks.len())
        {
            let (l, r) = self.engine.track_peak(i);
            store(&cells[0], l);
            store(&cells[1], r);
        }
        s.callbacks.fetch_add(1, Relaxed);
    }
}

/// The UI thread's half.
pub struct Controller {
    commands: Producer<Command>,
    garbage: Consumer<Garbage>,
    status: Arc<Status>,
    sample_rate: f64,
    max_block: usize,
    /// Decoded audio for audio clips, at the device's rate.
    pool: SamplePool,
    live_input: Option<LiveInput>,
}

impl Controller {
    /// The sender for live notes from another thread. There's one; later calls return `None`.
    pub fn take_live_input(&mut self) -> Option<LiveInput> {
        self.live_input.take()
    }

    /// The decoded audio files. Set its folder ([`SamplePool::set_base`]) to the project's
    /// folder before loading a project with relative audio paths.
    pub fn samples(&mut self) -> &mut SamplePool {
        &mut self.pool
    }

    /// Queues a command. Returns `false` if the queue is full, which means the audio thread
    /// has stopped running.
    pub fn send(&mut self, command: Command) -> bool {
        self.commands.push(command).is_ok()
    }

    /// Builds an engine for `project` here on the UI thread and swaps it in. Returns the
    /// engine's warnings (missing plugins and so on). Playback stops and starts again at 0.
    pub fn load(&mut self, project: Project) -> Result<Vec<String>, ProjectError> {
        let used: Vec<&str> = project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter_map(|c| c.audio.as_ref().map(|a| a.file.as_str()))
            .collect();
        self.pool.retain(&used);
        let engine =
            Engine::with_samples(project, self.sample_rate, self.max_block, &mut self.pool)?;
        let warnings = engine.warnings().to_vec();
        for cells in &self.status.track_peaks {
            store(&cells[0], 0.0);
            store(&cells[1], 0.0);
        }
        if !self.send(Command::Replace(Box::new(engine))) {
            return Ok(vec![
                "The audio engine isn't responding; the project will load when it does.".into(),
            ]);
        }
        Ok(warnings)
    }

    /// Replaces one track's clips while playback continues. The clips are sanitized here, on
    /// the UI thread, the same way a project file's are. Returns `false` if there are too many
    /// clips or the audio thread isn't responding. Audio clips' files are decoded here if they
    /// haven't been; ones that can't be played come back as warnings.
    pub fn set_clips(&mut self, track: usize, mut clips: Vec<Clip>) -> Result<Vec<String>, String> {
        if clips.len() > MAX_CLIPS_PER_TRACK {
            return Err(format!(
                "more than {MAX_CLIPS_PER_TRACK} clips on one track"
            ));
        }
        for clip in &mut clips {
            clip.sanitize();
        }
        let mut warnings = Vec::new();
        let samples = crate::engine::resolve_samples(
            &mut self.pool,
            &clips,
            self.sample_rate,
            &format!("{}", track + 1),
            &mut warnings,
        );
        if self.send(Command::SetClips {
            track,
            clips,
            samples,
        }) {
            Ok(warnings)
        } else {
            Err("The audio engine isn't responding.".into())
        }
    }

    /// Like [`Controller::load`], but keeps the playhead where it is, and keeps playing if it
    /// was. For edits that change the tracks themselves (adding or removing one).
    pub fn reload(&mut self, project: Project) -> Result<Vec<String>, ProjectError> {
        let (position, playing) = (self.position(), self.is_playing());
        let warnings = self.load(project)?;
        self.send(Command::Seek(position));
        if playing {
            self.send(Command::Play);
        }
        Ok(warnings)
    }

    /// Brings the audio thread up to date after a [`Session`](crate::Session) edit, sending
    /// only what changed: one track's clips, a mix setting, or a whole new engine when tracks
    /// or devices changed. Returns the new engine's warnings, if it was rebuilt.
    pub fn apply(&mut self, project: &Project, changes: &Changes) -> Result<Vec<String>, String> {
        if changes.reload {
            return self
                .reload(project.clone())
                .map_err(|e| format!("Audio engine: {e}"));
        }
        let mut ok = true;
        let mut warnings = Vec::new();
        for &track in &changes.clips {
            if let Some(t) = project.tracks.get(track) {
                match self.set_clips(track, t.clips.clone()) {
                    Ok(w) => warnings.extend(w),
                    Err(_) => ok = false,
                }
            }
        }
        for &track in &changes.mix {
            if let Some(t) = project.tracks.get(track) {
                ok &= self.send(Command::SetTrackMix {
                    track,
                    mix: mix_of(t),
                });
            }
        }
        for &(track, effect) in &changes.effect_params {
            if let Some(device) = project
                .tracks
                .get(track)
                .and_then(|t| t.effects.get(effect))
            {
                ok &= self.send(Command::SetEffectParams {
                    track,
                    effect,
                    values: crate::engine::param_values(device),
                });
            }
        }
        if changes.bpm {
            ok &= self.send(Command::SetBpm(project.bpm));
        }
        if changes.master {
            ok &= self.send(Command::SetMasterVolume(project.master_volume));
        }
        if changes.looping {
            ok &= self.send(Command::SetLoop {
                enabled: project.loop_enabled,
                start: project.loop_start,
                end: project.loop_end,
            });
        }
        if ok {
            Ok(warnings)
        } else {
            Err("The audio engine isn't responding.".into())
        }
    }

    /// Frees engines and clips the audio thread has finished with. Call it regularly, for example once
    /// per UI frame.
    pub fn collect_garbage(&mut self) {
        while let Ok(old) = self.garbage.pop() {
            drop(old);
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
    /// The playhead, in beats, as of the last audio callback.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.status.position.load(Relaxed))
    }
    pub fn is_playing(&self) -> bool {
        self.status.playing.load(Relaxed)
    }
    pub fn master_peak(&self) -> (f32, f32) {
        peak_pair(&self.status.master_peak)
    }
    pub fn track_peak(&self, index: usize) -> (f32, f32) {
        self.status
            .track_peaks
            .get(index)
            .map_or((0.0, 0.0), peak_pair)
    }
    /// How many audio callbacks have run. If it stops increasing, the audio device has stopped.
    pub fn callbacks(&self) -> u64 {
        self.status.callbacks.load(Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(player: &mut Player, frames: usize) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        player.process(&mut l, &mut r);
        l
    }

    #[test]
    fn loads_plays_and_reports_back() {
        let (mut controller, mut player) = channel(48_000.0, 512);
        assert!(controller.load(crate::demo::project()).unwrap().is_empty());
        assert!(controller.send(Command::Play));
        let out = run(&mut player, 48_000);
        assert!(out.iter().any(|x| x.abs() > 0.01), "the demo is audible");
        assert!(controller.is_playing());
        assert!(controller.position() > 1.0);
        assert!(controller.master_peak().0 > 0.01);
        assert!(controller.track_peak(0).0 > 0.0);
        assert_eq!(controller.callbacks(), 1);

        controller.send(Command::Stop);
        controller.send(Command::Seek(8.0));
        run(&mut player, 64);
        assert!(!controller.is_playing());
        assert_eq!(controller.position(), 8.0);
    }

    #[test]
    fn replaced_engines_come_back_to_be_freed() {
        let (mut controller, mut player) = channel(48_000.0, 512);
        controller.load(crate::demo::project()).unwrap();
        controller.load(crate::demo::project()).unwrap();
        run(&mut player, 64);
        assert_eq!(
            controller.garbage.slots(),
            2,
            "the starting engine and the first demo"
        );
        controller.collect_garbage();
        assert_eq!(controller.garbage.slots(), 0);
    }

    #[test]
    fn live_values_are_clamped() {
        let (mut controller, mut player) = channel(48_000.0, 512);
        controller.load(crate::demo::project()).unwrap();
        controller.send(Command::SetBpm(f64::INFINITY));
        controller.send(Command::SetTrackMix {
            track: 0,
            mix: TrackMix {
                volume: f32::NAN,
                pan: 0.0,
                mute: false,
                solo: false,
            },
        });
        run(&mut player, 64);
        assert_eq!(
            player.engine.project().bpm,
            110.0,
            "infinity keeps the old tempo"
        );
        assert_eq!(
            player.engine.project().tracks[0].volume,
            0.6,
            "NaN keeps the old volume"
        );
    }

    #[test]
    fn clip_edits_are_sanitized_and_swapped_while_playing() {
        let (mut controller, mut player) = channel(48_000.0, 512);
        controller.load(crate::demo::project()).unwrap();
        controller.send(Command::Play);
        run(&mut player, 4800);
        let bad = Clip {
            start: f64::NAN,
            length: -3.0,
            ..Default::default()
        };
        assert!(controller.set_clips(1, vec![bad]).is_ok());
        run(&mut player, 64);
        let clips = &player.engine.project().tracks[1].clips;
        assert_eq!((clips[0].start, clips[0].length), (0.0, 1.0 / 16.0));
        assert!(controller.is_playing(), "playback carries on");
        controller.collect_garbage();
        assert!(
            controller
                .set_clips(0, vec![Clip::default(); MAX_CLIPS_PER_TRACK + 1])
                .is_err()
        );
    }

    #[test]
    fn session_edits_reach_the_audio_thread() {
        use crate::session::{ClipRef, Session};
        let (mut controller, mut player) = channel(48_000.0, 512);
        let mut session = Session::new(crate::demo::project()).unwrap();
        controller.load(session.project().clone()).unwrap();
        run(&mut player, 64);

        let at = ClipRef { track: 1, clip: 0 };
        let changes = session.update_clip(at, |c| c.start = 40.0).unwrap();
        let mix = TrackMix {
            mute: true,
            ..crate::session::mix_of(&session.project().tracks[0])
        };
        let mix_changes = session.set_track_mix(0, mix).unwrap();
        let bpm_changes = session.set_bpm(150.0).unwrap();
        for c in [&changes, &mix_changes, &bpm_changes] {
            assert!(controller.apply(session.project(), c).unwrap().is_empty());
        }
        run(&mut player, 64);
        let heard = player.engine.project();
        assert_eq!(heard.tracks[1].clips[0].start, 40.0);
        assert!(heard.tracks[0].mute);
        assert_eq!(heard.bpm, 150.0);

        let (_, changes) = session.add_track(crate::project::Track::default()).unwrap();
        controller.apply(session.project(), &changes).unwrap();
        run(&mut player, 64);
        assert_eq!(
            player.engine.project().tracks.len(),
            session.project().tracks.len(),
            "adding a track rebuilds the engine"
        );
    }

    #[test]
    fn reload_keeps_the_transport() {
        let (mut controller, mut player) = channel(48_000.0, 512);
        controller.load(crate::demo::project()).unwrap();
        controller.send(Command::Seek(6.0));
        controller.send(Command::Play);
        run(&mut player, 64);
        controller.reload(crate::demo::project()).unwrap();
        run(&mut player, 64);
        assert!(controller.is_playing());
        assert!((controller.position() - 6.0).abs() < 0.01);
    }

    #[test]
    fn a_full_queue_is_reported_not_blocking() {
        let (mut controller, _player) = channel(48_000.0, 512);
        for _ in 0..COMMAND_CAPACITY {
            assert!(controller.send(Command::Play));
        }
        assert!(!controller.send(Command::Play));
    }
}
