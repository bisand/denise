//! A face rasterised in advance, as a glyph source.
//!
//! The `truetype` tier reads a font on the panel: about 270 KB of parser, run
//! at boot, to work out an outline of `A` at 16 px that was the same yesterday
//! and will be the same next year, because the font is compiled into the binary.
//! This tier does that work once, on the developer's machine, and compiles in
//! the answer instead: for each size the UI uses, every glyph's metrics and its
//! coverage, as plain tables. On the panel a glyph is a binary search and a
//! slice. No parser, no outlines, no external crate, and nothing to do at boot.
//!
//! What it gives up is anything the panel did not know at build time: a size
//! that was not baked snaps to the nearest one that was, a character that was
//! not baked draws as the box, and a font the user brings at run time is the
//! `truetype` tier's business — the two coexist, and
//! [`TextEngine::add_font`](crate::TextEngine::add_font) takes either.
//!
//! The tables come from the `bake` feature's [`bake`](crate::bake) module,
//! normally from a `build.rs`, and land in a `static` a program names:
//!
//! ```ignore
//! include!(concat!(env!("OUT_DIR"), "/inter.rs")); // `static INTER: BakedFont`
//! let inter = engine.add_font(Box::new(BakedSource::new(&INTER)));
//! ```
//!
//! # What it costs
//!
//! Flash, in proportion to what is baked: a glyph's coverage is one byte per
//! pixel of its box, so a Latin face at 16 px is around 15 KB and at 32 px
//! four times that, plus twenty bytes of table per glyph. It sits in the
//! binary's read-only data, so at run time it costs only the pages that are
//! touched, and a face that draws in three sizes is far smaller than the
//! font file it was baked from would have been.

use denise::Size;

use crate::source::{FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised};

/// A face baked at one or more sizes. Generated; not meant to be written by hand.
#[derive(Debug)]
pub struct BakedFont {
    /// What the face was called when it was baked.
    pub name: &'static str,
    /// Ascending by [`BakedSize::size_px`].
    pub sizes: &'static [BakedSize],
    /// Every glyph's coverage, one after another, one byte per pixel and one
    /// row after the next. A [`BakedGlyph`] says where its own starts.
    pub coverage: &'static [u8],
}

/// One size of a [`BakedFont`].
#[derive(Debug)]
pub struct BakedSize {
    /// Pixels to the em: what the text engine asks for.
    pub size_px: u16,
    /// The face's line metrics at this size.
    pub metrics: FontMetrics,
    /// Ascending by [`BakedGlyph::ch`]. `'\0'` is the box, drawn for a
    /// character the face has not got.
    pub glyphs: &'static [BakedGlyph],
}

/// One glyph at one size. The metrics are [`GlyphMetrics`] in halves, since no
/// baked glyph is thirty thousand pixels across and the table is most of what
/// a small size costs.
#[derive(Clone, Copy, Debug)]
pub struct BakedGlyph {
    /// The character, or `'\0'` for the box.
    pub ch: char,
    /// How far the pen moves on, in pixels.
    pub advance: i16,
    /// From the pen to the left of the ink.
    pub bearing_x: i16,
    /// From the baseline up to the top of the ink.
    pub bearing_y: i16,
    /// The ink's box.
    pub width: u16,
    /// The ink's box.
    pub height: u16,
    /// Where its coverage starts in [`BakedFont::coverage`].
    pub offset: u32,
}

impl BakedGlyph {
    pub(crate) const fn metrics(&self) -> GlyphMetrics {
        GlyphMetrics {
            advance: self.advance as i32,
            bearing_x: self.bearing_x as i32,
            bearing_y: self.bearing_y as i32,
            size: Size::new(self.width as u32, self.height as u32),
        }
    }
}

impl BakedSize {
    fn glyph(&self, ch: char) -> Option<&BakedGlyph> {
        self.glyphs
            .binary_search_by_key(&ch, |glyph| glyph.ch)
            .ok()
            .map(|index| &self.glyphs[index])
    }
}

/// A [`BakedFont`], drawn.
#[derive(Debug)]
pub struct BakedSource {
    font: &'static BakedFont,
}

impl BakedSource {
    /// Draws from `font`, which a `bake` generated.
    pub const fn new(font: &'static BakedFont) -> Self {
        Self { font }
    }

    /// The size that will actually be drawn for a requested one: the nearest
    /// baked, and the smaller of two equally near, since text that comes out
    /// smaller than asked still fits where it was put.
    fn nearest(&self, size_px: u16) -> Option<&BakedSize> {
        self.font.sizes.iter().min_by_key(|size| {
            let distance = size.size_px.abs_diff(size_px);
            (distance, size.size_px)
        })
    }

    fn lookup(&self, glyph: GlyphId, size_px: u16) -> Option<&BakedGlyph> {
        self.nearest(size_px)?.glyph(glyph.as_char()?)
    }
}

impl GlyphSource for BakedSource {
    fn name(&self) -> &str {
        self.font.name
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        self.nearest(size_px)
            .map(|size| size.metrics)
            .unwrap_or_default()
    }

    fn glyph_metrics(&mut self, glyph: GlyphId, size_px: u16) -> Option<GlyphMetrics> {
        self.lookup(glyph, size_px).map(BakedGlyph::metrics)
    }

    fn rasterise(&mut self, glyph: GlyphId, size_px: u16) -> Option<Rasterised<'_>> {
        let glyph = self.lookup(glyph, size_px)?;
        let metrics = glyph.metrics();
        let (start, len) = (
            glyph.offset as usize,
            glyph.width as usize * glyph.height as usize,
        );
        // A table that points past its own coverage was not generated by the
        // bake; a glyph with no ink is the honest answer.
        let coverage = self.font.coverage.get(start..start + len).unwrap_or(&[]);
        Some(Rasterised {
            metrics: if coverage.len() == len {
                metrics
            } else {
                GlyphMetrics {
                    size: Size::new(0, 0),
                    ..metrics
                }
            },
            coverage,
            stride: glyph.width as usize,
        })
    }

    fn contains(&self, ch: char) -> bool {
        // Baked at every size or none, so the first size answers for all.
        ch != '\0'
            && self
                .font
                .sizes
                .first()
                .is_some_and(|size| size.glyph(ch).is_some())
    }

    fn fallback_id(&self, _ch: char) -> Option<GlyphId> {
        Some(GlyphId::from_char('\0'))
    }

    fn snap_size(&self, size_px: u16) -> u16 {
        self.nearest(size_px).map_or(size_px, |size| size.size_px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two sizes of a face with `a`, `b` and the box. At 8 px `a` is a 2×2
    // square of full ink and `b` an empty advance; at 16 px `a` is 4×4.
    static TINY: BakedFont = BakedFont {
        name: "tiny",
        sizes: &[
            BakedSize {
                size_px: 8,
                metrics: FontMetrics {
                    ascent: 6,
                    descent: 2,
                    line_gap: 1,
                },
                glyphs: &[
                    BakedGlyph {
                        ch: '\0',
                        advance: 3,
                        bearing_x: 0,
                        bearing_y: 2,
                        width: 2,
                        height: 2,
                        offset: 0,
                    },
                    BakedGlyph {
                        ch: 'a',
                        advance: 3,
                        bearing_x: 0,
                        bearing_y: 2,
                        width: 2,
                        height: 2,
                        offset: 4,
                    },
                    BakedGlyph {
                        ch: 'b',
                        advance: 3,
                        bearing_x: 0,
                        bearing_y: 0,
                        width: 0,
                        height: 0,
                        offset: 8,
                    },
                ],
            },
            BakedSize {
                size_px: 16,
                metrics: FontMetrics {
                    ascent: 12,
                    descent: 4,
                    line_gap: 2,
                },
                glyphs: &[
                    BakedGlyph {
                        ch: '\0',
                        advance: 6,
                        bearing_x: 0,
                        bearing_y: 4,
                        width: 4,
                        height: 4,
                        offset: 8,
                    },
                    BakedGlyph {
                        ch: 'a',
                        advance: 6,
                        bearing_x: 1,
                        bearing_y: 4,
                        width: 4,
                        height: 4,
                        offset: 24,
                    },
                    BakedGlyph {
                        ch: 'b',
                        advance: 6,
                        bearing_x: 0,
                        bearing_y: 0,
                        width: 0,
                        height: 0,
                        offset: 40,
                    },
                ],
            },
        ],
        coverage: &[
            128, 128, 128, 128, // the 8 px box
            255, 255, 255, 255, // `a` at 8 px
            128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128,
            128, // box, 16
            255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
            255, // `a`, 16
        ],
    };

    #[test]
    fn a_size_snaps_to_the_nearest_baked_one_and_downwards_on_a_tie() {
        let source = BakedSource::new(&TINY);
        assert_eq!(source.snap_size(8), 8);
        assert_eq!(source.snap_size(9), 8);
        assert_eq!(source.snap_size(15), 16);
        assert_eq!(source.snap_size(12), 8, "half way: the smaller still fits");
        assert_eq!(source.snap_size(40), 16, "nothing bigger to give");
        assert_eq!(source.metrics(40).ascent, 12);
    }

    #[test]
    fn a_glyph_is_a_slice_of_the_table_and_its_metrics_come_with_it() {
        let mut source = BakedSource::new(&TINY);
        let a = source.glyph_id('a').expect("baked");
        let drawn = source.rasterise(a, 16).expect("ink");
        assert_eq!(drawn.coverage, [255; 16]);
        assert_eq!(drawn.stride, 4);
        assert_eq!(
            drawn.metrics,
            GlyphMetrics {
                advance: 6,
                bearing_x: 1,
                bearing_y: 4,
                size: Size::new(4, 4),
            }
        );
        let b = source.glyph_id('b').expect("baked");
        let drawn = source.rasterise(b, 8).expect("an advance");
        assert!(drawn.coverage.is_empty());
        assert_eq!(drawn.metrics.advance, 3);
    }

    #[test]
    fn a_character_that_was_not_baked_is_the_box() {
        let mut source = BakedSource::new(&TINY);
        assert!(source.contains('a'));
        assert!(!source.contains('z'));
        assert!(!source.contains('\0'), "the box is not a character");
        assert_eq!(source.glyph_id('z'), None);
        let fallback = source.fallback_id('z').expect("the box");
        let drawn = source.rasterise(fallback, 8).expect("the box has ink");
        assert_eq!(drawn.coverage, [128; 4]);
    }

    #[test]
    fn a_table_that_points_past_its_coverage_draws_nothing_rather_than_panicking() {
        static BROKEN: BakedFont = BakedFont {
            name: "broken",
            sizes: &[BakedSize {
                size_px: 8,
                metrics: FontMetrics {
                    ascent: 6,
                    descent: 2,
                    line_gap: 0,
                },
                glyphs: &[BakedGlyph {
                    ch: 'a',
                    advance: 3,
                    bearing_x: 0,
                    bearing_y: 2,
                    width: 2,
                    height: 2,
                    offset: 1000,
                }],
            }],
            coverage: &[255; 4],
        };
        let mut source = BakedSource::new(&BROKEN);
        let a = source.glyph_id('a').expect("listed");
        let drawn = source.rasterise(a, 8).expect("listed");
        assert!(drawn.coverage.is_empty());
        assert!(drawn.metrics.is_blank());
        assert_eq!(drawn.metrics.advance, 3, "it still takes its space");
    }

    #[test]
    fn a_font_with_no_sizes_answers_nothing_and_panics_never() {
        static EMPTY: BakedFont = BakedFont {
            name: "empty",
            sizes: &[],
            coverage: &[],
        };
        let mut source = BakedSource::new(&EMPTY);
        assert_eq!(source.snap_size(16), 16);
        assert_eq!(source.metrics(16), FontMetrics::default());
        assert!(!source.contains('a'));
        assert!(source.rasterise(GlyphId::from_char('a'), 16).is_none());
    }
}
