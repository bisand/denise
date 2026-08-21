//! Legacy Linux fbdev backend for Denise.
//!
//! # Read this before reaching for it
//!
//! This is a fallback, and on any current kernel it is a fallback to DRM through a
//! longer route. `/dev/fb0` on a modern system is almost always
//! `CONFIG_DRM_FBDEV_EMULATION` — DRM pretending to be fbdev. The Alpine VM this
//! was developed against reports its framebuffer's name as `virtio_gpudrmfb`, and
//! a Raspberry Pi running Bookworm reports `vc4drmfb`. Going through it means
//! giving up page flips, vsync and buffer age, to reach the same hardware
//! [`denise_drm`](../denise_drm/index.html) already drives properly.
//!
//! So prefer DRM. Use this when:
//!
//! - the kernel is old enough to predate a usable DRM driver for the panel,
//! - the panel has an fbdev driver and no DRM driver at all, which still happens
//!   with small SPI displays,
//! - or DRM master cannot be obtained and a degraded picture beats none.
//!
//! # What it costs
//!
//! No page flip and no vsync, so a frame can tear. Drawing goes through a shadow
//! buffer and only damaged rows are copied out, which keeps the tear as small as
//! the change that caused it — the only mitigation available here, and another
//! reason damage tracking belongs in the core rather than in a backend.
//!
//! # Permissions
//!
//! Writing to `/dev/fb*` needs the `video` group, or root.

// `chunks_exact` over `as_chunks`, against clippy 1.98's advice: `as_chunks`
// stabilised in 1.98 and this workspace supports 1.95, so taking the advice
// would trade a style lint for a compile error on every older toolchain. Revisit
// when the MSRV passes 1.98. `unknown_lints` because the lint does not exist
// before 1.98 either, and naming an absent lint is itself a warning.
#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

pub mod info;

pub use info::{FbInfo, FbInfoError, PixelLayout};

#[cfg(target_os = "linux")]
mod error;
#[cfg(target_os = "linux")]
mod surface;

#[cfg(target_os = "linux")]
pub use error::FbdevError;
#[cfg(target_os = "linux")]
pub use surface::FbdevSurface;

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
