//! Fonts, a bounded glyph cache, line layout and word wrapping.
//!
//! One [`TextEngine`] holds every font an application uses and one [`GlyphAtlas`]
//! that caches what has been rasterised. Measurement and drawing both go through
//! it, so a label that is measured during layout and drawn a moment later
//! rasterises its glyphs exactly once.
//!
//! ```no_run
//! # use denise::{Color, Point};
//! # use denise_render::Pen;
//! # use denise_text::{TextEngine, TextStyle};
//! # fn demo(canvas: &mut Pen<'_>) {
//! let mut text = TextEngine::new();
//! let style = TextStyle::built_in(16);
//! let extent = text.measure(style, "Kjærlighet på Øy");
//! text.draw(canvas, style, Point::new(20, 20), "Kjærlighet på Øy", Color::WHITE);
//! # let _ = extent;
//! # }
//! ```
//!
//! # Wrapping
//!
//! [`TextEngine::wrap`] breaks a string into the lines it becomes at a given
//! width, greedily, returning slices that borrow the input. Explicit `\n` always
//! breaks. A word wider than the line overflows on a line of its own rather than
//! being cut between characters — breaking mid-word means knowing where a
//! grapheme ends, and half of an `æ` is nothing.
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
// Labels every feature-gated item on docs.rs with the feature it needs. Nightly
// only, and `docsrs` is set by nothing but docs.rs — an ordinary build never
// sees this line.
#![cfg_attr(docsrs, feature(doc_cfg))]
// `chunks_exact` over `as_chunks`, against clippy 1.98's advice: `as_chunks`
// stabilised in 1.98 and this workspace supports 1.95, so taking the advice
// would trade a style lint for a compile error on every older toolchain. Revisit
// when the MSRV passes 1.98. `unknown_lints` because the lint does not exist
// before 1.98 either, and naming an absent lint is itself a warning.
#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

extern crate alloc;

pub mod atlas;
pub mod bitmap;
pub mod engine;
#[cfg(feature = "shaping")]
pub mod shaped;
pub mod source;
#[cfg(feature = "truetype")]
pub mod truetype;

pub use atlas::{AtlasStats, GlyphAtlas, GlyphKey, Placed};
pub use bitmap::BitmapSource;
pub use engine::{PositionedGlyph, TextEngine, TextStyle};
#[cfg(feature = "shaping")]
pub use shaped::ShapedSource;
pub use source::{
    FontId, FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised, ShapedGlyph,
};
#[cfg(feature = "truetype")]
pub use truetype::TrueTypeSource;

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
