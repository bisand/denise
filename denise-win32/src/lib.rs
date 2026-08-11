//! A Denise panel in a Win32 child window.
//!
//! The oldest of the reasons this project exists. CoreCanvas shipped inside
//! Windows applications that were not going to be rewritten — MFC, WinForms, VB6
//! through the ActiveX shim — and the thing they all need is a control they can
//! put in a dialog next to the ones they already have.
//!
//! So: [`DeniseControl`] registers a window class and creates a child `HWND`. The
//! host owns the window, the message loop and the parent; Denise owns the pixels
//! inside one rectangle and nothing else.
//!
//! # What is different from the bare-metal backends
//!
//! - **The host owns the message loop.** There is no `run` function here. Windows
//!   decides when to paint and Denise answers, which is the opposite of the DRM
//!   backend where Denise decides and the display follows.
//! - **Damage is real bandwidth.** `BitBlt` moves only the rectangles it is given,
//!   unlike a DRM page flip where the whole buffer goes regardless. The tree's
//!   damage is worth passing on rather than rounding up to the client area.
//! - **The pixel format already matches.** A 32-bit `BI_RGB` DIB section is
//!   `0xAARRGGBB` in a little-endian `DWORD`, which is exactly what the rasteriser
//!   writes. No conversion pass anywhere.
//! - **There is already a cursor.** Windows draws one, so the composited sprite
//!   stays off: `Ui::show_cursor(false)`, which since M5 is a decision that sticks
//!   rather than one the next mouse move overrides.
//!
//! # Status
//!
//! Written against the documentation and compile-checked for
//! `x86_64-pc-windows-msvc`; **not yet run on Windows**. The parts most likely to
//! be wrong are the ones no compiler checks: message ordering, focus behaviour
//! inside a dialog, and DPI changes. See the README.

#![cfg(windows)]

mod control;
mod keymap;
mod surface;

pub use control::{ControlDelegate, DeniseControl};
pub use keymap::key_code;
pub use surface::DibSurface;

/// Failures from this backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A surface was asked for with no pixels in it.
    #[error("a surface needs a non-zero width and height")]
    EmptySurface,

    /// `CreateDIBSection` failed, or handed back no pixels.
    #[error("could not create a DIB section")]
    DibSection,

    /// `CreateCompatibleDC` failed.
    #[error("could not create a memory device context")]
    MemoryDc,

    /// The window class could not be registered.
    #[error("could not register the window class")]
    RegisterClass,

    /// `CreateWindowEx` failed.
    #[error("could not create the control window")]
    CreateWindow,

    /// A surface operation failed.
    #[error(transparent)]
    Surface(#[from] denise::SurfaceError),
}
