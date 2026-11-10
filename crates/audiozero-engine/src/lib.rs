//! AudioZero's audio engine and project format, with no UI or platform code.
//!
//! - [`project`]: the project model, its JSON format, and the limits every project is held to.
//! - [`control`]: drives an engine on the audio thread from the UI thread, without locks.
//! - [`session`]: every edit to a project, with undo. The window and the MCP server both edit
//!   through it.
//! - [`engine`]: renders a project to audio, sample-accurately, without allocating on the audio thread.
//! - [`synth`]: the [`Instrument`](synth::Instrument) trait and built-in instruments, including
//!   Nusick's Kick Me.
//! - [`demo`]: a short built-in song.
//! - [`wav`]: a minimal WAV writer for export.

pub mod control;
pub mod demo;
pub mod engine;
pub mod project;
pub mod session;
pub mod synth;
pub mod wav;

pub use control::{Command, Controller, Player};
pub use engine::{Engine, TrackMix};
pub use project::{Clip, Device, Note, PluginFormat, PluginRef, Project, ProjectError, Track};
pub use session::{Changes, ClipRef, EditError, Session};
