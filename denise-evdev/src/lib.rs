//! Linux evdev input for Denise.
//!
//! Reads mice, touchscreens and keyboards straight from `/dev/input/event*`, with
//! no display server in the way.
//!
//! # Testing
//!
//! [`translate`] and [`keymap`] are platform-independent and unit tested
//! everywhere. That is not tidiness: multitouch slot tracking, frame batching and
//! modifier state are the parts that break, and each one is far easier to pin down
//! as a table of raw event codes than by dragging a finger across a panel and
//! guessing. Only device discovery and reading are gated to Linux.
//!
//! # Permissions
//!
//! Reading `/dev/input/event*` needs membership in the `input` group, or root.
//! Being able to read every keystroke on the machine is exactly as sensitive as it
//! sounds, which is why the group exists.
//!
//! # Blocking
//!
//! [`InputBackend::poll`] never blocks: it drains whatever is ready and returns.
//! A frame loop that wants to sleep should wait on [`InputBackend::raw_fds`]
//! together with the DRM device's descriptor, so the process idles in the kernel
//! until either input arrives or the display retires a flip — rather than spinning
//! to ask.

pub mod codes;
pub mod keymap;
pub mod layout;
pub mod translate;

pub use keymap::key_code;
pub use translate::{AbsAxis, MAX_SLOTS, RawEvent, Translator};

#[cfg(target_os = "linux")]
mod device;
#[cfg(target_os = "linux")]
mod error;

#[cfg(target_os = "linux")]
pub use device::{Capabilities, InputBackend, InputDevice};
#[cfg(target_os = "linux")]
pub use error::EvdevError;
