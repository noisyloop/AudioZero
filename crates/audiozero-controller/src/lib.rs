//! Game controllers as a playable instrument.
//!
//! The pieces, from the hardware up:
//!
//! - [`device`] polls controllers (through `gilrs`, which reads Linux's evdev devices, so a
//!   DualShock 4 needs no driver, no root and no MIDI port) on its own thread and sends plain
//!   [`RawEvent`]s over a channel. It never touches the audio thread.
//! - [`Mapper`] turns those events into musical [`Action`]s: notes in the song's key and scale,
//!   octave shifts, transport, and dead-zoned, smoothed analog controls. It is plain data and
//!   functions, so it is tested without a controller.
//! - [`Profile`] is what the user can change, saved as JSON.
//! - [`midi`] shows an action as the MIDI bytes it stands for, for the monitor and the
//!   diagnostic tool.
//!
//! The app owns the last step: it plays the actions through its live-note path, so recording,
//! the armed track and the key all behave as they do for a MIDI keyboard.

pub mod device;
pub mod input;
pub mod mapper;
pub mod midi;
pub mod profile;

pub use device::{Controllers, PadInfo};
pub use input::{Ax, Btn, RawEvent};
pub use mapper::{Action, Layout, Mapper};
pub use profile::Profile;
