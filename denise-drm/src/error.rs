//! Failures from the DRM backend.

use std::fmt;
use std::io;
use std::path::PathBuf;

use denise::SurfaceError;

use crate::mode::SelectionError;

/// Something went wrong talking to the display.
#[derive(Debug)]
#[non_exhaustive]
pub enum DrmError {
    /// `/dev/dri` holds no card node.
    NoDevice,

    /// A card node could not be opened.
    Open {
        /// The device that failed.
        path: PathBuf,
        /// The underlying failure.
        source: io::Error,
    },

    /// Every card node was opened but none can drive a display.
    ///
    /// Usually means only render nodes are present — a headless GPU, or a card
    /// whose display side is claimed by another driver.
    NoDisplayCapableDevice,

    /// This process could not become DRM master.
    ///
    /// Only one process at a time may set modes. See the crate documentation: run
    /// on a bare VT, or take a descriptor from `libseat` or systemd.
    NotMaster(io::Error),

    /// Enumerating connectors, encoders and CRTCs failed.
    Resources(io::Error),

    /// No output could be chosen.
    Selection(SelectionError),

    /// The chosen connector has no CRTC that can drive it.
    NoCrtc {
        /// The connector id that could not be routed.
        connector: u32,
    },

    /// A scanout buffer could not be allocated.
    Allocate {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// The underlying failure.
        source: io::Error,
    },

    /// A scanout buffer could not be mapped into this process.
    Map(io::Error),

    /// The driver handed back a row pitch that is not a whole number of pixels.
    ///
    /// Denise addresses buffers as `u32` words, so a pitch that is not a multiple
    /// of four bytes cannot be expressed as a pixel stride.
    UnalignedPitch {
        /// The pitch the driver reported, in bytes.
        pitch: u32,
    },

    /// Attaching a buffer to the display failed.
    AddFramebuffer(io::Error),

    /// Setting the mode failed.
    SetMode {
        /// The mode that was attempted.
        mode: String,
        /// The CRTC id.
        crtc: u32,
        /// The underlying failure.
        source: io::Error,
    },

    /// Queueing a page flip failed.
    PageFlip(io::Error),

    /// Waiting for the flip to complete failed.
    WaitVblank(io::Error),
}

impl fmt::Display for DrmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => f.write_str("no DRM device found under /dev/dri"),
            Self::Open { path, .. } => write!(f, "opening {}", path.display()),
            Self::NoDisplayCapableDevice => f.write_str("no DRM device has a display output"),
            Self::NotMaster(_) => f.write_str(
                "cannot become DRM master — another process (a compositor, or another \
                 instance of this program) already holds it, or this process lacks the rights",
            ),
            Self::Resources(_) => f.write_str("reading DRM resources"),
            Self::Selection(err) => core::fmt::Display::fmt(err, f),
            Self::NoCrtc { connector } => write!(f, "no CRTC can drive connector {connector}"),
            Self::Allocate { width, height, .. } => {
                write!(f, "allocating a {width}x{height} dumb buffer")
            }
            Self::Map(_) => f.write_str("mapping a dumb buffer"),
            Self::UnalignedPitch { pitch } => write!(
                f,
                "dumb buffer pitch {pitch} is not a whole number of 32-bit pixels"
            ),
            Self::AddFramebuffer(_) => f.write_str("registering a framebuffer"),
            Self::SetMode { mode, crtc, .. } => write!(f, "setting mode {mode} on CRTC {crtc}"),
            Self::PageFlip(_) => f.write_str("queueing a page flip"),
            Self::WaitVblank(_) => f.write_str("waiting for vblank"),
        }
    }
}

impl std::error::Error for DrmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. }
            | Self::Allocate { source, .. }
            | Self::SetMode { source, .. }
            | Self::NotMaster(source)
            | Self::Resources(source)
            | Self::Map(source)
            | Self::AddFramebuffer(source)
            | Self::PageFlip(source)
            | Self::WaitVblank(source) => Some(source),
            Self::NoDevice
            | Self::NoDisplayCapableDevice
            | Self::Selection(_)
            | Self::NoCrtc { .. }
            | Self::UnalignedPitch { .. } => None,
        }
    }
}

impl From<SelectionError> for DrmError {
    fn from(err: SelectionError) -> Self {
        Self::Selection(err)
    }
}

impl From<DrmError> for SurfaceError {
    fn from(err: DrmError) -> Self {
        SurfaceError::backend(err)
    }
}
