//! Failures from the fbdev backend.

use std::io;
use std::path::PathBuf;

use denise::SurfaceError;

use crate::info::FbInfoError;

/// Something went wrong driving `/dev/fbN`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FbdevError {
    /// No `/dev/fb*` node exists.
    ///
    /// Expected on a modern kernel: fbdev is optional, and where it does exist it
    /// is usually DRM's emulation layer rather than a driver of its own.
    #[error("no framebuffer device found")]
    NoDevice,

    /// The device node could not be opened.
    ///
    /// Writing to a framebuffer needs the `video` group, or root.
    #[error("opening {path}")]
    Open {
        /// The device that failed.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// A sysfs attribute could not be read.
    #[error("reading {path}")]
    Sysfs {
        /// The attribute that failed.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// The geometry could not be understood.
    #[error(transparent)]
    Geometry(#[from] FbInfoError),

    /// The framebuffer could not be mapped into this process.
    #[error("mapping the framebuffer")]
    Map(#[source] io::Error),

    /// The mapping is smaller than the reported geometry needs.
    #[error("framebuffer is {actual} bytes but the geometry needs {required}")]
    TooSmall {
        /// Bytes the geometry requires.
        required: usize,
        /// Bytes actually mapped.
        actual: usize,
    },
}

impl From<FbdevError> for SurfaceError {
    fn from(err: FbdevError) -> Self {
        SurfaceError::backend(err)
    }
}
