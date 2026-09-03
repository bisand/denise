//! Denise — a direct-rendering UI toolkit for systems without a desktop environment.
//!
//! This crate is the platform-agnostic core. It owns geometry, colour, the pixel
//! buffer contract, input events and damage tracking. It contains no platform code
//! and no `unsafe`.
//!
//! Backends implement two traits:
//!
//! - [`Surface`] — hands out a [`Frame`] to draw into and presents damaged regions.
//! - [`InputSource`] — drains platform input into [`InputEvent`]s.
//!
//! # Frame lifecycle
//!
//! ```no_run
//! # use denise::{Surface, DamageTracker, Rect, MAX_DAMAGE_RECTS};
//! # fn demo(surface: &mut impl Surface, tracker: &mut DamageTracker) -> Result<(), denise::SurfaceError> {
//! let mut frame = surface.acquire()?;
//! // The buffer we just got back may be several frames old. Repaint everything
//! // that has been damaged since it was last current, not just this frame's damage.
//! let damage: &[Rect] = tracker.resolve(frame.age());
//! // ... draw into `frame`, clipped to `damage` ...
//! drop(frame);
//! surface.present(damage)?;
//! tracker.end_frame();
//! # Ok(())
//! # }
//! ```
//!
//! Getting that sequence wrong is the classic double-buffering bug: repaint only the
//! current frame's damage and every second frame shows stale content.
//!
//! # Theming
//!
//! Colours are named by **role** — [`Role::Primary`], [`Role::Base100`],
//! [`Role::Error`] — each with a `*Content` partner. [`Theme::pair`] returns a
//! surface and a foreground that are *guaranteed* to contrast, and
//! [`Theme::validate`] proves it for a custom theme rather than leaving it to be
//! discovered on a panel in daylight. The arithmetic is WCAG relative luminance in
//! integers, so it is identical on x86 and ARM.
//!
//! # Status
//!
//! M5 complete, M6 in progress. Everything through M5 has run on real hardware —
//! a Raspberry Pi on DRM/KMS, Windows 11 on ARM64, macOS — not only in CI. See the
//! README for the roadmap.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod angle;
pub mod color;
pub mod cursor;
pub mod damage;
pub mod geom;
pub mod icon;
pub mod input;
pub mod paint;
pub mod painter;
pub mod pixels;
pub mod surface;
pub mod theme;

pub use angle::TURN;
pub use color::Color;
pub use cursor::CursorPlane;
pub use damage::{DamageTracker, MAX_DAMAGE_RECTS, MAX_TRACKED_FRAMES, RectList};
pub use geom::{Point, Rect, Size};
pub use icon::{GRID, Icon, Ink, MAX_ICON_VERTICES, MAX_SHAPES, Shape};
pub use input::{
    ElementState, InputEvent, InputSource, KeyCode, Modifiers, PointerButton, TouchId,
};
pub use paint::Paint;
pub use painter::{ClipToken, Clipped, Painter, PainterExt, Pen};
pub use pixels::{Mask, PixelView};
pub use surface::{BufferAge, Frame, PixelFormat, Surface, SurfaceError, required_words};
pub use theme::{ColorScheme, Metrics, Radius, Role, Theme};

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
