//! Real TrueType and OpenType fonts, behind the `truetype` feature.
//!
//! Anti-aliased, proportionally spaced, at any size — everything the built-in
//! bitmap font cannot do, for about 65 KB of static binary. What it still does
//! not do is *shaping*: no ligatures, no contextual forms, no reordering, no
//! bidirectional text. For Latin, Cyrillic and Greek that costs nothing anyone
//! will notice. For Arabic or Devanagari it is the whole ball game, and those need
//! the `shaping` feature and its three megabytes.
//!
//! # Where the font comes from
//!
//! You supply the bytes. There is no font discovery, no fontconfig and no system
//! font directory, because a device that boots from flash with a read-only root
//! very often has no fonts installed at all, and a UI that renders nothing on a
//! customer's hardware because a directory was empty is not a UI.
//!
//! ```ignore
//! static INTER: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
//! let source = TrueTypeSource::from_static("Inter", INTER)?;
//! let inter = engine.add_font(Box::new(source));
//! ```
//!
//! Embedding with `include_bytes!` is the deployment this is designed for: the
//! font is in the binary, so there is exactly one file to copy to the device and
//! no way for it to arrive without its font.
//!
//! # What a face costs
//!
//! About what its file does. A glyph's outline is read from the font the first
//! time that glyph is drawn at a size, and the atlas keeps the bitmap, so a face
//! in memory is its bytes and a few tables — not every glyph it has, worked out
//! in advance. For a Nerd Font, whose twelve thousand glyphs are mostly icons
//! nobody draws, that is 3 MB rather than 60 MB, and opening it takes a
//! millisecond rather than most of a second on a slow core.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ab_glyph::{Font, FontRef, FontVec, OutlinedGlyph, PxScale, PxScaleFactor, point};
use denise::Size;

use crate::source::{FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised};

/// What a face is read from: bytes the source owns, or bytes that live as long
/// as the program does and need no copy.
enum Face {
    Owned(FontVec),
    // A parsed face's tables are a few kilobytes, which `FontVec` keeps boxed.
    Static(Box<FontRef<'static>>),
}

/// A TrueType or OpenType face, read glyph by glyph as it is drawn.
pub struct TrueTypeSource {
    name: String,
    face: Face,
    scratch: Vec<u8>,
}

impl TrueTypeSource {
    /// Reads a font from its bytes, which are copied: the face keeps them.
    ///
    /// Returns the parser's complaint on failure, so a panel can log *why* its
    /// font did not load and fall back to the built-in one rather than showing
    /// nothing.
    pub fn from_bytes(name: &str, data: &[u8]) -> Result<Self, String> {
        Self::from_vec(name, data.to_vec())
    }

    /// Reads a font from bytes read for it, which the face takes without a copy.
    pub fn from_vec(name: &str, data: Vec<u8>) -> Result<Self, String> {
        let font = FontVec::try_from_vec(data).map_err(|e| e.to_string())?;
        Ok(Self::with(name, Face::Owned(font)))
    }

    /// Reads a font from bytes that live as long as the program — an
    /// `include_bytes!` — which are neither copied nor held twice.
    pub fn from_static(name: &str, data: &'static [u8]) -> Result<Self, String> {
        let font = FontRef::try_from_slice(data).map_err(|e| e.to_string())?;
        Ok(Self::with(name, Face::Static(Box::new(font))))
    }

    fn with(name: &str, face: Face) -> Self {
        Self {
            name: name.to_owned(),
            face,
            scratch: Vec::new(),
        }
    }

    fn font(&self) -> &dyn Font {
        match &self.face {
            Face::Owned(font) => font,
            Face::Static(font) => &**font,
        }
    }

    /// Pixels per font unit at `size_px` pixels to the em, which is what this
    /// trait's sizes are, as a CSS `font-size` is.
    fn factor(&self, size_px: u16) -> f32 {
        // A face that does not say how big its em is is broken; 1000 units is
        // what most of the ones that forget would have said.
        f32::from(size_px) / self.font().units_per_em().unwrap_or(1000.0)
    }

    /// `glyph` at `size_px`, outlined on the baseline at the origin, and its
    /// advance. The outline is `None` for a glyph with no ink, such as a space,
    /// and the whole answer is `None` for an id no face can have.
    fn outline(&self, glyph: GlyphId, size_px: u16) -> Option<(i32, Option<OutlinedGlyph>)> {
        let id = ab_glyph::GlyphId(u16::try_from(glyph.0).ok()?);
        let font = self.font();
        let factor = self.factor(size_px);
        // Advances are fractional and pixels are not. Rounding here rather
        // than accumulating in floats keeps a line's width reproducible and
        // keeps the rasteriser's no-floating-point promise intact everywhere
        // except the one crate that has to break it.
        let advance = round(font.h_advance_unscaled(id) * factor);
        let outlined = font.outline(id).map(|outline| {
            // ab_glyph's scale is the height from descender to ascender rather
            // than the em, so the em is turned into one.
            let scale = PxScale::from(factor * font.height_unscaled());
            let positioned = id.with_scale_and_position(scale, point(0.0, 0.0));
            let factor = PxScaleFactor {
                horizontal: factor,
                vertical: factor,
            };
            OutlinedGlyph::new(positioned, outline, factor)
        });
        Some((advance, outlined))
    }
}

/// `value` to the nearest whole pixel, halves away from zero: `f32::round`,
/// which `core` does not have.
fn round(value: f32) -> i32 {
    if value < 0.0 {
        (value - 0.5) as i32
    } else {
        (value + 0.5) as i32
    }
}

/// The metrics of a glyph from the pixels its outline covers, which ab_glyph
/// gives with y growing downwards from the baseline. This trait wants the top
/// as a distance above the baseline.
fn convert(advance: i32, outlined: Option<&OutlinedGlyph>) -> GlyphMetrics {
    let Some(bounds) = outlined.map(OutlinedGlyph::px_bounds) else {
        return GlyphMetrics {
            advance,
            bearing_x: 0,
            bearing_y: 0,
            size: Size::new(0, 0),
        };
    };
    GlyphMetrics {
        advance,
        bearing_x: bounds.min.x as i32,
        bearing_y: -bounds.min.y as i32,
        size: Size::new(bounds.width() as u32, bounds.height() as u32),
    }
}

impl GlyphSource for TrueTypeSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        let font = self.font();
        let (ascent, descent) = (font.ascent_unscaled(), font.descent_unscaled());
        if ascent == 0.0 && descent == 0.0 {
            // A face with no vertical metrics is broken, but guessing from the
            // requested size beats returning zeroes and stacking every line on
            // top of the last.
            return FontMetrics {
                ascent: i32::from(size_px) * 4 / 5,
                descent: i32::from(size_px) / 5,
                line_gap: i32::from(size_px) / 8,
            };
        }
        let factor = self.factor(size_px);
        FontMetrics {
            ascent: round(ascent * factor),
            // A face's descent is negative going down; this trait wants a
            // positive distance from the baseline.
            descent: round(-descent * factor),
            line_gap: round(font.line_gap_unscaled() * factor),
        }
    }

    fn glyph_id(&self, ch: char) -> Option<GlyphId> {
        // Indexed by glyph, and index 0 is `.notdef` — the box. Keeping the
        // index rather than the character means the cache holds one entry for
        // every glyph the face actually has, not one per code point that maps to
        // the same one.
        Some(GlyphId(u32::from(self.font().glyph_id(ch).0)))
    }

    fn glyph_metrics(&mut self, glyph: GlyphId, size_px: u16) -> Option<GlyphMetrics> {
        let (advance, outlined) = self.outline(glyph, size_px)?;
        Some(convert(advance, outlined.as_ref()))
    }

    fn rasterise(&mut self, glyph: GlyphId, size_px: u16) -> Option<Rasterised<'_>> {
        let (advance, outlined) = self.outline(glyph, size_px)?;
        let metrics = convert(advance, outlined.as_ref());
        let width = metrics.size.width as usize;
        self.scratch.clear();
        self.scratch.resize(width * metrics.size.height as usize, 0);
        if let Some(outlined) = outlined {
            let scratch = &mut self.scratch;
            outlined.draw(|x, y, coverage| {
                if let Some(cell) = scratch.get_mut(y as usize * width + x as usize) {
                    *cell = (coverage.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            });
        }
        Some(Rasterised {
            metrics,
            coverage: &self.scratch,
            stride: width,
        })
    }

    fn contains(&self, ch: char) -> bool {
        self.font().glyph_id(ch).0 != 0
    }

    fn fallback_id(&self, _ch: char) -> Option<GlyphId> {
        // Glyph zero is `.notdef`, which every well-formed face draws as a box.
        Some(GlyphId(0))
    }
}

impl core::fmt::Debug for TrueTypeSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TrueTypeSource")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_a_font_fails_with_a_reason() {
        let error = TrueTypeSource::from_bytes("junk", b"not a font at all")
            .expect_err("that is not a font");
        assert!(!error.is_empty(), "the failure has to say something");
    }

    #[cfg(feature = "std")]
    /// A face this machine has, or `None` where it has none of them: no font
    /// ships with Denise.
    fn system_face() -> Option<TrueTypeSource> {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ]
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(|bytes| TrueTypeSource::from_vec("system", bytes).expect("a font"))
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_descender_hangs_below_the_baseline_and_a_cap_stands_on_it() {
        // Getting the sign of the top wrong puts every descender in the wrong
        // place, and only descenders, which is exactly the bug that survives a
        // casual look at a screenshot.
        let Some(mut face) = system_face() else {
            return;
        };
        let size = 20;
        let line = face.metrics(size);
        let g = face.glyph_id('g').expect("g");
        let g = face.glyph_metrics(g, size).expect("metrics");
        assert!(
            g.bearing_y > 0 && g.bearing_y < g.size.height as i32,
            "{g:?}"
        );
        let cap = face.glyph_id('H').expect("H");
        let cap = face.glyph_metrics(cap, size).expect("metrics");
        assert_eq!(cap.bearing_y, cap.size.height as i32, "{cap:?}");
        assert!(cap.bearing_y <= line.ascent && cap.bearing_y > size as i32 / 2);
        assert!(line.descent > 0 && line.ascent + line.descent >= size as i32);
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_space_advances_without_ink_and_a_letter_is_drawn_to_its_size() {
        let Some(mut face) = system_face() else {
            return;
        };
        let space = face.glyph_id(' ').expect("space");
        let space = face.rasterise(space, 16).expect("space");
        assert!(space.metrics.advance > 0);
        assert!(space.coverage.is_empty());
        let m = face.glyph_id('M').expect("M");
        let m = face.rasterise(m, 16).expect("M");
        let size = m.metrics.size;
        assert_eq!(m.coverage.len(), (size.width * size.height) as usize);
        assert_eq!(m.stride, size.width as usize);
        assert!(m.coverage.contains(&255), "a stem is solid somewhere");
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_glyph_the_face_does_not_have_is_the_box() {
        let Some(face) = system_face() else {
            return;
        };
        assert!(!face.contains('\u{10FFFD}'));
        assert!(face.contains('a'));
    }
}
