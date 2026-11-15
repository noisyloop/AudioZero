//! AudioZero's audio engine and project format, with no UI or platform code.
//!
//! - [`project`]: the project model, its JSON format, and the limits every project is held to.
//! - [`arp`]: the arpeggiator, a MIDI effect that turns held notes into patterns.
//! - [`control`]: drives an engine on the audio thread from the UI thread, without locks.
//! - [`session`]: every edit to a project, with undo. The window and the MCP server both edit
//!   through it.
//! - [`drums`]: the drum rack: synthesized drum kits for several genres, and ready-made beats.
//! - [`effects`]: built-in track effects (delay, reverb, compressor).
//! - [`engine`]: renders a project to audio, sample-accurately, without allocating on the audio thread.
//! - [`fx`]: more track effects: EQ, chorus, phaser, bit crusher and distortion.
//! - [`glitch`]: a tempo-synced glitch effect (stutter, reverse, tape stop, gate, crush).
//! - [`export`]: rendering a song to a WAV or MP3 file.
//! - [`soundgineer`]: Soundgineer, the built-in wavetable synth.
//! - [`sample`]: audio files for audio clips: path checks, bounded decoding and a pool of
//!   decoded audio.
//! - [`synth`]: the [`Instrument`](synth::Instrument) trait and built-in instruments, including
//!   Nusick's Kick Me.
//! - [`operator`]: Operator, the built-in four-operator FM synth.
//! - [`music`]: keys and scales, and a melody generator that stays in key.
//! - [`demo`]: a short built-in song.
//! - [`wav`]: a minimal WAV writer for export.

pub mod arp;
pub mod control;
pub mod demo;
pub mod drums;
pub mod effects;
pub mod engine;
pub mod export;
pub mod fx;
pub mod glitch;
pub mod metadata;
pub mod music;
pub mod operator;
pub mod project;
pub mod sample;
pub mod session;
pub mod soundgineer;
pub mod synth;
pub mod tags;
pub mod wav;

pub use control::{Command, Controller, LiveInput, LiveNote, Player};
pub use engine::{Engine, TrackMix};
pub use project::{
    AudioRegion, Clip, Device, Note, PluginFormat, PluginRef, Project, ProjectError, Track,
};
pub use session::{Changes, ClipRef, EditError, Session};
