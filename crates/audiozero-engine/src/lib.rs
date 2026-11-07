//! AudioZero's audio engine and project format, with no UI or platform code.
//!
//! - [`project`]: the project model, its JSON format, and the limits every project is held to.
//! - [`engine`]: renders a project to audio, sample-accurately, without allocating on the audio thread.
//! - [`synth`]: the [`Instrument`](synth::Instrument) trait and built-in instruments.
//! - [`demo`]: a short built-in song.
//! - [`wav`]: a minimal WAV writer for export.

pub mod demo;
pub mod engine;
pub mod project;
pub mod synth;
pub mod wav;

pub use engine::Engine;
pub use project::{Clip, Device, Note, PluginFormat, PluginRef, Project, ProjectError, Track};
