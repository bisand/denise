//! Failures from the fbdev backend.

use std::fmt;
use std::io;
use std::path::PathBuf;

use denise::SurfaceError;

use crate::info::FbInfoError;

/// Something went wrong driving `/dev/fbN`.
#[derive(Debug)]
#[non_exhaustive]
pub enum FbdevError {
    /// No `/dev/fb*` node exists.
    ///
    /// Expected on a modern kernel: fbdev is optional, and where it does exist it
    /// is usually DRM's emulation layer rather than a driver of its own.
    NoDevice,

    /// The device node could not be opened.
    ///
    /// Writing to a framebuffer needs the `video` group, or root.
    Open {
        /// The device that failed.
        path: PathBuf,
        /// The underlying failure.
        source: io::Error,
    },

    /// A sysfs attribute could not be read.
    Sysfs {
        /// The attribute that failed.
        path: PathBuf,
        /// The underlying failure.
        source: io::Error,
    },

    /// The geometry could not be understood.
    Geometry(FbInfoError),

    /// The framebuffer could not be mapped into this process.
    Map(io::Error),

    /// The mapping is smaller than the reported geometry needs.
    TooSmall {
        /// Bytes the geometry requires.
        required: usize,
        /// Bytes actually mapped.
        actual: usize,
    },
}

impl fmt::Display for FbdevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => f.write_str("no framebuffer device found"),
            Self::Open { path, .. } => write!(f, "opening {}", path.display()),
            Self::Sysfs { path, .. } => write!(f, "reading {}", path.display()),
            Self::Geometry(err) => core::fmt::Display::fmt(err, f),
            Self::Map(_) => f.write_str("mapping the framebuffer"),
            Self::TooSmall { required, actual } => write!(
                f,
                "framebuffer is {actual} bytes but the geometry needs {required}"
            ),
        }
    }
}

impl std::error::Error for FbdevError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Sysfs { source, .. } | Self::Map(source) => {
                Some(source)
            }
            Self::NoDevice | Self::Geometry(_) | Self::TooSmall { .. } => None,
        }
    }
}

impl From<FbInfoError> for FbdevError {
    fn from(err: FbInfoError) -> Self {
        Self::Geometry(err)
    }
}

impl From<FbdevError> for SurfaceError {
    fn from(err: FbdevError) -> Self {
        SurfaceError::backend(err)
    }
}
