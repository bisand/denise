//! What this board's hardware decodes, asked rather than guessed.
//!
//! A V4L2 decoder *enumerates* the compressed formats it accepts
//! (`VIDIOC_ENUM_FMT` on the output queue), so detection is a walk over
//! `/dev/video*` — the same walk the `probe` example prints. On a Pi this
//! finds `bcm2835-codec-decode` (H.264, Pi Zero through 4) and `rpivid`
//! (HEVC, Pi 4 and 5); on other SoCs, whatever their vendor shipped.

use std::path::{Path, PathBuf};

use crate::annexb::Codec;
use crate::v4l2;

/// Where video nodes live.
const DEV_DIR: &str = "/dev";

/// One decodable asset an application offers: a codec and the file that holds
/// its elementary stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// Which of the menu's two codecs the file contains.
    pub codec: Codec,
    /// The elementary stream (`.h264` / `.h265`).
    pub path: PathBuf,
}

impl Asset {
    /// An H.264 elementary stream.
    pub fn h264(path: impl Into<PathBuf>) -> Self {
        Self {
            codec: Codec::H264,
            path: path.into(),
        }
    }

    /// An HEVC elementary stream.
    pub fn h265(path: impl Into<PathBuf>) -> Self {
        Self {
            codec: Codec::H265,
            path: path.into(),
        }
    }
}

/// One decoder node and what it accepts.
#[derive(Clone, Debug)]
pub struct DecoderInfo {
    /// The device node, `/dev/videoN`.
    pub path: PathBuf,
    /// The driver's name, as it reports it.
    pub driver: String,
    /// Whether the compressed queue accepts H.264.
    pub h264: bool,
    /// Whether the compressed queue accepts HEVC.
    pub hevc: bool,
    /// Whether the decoder is **stateful** — feed bytes, frames come out.
    ///
    /// A stateless decoder (`rpivid`) advertises the codec but needs
    /// userspace to parse slices and drive the request API; this crate's
    /// stateful path must not be pointed at one. Heuristic: stateless
    /// drivers expose their controls and are known by name.
    pub stateful: bool,
}

/// The board's decoders, enumerated once.
#[derive(Clone, Debug, Default)]
pub struct Decoders {
    /// Every M2M decoder found, in `/dev/video*` order.
    pub found: Vec<DecoderInfo>,
}

impl Decoders {
    /// Walks `/dev/video*` and asks each node what it is.
    ///
    /// Nodes that refuse to open or are not memory-to-memory decoders are
    /// skipped silently — a camera is not an error, it is a camera.
    pub fn detect() -> Self {
        Self::detect_in(DEV_DIR)
    }

    /// [`Decoders::detect`] against an alternate `/dev`, for tests.
    pub fn detect_in(dev: impl AsRef<Path>) -> Self {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dev.as_ref()) else {
            return Self { found };
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("video") && n[5..].parse::<u32>().is_ok())
            })
            .collect();
        paths.sort();
        for path in paths {
            if let Some(info) = Self::inspect(&path) {
                found.push(info);
            }
        }
        Self { found }
    }

    /// Asks one node whether it is a decoder, and for what.
    fn inspect(path: &Path) -> Option<DecoderInfo> {
        use std::os::fd::AsFd;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .ok()?;
        let fd = file.as_fd();
        let cap = v4l2::querycap(fd).ok()?;
        let caps = cap.caps();
        if caps & v4l2::CAP_VIDEO_M2M_MPLANE == 0 || caps & v4l2::CAP_STREAMING == 0 {
            return None;
        }
        let (mut h264, mut hevc, mut any_compressed) = (false, false, false);
        for index in 0.. {
            match v4l2::enum_fmt(fd, v4l2::BUF_TYPE_OUTPUT_MPLANE, index).ok()? {
                None => break,
                Some(desc) => {
                    if desc.flags & v4l2::FMT_FLAG_COMPRESSED != 0 {
                        any_compressed = true;
                    }
                    match desc.pixelformat {
                        v4l2::PIX_FMT_H264 => h264 = true,
                        v4l2::PIX_FMT_HEVC => hevc = true,
                        _ => {}
                    }
                }
            }
        }
        // A decoder takes compressed in; an encoder takes it out. A node with
        // no compressed input format is not a decoder.
        if !any_compressed {
            return None;
        }
        let driver = cap.driver_name().to_owned();
        // The known stateless drivers. Wrong-by-default is the safe polarity:
        // an unknown stateless driver marked stateful fails loudly at S_FMT,
        // where an unknown stateful driver marked stateless would silently
        // never be used.
        let stateful = !matches!(
            driver.as_str(),
            "rpivid" | "hantro-vpu" | "rkvdec" | "cedrus"
        );
        Some(DecoderInfo {
            path: path.to_path_buf(),
            driver,
            h264,
            hevc,
            stateful,
        })
    }

    /// The node the **stateful** path uses for `codec`, if any.
    pub fn stateful_for(&self, codec: Codec) -> Option<&DecoderInfo> {
        self.found.iter().find(|d| {
            d.stateful
                && match codec {
                    Codec::H264 => d.h264,
                    Codec::H265 => d.hevc,
                }
        })
    }

    /// The menu's rule: the first offered asset this board's stateful path
    /// plays.
    ///
    /// A kiosk offers both files; the board picks. Order expresses the
    /// application's preference when a board plays both.
    pub fn pick<'a>(&self, assets: &'a [Asset]) -> Option<(&'a Asset, &DecoderInfo)> {
        assets
            .iter()
            .find_map(|asset| self.stateful_for(asset.codec).map(|d| (asset, d)))
    }
}
