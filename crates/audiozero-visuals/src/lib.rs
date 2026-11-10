//! Audio-reactive visuals for AudioZero: a built-in VJ that follows the arrangement.
//!
//! - [`analysis`]: renders the song with the engine and works out, for every video frame,
//!   loudness, bands, spectrum, onsets, the beat and bar, and every note on every track.
//! - [`canvas`]: a small CPU drawing surface (fades, additive glow, soft shapes).
//! - [`looks`]: the built-in looks: spectrum bars, track rings and note particles.
//! - [`export`]: renders a look over the song to MP4/MOV/MKV/WebM (through FFmpeg, with the
//!   song's audio) or to PNG frames, in widescreen, shorts, square, portrait or 4K.
//!
//! Everything is deterministic: the same project gives the same video.

pub mod analysis;
pub mod canvas;
pub mod export;
pub mod looks;

pub use analysis::{Analyzer, Features, LiveAnalyzer, Range, render_song};
pub use export::{FORMATS, VideoSettings, export, has_ffmpeg, still};
pub use looks::{LOOKS, Look, create_look};
