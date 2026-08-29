//! Hardware video decode onto a DRM plane.
//!
//! A kiosk playing a promo loop is a set-top box with a different sticker, and
//! this crate does what a set-top box does: compressed bytes go to the SoC's
//! decoder over V4L2 memory-to-memory, decoded frames come back as dmabufs,
//! and each dmabuf is imported as a DRM framebuffer and flipped onto a video
//! **plane** the display controller composites during scanout. The frame never
//! passes through `denise-render` at all; the UI keeps painting its own buffer
//! and the plane sits in the stack with it. Zero copies end to end, no ffmpeg,
//! no GStreamer, no C library — the same single-static-binary discipline as
//! `denise-drm`.
//!
//! # The format menu
//!
//! Two elementary streams, chosen so **every Raspberry Pi hardware-plays at
//! least one**: H.264 (Constrained Baseline/Main, Annex-B, `.h264`) for Pi
//! Zero through 4 and most other embedded SoCs, and HEVC (Main, `.h265`) for
//! Pi 4 and 5. Both yuv420, at most 1080p30. The board picks:
//! [`Decoders::detect`] asks the hardware, [`Decoders::pick`] applies the
//! rule, and a kiosk ships both files — two ffmpeg lines at build time
//! instead of one.
//!
//! No container, no demuxer, no seeking: play, loop and stop, which is what a
//! promo loop is. Audio is a different subsystem and deliberately absent.
//!
//! # What runs where
//!
//! Everything that talks to `/dev` is Linux-only and `cfg`-gated to nothing
//! elsewhere. The Annex-B access-unit logic in [`annexb`] is pure and
//! compiled — and tested — everywhere.
//!
//! # Status
//!
//! The **stateful** decode path (H.264 via `bcm2835-codec` on the Pi, and its
//! equivalents on i.MX, Rockchip and Amlogic), verified end to end on a Pi 3A+:
//! access units in, dmabuf out, imported as a DRM framebuffer and flipped onto
//! a plane at a paced 29.5 fps over a live UI surface.
//!
//! The **stateless** HEVC path for `rpivid` — what the Pi 5 needs, and an order
//! of magnitude more work: slice parsing, reference management, the media
//! request API — is [#36](https://github.com/bisand/denise/issues/36).
//! [`Decoders::detect`] already reports it where the hardware offers it, so a
//! board with `rpivid` and no `bcm2835-codec` will say so and then decline to
//! play, which is the honest answer until #36 lands.

// Off Linux, some of the items the documentation above links to are compiled
// out, so those links resolve to nothing and `cargo doc` fails. CI documents on
// Ubuntu and never sees it; a developer on a Mac cannot avoid it. Linux stays
// the platform that checks these links, being the one with the items to check
// them against.
#![cfg_attr(not(target_os = "linux"), allow(rustdoc::broken_intra_doc_links))]

pub mod annexb;

#[cfg(target_os = "linux")]
mod decode;
#[cfg(target_os = "linux")]
mod detect;
#[cfg(target_os = "linux")]
mod error;
#[cfg(target_os = "linux")]
mod plane;
#[cfg(target_os = "linux")]
mod player;
/// The raw uapi layer. Hidden rather than private so the probe example can
/// narrate each enumeration step — a board where detection fails needs the
/// errno, not a shrug.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod v4l2;

#[cfg(target_os = "linux")]
pub use decode::{DecodedFrame, Decoder};
#[cfg(target_os = "linux")]
pub use detect::{Asset, DecoderInfo, Decoders};
#[cfg(target_os = "linux")]
pub use error::VideoError;
#[cfg(target_os = "linux")]
pub use plane::VideoPlane;
#[cfg(target_os = "linux")]
pub use player::Player;

/// Compiles the examples in this crate's README, so they cannot drift from the
/// API they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
