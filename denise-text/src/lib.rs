//! Fonts, a bounded glyph cache, and line layout.
//!
//! One [`TextEngine`] holds every font an application uses and one [`GlyphAtlas`]
//! that caches what has been rasterised. Measurement and drawing both go through
//! it, so a label that is measured during layout and drawn a moment later
//! rasterises its glyphs exactly once.
//!
//! ```no_run
//! # use denise::{Color, Point};
//! # use denise_render::Canvas;
//! # use denise_text::{TextEngine, TextStyle};
//! # fn demo(canvas: &mut Canvas<'_>) {
//! let mut text = TextEngine::new();
//! let style = TextStyle::built_in(16);
//! let extent = text.measure(style, "Kjærlighet på Øy");
//! text.draw(canvas, style, Point::new(20, 20), "Kjærlighet på Øy", Color::WHITE);
//! # let _ = extent;
//! # }
//! ```
//!
//! # Three tiers, and what each costs
//!
//! Measured as the increase in a stripped, statically linked
//! `aarch64-unknown-linux-musl` binary:
//!
//! | Tier | Feature | Cost | What it buys |
//! |---|---|---|---|
//! | Built-in bitmap | none | 0 | Latin plus `æøå`, whole-number scales |
//! | TrueType | `truetype` | +145 KB | Real fonts, proportional metrics, anti-aliasing |
//! | Shaped | `shaping` | +3.1 MB | Ligatures, bidi, complex scripts, font fallback |
//!
//! For scale: the whole of Denise, DRM, evdev and the widgets is about 840 KB, so
//! the shaping tier is four times the rest of the toolkit put together. It is
//! there because some panels genuinely need it, and off by default because most
//! do not — a temperature readout and a Norwegian name do not need a shaper.
//!
//! # What this is not
//!
//! Not a text editor's model: no bidi cursor movement, no grapheme-cluster
//! segmentation, no line breaking by dictionary. `\n` breaks a line and nothing
//! else does. Those belong to whoever turns this into a document viewer.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod atlas;
pub mod bitmap;
pub mod engine;
pub mod source;
#[cfg(feature = "truetype")]
pub mod truetype;

pub use atlas::{AtlasStats, GlyphAtlas, GlyphKey, Placed};
pub use bitmap::BitmapSource;
pub use engine::{PositionedGlyph, TextEngine, TextStyle};
pub use source::{FontId, FontMetrics, GlyphMetrics, GlyphSource, Rasterised};
#[cfg(feature = "truetype")]
pub use truetype::TrueTypeSource;
