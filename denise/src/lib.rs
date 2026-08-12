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
//! # Status
//!
//! M0. The abstraction exists and is proven against a desktop backend; the scene
//! graph, component model and hardware backends are not here yet. See the README.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod color;
pub mod cursor;
pub mod damage;
pub mod geom;
pub mod input;
pub mod surface;
pub mod theme;

pub use color::Color;
pub use cursor::CursorPlane;
pub use damage::{DamageTracker, MAX_DAMAGE_RECTS, RectList};
pub use geom::{Point, Rect, Size};
pub use input::{
    ElementState, InputEvent, InputSource, KeyCode, Modifiers, PointerButton, TouchId,
};
pub use surface::{BufferAge, Frame, PixelFormat, Surface, SurfaceError};
pub use theme::{ColorScheme, Metrics, Radius, Role, Theme};
