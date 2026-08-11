//! Denise's software rasteriser.
//!
//! Rectangles, rounded rectangles, lines, rectangular clipping and source-over
//! alpha blending, straight into a [`denise::Frame`]. No GPU, no path builder, no
//! allocator: this crate needs neither `std` nor `alloc`, and every operation
//! writes through a borrowed slice the caller already owns.
//!
//! ```no_run
//! # use denise::{Color, Frame, Rect, Point};
//! # use denise_render::Canvas;
//! # fn draw(frame: &mut Frame<'_>, damage: &[Rect]) {
//! let mut canvas = Canvas::new(frame);
//! for region in damage {
//!     // Everything inside this borrow is confined to `region`.
//!     let mut c = canvas.with_clip(*region);
//!     c.clear(Color::from_rgb888(0x1E1E2E));
//!     c.fill_rounded_rect(Rect::new(40, 40, 200, 64), 12, Color::rgba(255, 255, 255, 32));
//!     c.draw_line(Point::new(40, 120), Point::new(240, 121), Color::WHITE);
//! }
//! # }
//! ```
//!
//! # No floating point
//!
//! The rasteriser is integer throughout, including anti-aliasing coverage. That
//! avoids a `libm` dependency on `no_std` targets and keeps output bit-identical
//! between x86 and ARM, so the reference tests mean the same thing on a developer's
//! desktop and on the Pi.
//!
//! # No `unsafe`
//!
//! Deliberately, for now. Bounds checks are hoisted by working through row slices
//! rather than indexing pixel by pixel. If the benches ever show that costing real
//! time, that is the evidence needed to justify unchecked access — not before.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod blend;
pub mod canvas;

mod line;
mod rect;
mod rounded;

#[cfg(test)]
mod testing;

pub use blend::Paint;
pub use canvas::{Canvas, PixelView};
