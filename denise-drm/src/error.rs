//! Failures from the DRM backend.

use std::io;
use std::path::PathBuf;

use denise::SurfaceError;

use crate::mode::SelectionError;

/// Something went wrong talking to the display.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DrmError {
    /// `/dev/dri` holds no card node.
    #[error("no DRM device found under /dev/dri")]
    NoDevice,

    /// A card node could not be opened.
    #[error("opening {path}")]
    Open {
        /// The device that failed.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// Every card node was opened but none can drive a display.
    ///
    /// Usually means only render nodes are present — a headless GPU, or a card
    /// whose display side is claimed by another driver.
    #[error("no DRM device has a display output")]
    NoDisplayCapableDevice,

    /// This process could not become DRM master.
    ///
    /// Only one process at a time may set modes. See the crate documentation: run
    /// on a bare VT, or take a descriptor from `libseat` or systemd.
    #[error(
        "cannot become DRM master — another process (a compositor, or another \
         instance of this program) already holds it, or this process lacks the rights"
    )]
    NotMaster(#[source] io::Error),

    /// Enumerating connectors, encoders and CRTCs failed.
    #[error("reading DRM resources")]
    Resources(#[source] io::Error),

    /// No output could be chosen.
    #[error(transparent)]
    Selection(#[from] SelectionError),

    /// The chosen connector has no CRTC that can drive it.
    #[error("no CRTC can drive connector {connector}")]
    NoCrtc {
        /// The connector id that could not be routed.
        connector: u32,
    },

    /// A scanout buffer could not be allocated.
    #[error("allocating a {width}x{height} dumb buffer")]
    Allocate {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// A scanout buffer could not be mapped into this process.
    #[error("mapping a dumb buffer")]
    Map(#[source] io::Error),

    /// The driver handed back a row pitch that is not a whole number of pixels.
    ///
    /// Denise addresses buffers as `u32` words, so a pitch that is not a multiple
    /// of four bytes cannot be expressed as a pixel stride.
    #[error("dumb buffer pitch {pitch} is not a whole number of 32-bit pixels")]
    UnalignedPitch {
        /// The pitch the driver reported, in bytes.
        pitch: u32,
    },

    /// Attaching a buffer to the display failed.
    #[error("registering a framebuffer")]
    AddFramebuffer(#[source] io::Error),

    /// Setting the mode failed.
    #[error("setting mode {mode} on CRTC {crtc}")]
    SetMode {
        /// The mode that was attempted.
        mode: String,
        /// The CRTC id.
        crtc: u32,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },

    /// Queueing a page flip failed.
    #[error("queueing a page flip")]
    PageFlip(#[source] io::Error),

    /// Waiting for the flip to complete failed.
    #[error("waiting for vblank")]
    WaitVblank(#[source] io::Error),
}

impl From<DrmError> for SurfaceError {
    fn from(err: DrmError) -> Self {
        SurfaceError::backend(err)
    }
}
