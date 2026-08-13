//! What can go wrong between a file and a plane.

use std::path::PathBuf;

/// Video errors: enumeration, negotiation, streaming, scanout.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VideoError {
    /// A device node would not open.
    #[error("could not open {path}: {source}")]
    Open {
        /// The node that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },

    /// No decoder on this board accepts any of the offered assets.
    ///
    /// The menu is two codecs wide precisely so this stays unreachable on a
    /// Raspberry Pi — a kiosk that ships both files always has a playable one.
    #[error("no hardware decoder accepts any offered asset")]
    NothingPlayable,

    /// An ioctl against the decoder failed.
    #[error("V4L2 {what} failed: {source}")]
    V4l2 {
        /// Which call.
        what: &'static str,
        /// The errno.
        source: rustix::io::Errno,
    },

    /// The decoder produced a frame format the plane path does not handle.
    #[error("decoder produced unsupported pixel format {0:#010x}")]
    UnsupportedFormat(u32),

    /// The stream never yielded a decodable picture.
    #[error("the stream produced no decoded frames — not an Annex-B elementary stream?")]
    NoFrames,

    /// A DRM call on the plane path failed.
    #[error("DRM {what} failed: {source}")]
    Drm {
        /// Which call.
        what: &'static str,
        /// The underlying error.
        source: std::io::Error,
    },

    /// No video plane on the CRTC supports the decoder's output format.
    #[error("no DRM plane accepts the decoded format on this CRTC")]
    NoPlane,
}

impl VideoError {
    pub(crate) fn v4l2(what: &'static str, source: rustix::io::Errno) -> Self {
        Self::V4l2 { what, source }
    }

    pub(crate) fn drm(what: &'static str, source: impl Into<std::io::Error>) -> Self {
        Self::Drm {
            what,
            source: source.into(),
        }
    }
}
