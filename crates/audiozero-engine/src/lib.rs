//! AudioZero's audio engine and project format, with no UI or platform code.
//!
//! - [`project`]: the project model, its JSON format, and the limits every project is held to.
//! - [`control`]: drives an engine on the audio thread from the UI thread, without locks.
//! - [`session`]: every edit to a project, with undo. The window and the MCP server both edit
//!   through it.
//! - [`effects`]: built-in track effects (delay, reverb, compressor).
//! - [`engine`]: renders a project to audio, sample-accurately, without allocating on the audio thread.
//! - [`sample`]: audio files for audio clips: path checks, bounded decoding and a pool of
//!   decoded audio.
//! - [`synth`]: the [`Instrument`](synth::Instrument) trait and built-in instruments, including
//!   Nusick's Kick Me.
//! - [`demo`]: a short built-in song.
//! - [`wav`]: a minimal WAV writer for export.

pub mod control;
pub mod demo;
pub mod effects;
pub mod engine;
pub mod project;
pub mod sample;
pub mod session;
pub mod synth;
pub mod wav;

pub use control::{Command, Controller, LiveInput, LiveNote, Player};
pub use engine::{Engine, TrackMix};
pub use project::{
    AudioRegion, Clip, Device, Note, PluginFormat, PluginRef, Project, ProjectError, Track,
};
pub use session::{Changes, ClipRef, EditError, Session};
