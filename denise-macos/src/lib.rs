//! An embeddable Cocoa view backend for Denise.
//!
//! Not a way to ship Denise on a Mac — [`denise_winit`] already previews on one,
//! and a desktop application should use a desktop toolkit. This exists for the
//! same reason the Win32 control does: an existing Cocoa application that wants a
//! Denise panel *inside* it, next to its own views, with the host owning the
//! window and the run loop.
//!
//! ```no_run
//! # #[cfg(target_os = "macos")]
//! # fn demo() -> Result<(), denise_macos::Error> {
//! use denise::Size;
//! use denise_macos::ViewSurface;
//!
//! // The host has a view; Denise has a surface the size of its backing store.
//! let mut surface = ViewSurface::new(Size::new(800, 480), 2.0)?;
//! # let _ = &mut surface;
//! # Ok(())
//! # }
//! ```
//!
//! # What is different from the bare-metal backends
//!
//! - **The host owns the run loop.** There is no `run` function here. AppKit
//!   decides when to draw and Denise answers, which is the opposite of the DRM
//!   backend where Denise decides and the display follows.
//! - **Damage is real.** `setNeedsDisplayInRect:` genuinely limits what gets
//!   composited, unlike a DRM page flip where the whole buffer goes regardless.
//!   So the rectangles the tree produces are worth passing on rather than
//!   rounding up to the whole view.
//! - **Points are not pixels.** A Retina view is 2 physical pixels per point.
//!   Denise lays out in physical pixels throughout — the conversion happens once,
//!   at this edge, and nothing above it needs the scale factor to hit-test.
//! - **There is already a cursor.** The host's window system draws one, so the
//!   composited sprite must stay off: `Ui::show_cursor(false)`, which since M5 is
//!   a decision that sticks rather than one the next mouse move overrides.

#![cfg(target_os = "macos")]

mod keymap;
mod surface;
mod view;

pub use keymap::key_code;
pub use surface::ViewSurface;
pub use view::{DeniseView, ViewDelegate, ViewState};

/// Failures from this backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A surface was asked for with no pixels in it.
    #[error("a surface needs a non-zero width and height")]
    EmptySurface,

    /// `CGColorSpaceCreateDeviceRGB` returned null, which should not happen and
    /// leaves nothing sensible to fall back to.
    #[error("could not create a device RGB colour space")]
    ColorSpace,

    /// `CGBitmapContextCreate` failed, or produced a pitch that is not a whole
    /// number of 32-bit words.
    #[error("could not create a bitmap context")]
    BitmapContext,

    /// A surface operation failed.
    #[error(transparent)]
    Surface(#[from] denise::SurfaceError),
}

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
