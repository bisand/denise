//! Denise's software rasteriser.
//!
//! Rectangles, rounded rectangles, circles, arcs, stars, lines, image blitting,
//! rectangular clipping and source-over alpha blending, straight into a
//! [`denise::Frame`]. No GPU, no
//! path builder, no allocator: this crate needs neither `std` nor `alloc`, and
//! every operation writes through a borrowed slice the caller already owns.
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
//!     // A 70% progress ring. Angles are binary turns: 0 is twelve o'clock,
//!     // clockwise positive, no radians and no floats anywhere.
//!     c.stroke_arc(Point::new(300, 90), 40, 8, 0, 70 * denise_render::TURN / 100, Color::WHITE);
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
pub mod coverage;
pub mod font;

mod arc;
// Public for its documentation rather than its contents: the module holds only
// `impl Canvas`, so its page carries no items -- but it is where the
// premultiplied source format is explained, and a private module renders
// nowhere at all.
pub mod blit;
pub mod icon;
pub mod painter;

pub use polygon::MAX_ICON_VERTICES;
mod line;
mod polygon;
mod rect;
mod rounded;

#[cfg(test)]
mod testing;

pub use arc::TURN;
pub use blend::Paint;
pub use canvas::{Canvas, PixelView};
pub use coverage::Mask;
pub use font::BitmapFont;
pub use painter::{ClipToken, Clipped, Painter, PainterExt, Pen};

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
