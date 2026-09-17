//! What can go wrong between a file and a plane.

use std::path::PathBuf;

/// Video errors: enumeration, negotiation, streaming, scanout.
#[derive(Debug)]
#[non_exhaustive]
pub enum VideoError {
    /// A device node would not open.
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
    NothingPlayable,

    /// An ioctl against the decoder failed.
    V4l2 {
        /// Which call.
        what: &'static str,
        /// The errno.
        source: rustix::io::Errno,
    },

    /// The decoder produced a frame format the plane path does not handle.
    UnsupportedFormat(u32),

    /// The stream never yielded a decodable picture.
    NoFrames,

    /// A DRM call on the plane path failed.
    Drm {
        /// Which call.
        what: &'static str,
        /// The underlying error.
        source: std::io::Error,
    },

    /// No video plane on the CRTC supports the decoder's output format.
    NoPlane,
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { path, source } => {
                write!(f, "could not open {}: {source}", path.display())
            }
            Self::NothingPlayable => f.write_str("no hardware decoder accepts any offered asset"),
            Self::V4l2 { what, source } => write!(f, "V4L2 {what} failed: {source}"),
            Self::UnsupportedFormat(format) => write!(
                f,
                "decoder produced unsupported pixel format {format:#010x}"
            ),
            Self::NoFrames => f.write_str(
                "the stream produced no decoded frames — not an Annex-B elementary stream?",
            ),
            Self::Drm { what, source } => write!(f, "DRM {what} failed: {source}"),
            Self::NoPlane => f.write_str("no DRM plane accepts the decoded format on this CRTC"),
        }
    }
}

impl std::error::Error for VideoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Drm { source, .. } => Some(source),
            Self::V4l2 { source, .. } => Some(source),
            Self::NothingPlayable | Self::UnsupportedFormat(_) | Self::NoFrames | Self::NoPlane => {
                None
            }
        }
    }
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
