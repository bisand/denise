//! Failures from the evdev backend.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Something went wrong reading input devices.
#[derive(Debug)]
#[non_exhaustive]
pub enum EvdevError {
    /// `/dev/input` could not be listed.
    Enumerate(io::Error),

    /// A device node could not be opened.
    ///
    /// Almost always a permission problem: reading `/dev/input/event*` needs the
    /// `input` group.
    Open {
        /// The device that failed.
        path: PathBuf,
        /// The underlying failure.
        source: io::Error,
    },

    /// No device that this backend can use was found.
    NoDevices,
}

impl fmt::Display for EvdevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enumerate(_) => f.write_str("listing input devices"),
            Self::Open { path, .. } => write!(f, "opening {}", path.display()),
            Self::NoDevices => f.write_str("no usable pointer, touch or keyboard device found"),
        }
    }
}

impl std::error::Error for EvdevError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Enumerate(source) | Self::Open { source, .. } => Some(source),
            Self::NoDevices => None,
        }
    }
}
