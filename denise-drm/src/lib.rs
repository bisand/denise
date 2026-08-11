//! Linux DRM/KMS backend for Denise.
//!
//! Opens the display directly, sets a mode, and page-flips CPU-rendered dumb
//! buffers straight to the scanout engine. No compositor, no window server, no
//! GPU driver stack, no X.
//!
//! # What this is not doing, and why
//!
//! **No `gbm`.** GBM exists to allocate buffers a *GPU* renders into. Denise
//! renders with the CPU, so DRM dumb buffers are exactly the right allocation:
//! scanout-capable, CPU-mappable, and free of any C library. Adding GBM would drag
//! in libgbm and Mesa, which would end both the single-static-binary goal and easy
//! cross-compilation.
//!
//! **No atomic modesetting, yet.** Atomic buys three things: `FB_DAMAGE_CLIPS`,
//! plane composition, and tear-free guarantees. The first is worth little here —
//! a page flip swaps whole buffers, so damage saves rasterisation, not bandwidth,
//! and most drivers ignore the property anyway. The second has a legacy equivalent
//! for the one plane that matters, the hardware cursor. So the legacy path gets
//! this milestone working on real hardware at a third of the code, behind a seam
//! that atomic can take over when planes actually earn their keep.
//!
//! # Becoming DRM master
//!
//! Setting a mode requires being DRM master, and only one process can be. If a
//! compositor or another Denise process holds it, [`Card::become_master`] fails
//! with `EBUSY` or `EACCES` — that is the single most common reason a first run on
//! a Pi does nothing. Three ways to have the right:
//!
//! - Run on a bare VT with no display server. The usual kiosk deployment.
//! - Be handed a file descriptor by `libseat` or a systemd unit, via
//!   [`Card::from_fd`]. Preferred for anything that has to coexist.
//! - Run as root. Works, and is a poor way to ship a product.
//!
//! # Testing
//!
//! [`mode`] and [`swapchain`] are platform-independent on purpose and are unit
//! tested everywhere, including on machines with no DRM device. They hold the
//! decisions that are hard to debug in the field and easy to check on a laptop.
//! Everything else is a thin wrapper over ioctls and can only be proven on real
//! hardware.

pub mod mode;
pub mod swapchain;

pub use mode::{
    ConnectorInfo, ConnectorKind, ModeInfo, ModePreference, OutputPreference, Selection,
    SelectionError,
};
pub use swapchain::Swapchain;

#[cfg(target_os = "linux")]
mod device;
#[cfg(target_os = "linux")]
mod error;

#[cfg(target_os = "linux")]
pub use device::Card;
#[cfg(target_os = "linux")]
pub use error::DrmError;
