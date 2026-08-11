//! Failures from the evdev backend.

use std::io;
use std::path::PathBuf;

/// Something went wrong reading input devices.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EvdevError {
    /// `/dev/input` could not be listed.
    #[error("listing input devices")]
    Enumerate(#[source] io::Error),

    /// A device node could not be opened.
    ///
    /// Almost always a permission problem: reading `/dev/input/event*` needs the
    /// `input` group.
    #[error("opening {path}")]
    Open {
        /// The device that failed.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// No device that this backend can use was found.
    #[error("no usable pointer, touch or keyboard device found")]
    NoDevices,
}
