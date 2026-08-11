//! Shaping, bidirectional text and font fallback, behind the `shaping` feature.
//!
//! This is the tier that makes Arabic join, Devanagari reorder, Hebrew run right
//! to left and `fi` become one glyph. It is also **3.1 MB of static binary**,
//! measured against a stripped `aarch64-unknown-linux-musl` build — roughly four
//! times the whole of the rest of Denise. That is not a criticism of cosmic-text,
//! which is doing an enormous amount of genuinely difficult work; it is the reason
//! the feature is off by default and the reason the middle tier exists.
//!
//! # Choose this deliberately
//!
//! - A temperature readout, a Norwegian name, a menu of European languages: the
//!   `truetype` tier at 145 KB draws all of it correctly. Measured on the same
//!   Norwegian pangram, the two tiers differ by **two pixels** of total width.
//! - Anything that has to typeset a script where a character is not a glyph: this
//!   tier, and the three megabytes, and there is no way around it.
//!
//! The failure mode of choosing wrong is worth knowing. Given Arabic, the
//! built-in bitmap font draws boxes — obviously missing, obviously a defect. The
//! `truetype` tier draws the *right glyphs, unjoined and in logical order*, which
//! is fluent nonsense: it looks like text and it is wrong, and nobody who cannot
//! read the script will notice. `examples/specimen` takes a sample string as its
//! third argument precisely so that can be checked before a device ships.
//!
//! # Font fallback and the cache key
//!
//! Shaping may pick glyphs from *several* faces for one run — that is what
//! fallback means. A [`GlyphId`] therefore packs the face into its high bits and
//! the face's own glyph index into its low ones, so two glyphs numbered 42 in two
//! different fallback faces are two cache entries rather than one wrong one.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics as CosmicMetrics, Shaping,
    SwashCache, SwashContent,
};
use denise::Size;

use crate::source::{FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised, ShapedGlyph};

/// How many faces a [`GlyphId`] can distinguish. The rest of the `u32` is the
/// face's own glyph index, which is a `u16` in every format in use.
const FACE_SHIFT: u32 = 16;

/// A shaping text source over an embedded set of faces.
pub struct ShapedSource {
    name: String,
    fonts: FontSystem,
    cache: SwashCache,
    /// Faces, in the order shaping first asked for them. The index is what a
    /// [`GlyphId`] carries.
    faces: Vec<cosmic_text::fontdb::ID>,
    scratch: Vec<u8>,
}

impl ShapedSource {
    /// Builds a source over `fonts`, which are TrueType or OpenType bytes.
    ///
    /// As with the `truetype` tier there is no font discovery: nothing is read
    /// from the filesystem and nothing is inherited from the host. A device that
    /// boots from flash with a read-only root very often has no fonts installed,
    /// and a UI whose text depends on that is a UI that fails in the field.
    pub fn from_fonts(
        name: &str,
        fonts: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<Self, String> {
        // The database is built by hand rather than through `new_with_fonts`,
        // which despite its name *also* loads every font on the host — 812 of
        // them on the machine this was written on. On a device that would mean
        // rendering depending on whatever a given unit happens to have installed,
        // and on a bare rootfs it would mean rendering nothing at all.
        let mut db = cosmic_text::fontdb::Database::new();
        for data in fonts {
            db.load_font_data(data);
        }
        if db.is_empty() {
            return Err("no usable faces in the fonts provided".to_owned());
        }
        // Point every generic family at what was actually embedded, so
        // `Family::SansSerif` resolves rather than falling through to nothing.
        let embedded = db
            .faces()
            .next()
            .and_then(|face| face.families.first().map(|(name, _)| name.clone()));
        if let Some(family) = embedded {
            db.set_sans_serif_family(&family);
            db.set_serif_family(&family);
            db.set_monospace_family(&family);
            db.set_cursive_family(&family);
            db.set_fantasy_family(&family);
        }
        // The locale is passed rather than queried: `sys-locale` asks the host,
        // and a panel's text should not change because someone set LANG.
        let fonts = FontSystem::new_with_locale_and_db("en-US".to_owned(), db);
        Ok(Self {
            name: name.to_owned(),
            fonts,
            cache: SwashCache::new(),
            faces: Vec::new(),
            scratch: Vec::new(),
        })
    }

    /// The face slot for a `fontdb` id, assigning one on first sight.
    fn slot(&mut self, id: cosmic_text::fontdb::ID) -> Option<u32> {
        if let Some(index) = self.faces.iter().position(|&f| f == id) {
            return Some(index as u32);
        }
        // Beyond this the packing would collide, and a wrong glyph is worse than
        // a missing one.
        if self.faces.len() as u32 >= (1 << FACE_SHIFT) {
            return None;
        }
        self.faces.push(id);
        Some(self.faces.len() as u32 - 1)
    }

    fn pack(slot: u32, glyph: u16) -> GlyphId {
        GlyphId((slot << FACE_SHIFT) | u32::from(glyph))
    }

    fn unpack(&self, id: GlyphId) -> Option<(cosmic_text::fontdb::ID, u16)> {
        let slot = (id.0 >> FACE_SHIFT) as usize;
        let glyph = (id.0 & ((1 << FACE_SHIFT) - 1)) as u16;
        self.faces.get(slot).map(|&face| (face, glyph))
    }

    /// Shapes `text` into a buffer, calling `f` for each glyph cosmic-text placed.
    fn with_run(
        &mut self,
        text: &str,
        size_px: u16,
        mut f: impl FnMut(&mut Self, cosmic_text::LayoutGlyph),
    ) {
        let size = f32::from(size_px.max(1));
        let metrics = CosmicMetrics::new(size, size * 1.25);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let mut borrowed = buffer.borrow_with(&mut self.fonts);
        borrowed.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        borrowed.shape_until_scroll(false);

        // Collected first because `f` needs `&mut self` and the runs borrow the
        // buffer, which borrows the font system.
        let glyphs: Vec<cosmic_text::LayoutGlyph> = buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().cloned())
            .collect();
        for glyph in glyphs {
            f(self, glyph);
        }
    }
}

impl GlyphSource for ShapedSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        // cosmic-text's line metrics live on the buffer rather than the face, and
        // building a buffer needs `&mut`. These proportions are the usual ones for
        // a Latin sans face and are only used for line stacking.
        let size = i32::from(size_px);
        FontMetrics {
            ascent: size * 4 / 5,
            descent: size / 5,
            line_gap: size / 4,
        }
    }

    fn glyph_id(&self, _ch: char) -> Option<GlyphId> {
        // A shaping source has no character-to-glyph mapping worth exposing: the
        // answer depends on the neighbours. Callers go through `shape`.
        None
    }

    fn glyph_metrics(&mut self, glyph: GlyphId, size_px: u16) -> Option<GlyphMetrics> {
        // Metrics come from the rasterised image, because swash reports placement
        // and rasterisation together. The atlas caches the result, so this is paid
        // once per glyph per size rather than once per measurement.
        self.rasterise(glyph, size_px).map(|r| r.metrics)
    }

    fn rasterise(&mut self, glyph: GlyphId, size_px: u16) -> Option<Rasterised<'_>> {
        let (face, index) = self.unpack(glyph)?;
        let key = cosmic_text::CacheKey {
            font_id: face,
            glyph_id: index,
            font_size_bits: f32::from(size_px.max(1)).to_bits(),
            x_bin: cosmic_text::SubpixelBin::Zero,
            y_bin: cosmic_text::SubpixelBin::Zero,
            font_weight: cosmic_text::Weight::NORMAL,
            flags: CacheKeyFlags::empty(),
        };
        let image = self.cache.get_image_uncached(&mut self.fonts, key)?;
        let width = image.placement.width;
        let height = image.placement.height;

        self.scratch.clear();
        match image.content {
            SwashContent::Mask => self.scratch.extend_from_slice(&image.data),
            // A colour glyph — an emoji — reduced to its alpha. Denise composites
            // one colour through a coverage mask, so this is the honest shape of
            // it rather than a silently wrong one.
            SwashContent::Color => self
                .scratch
                .extend(image.data.chunks_exact(4).map(|px| px[3])),
            SwashContent::SubpixelMask => self
                .scratch
                .extend(image.data.chunks_exact(4).map(|px| px[3])),
        }
        if self.scratch.len() < (width * height) as usize {
            self.scratch.resize((width * height) as usize, 0);
        }

        Some(Rasterised {
            metrics: GlyphMetrics {
                // The advance comes from shaping, not from here; a glyph drawn
                // from a cached mask is positioned by the run it belongs to.
                advance: 0,
                bearing_x: image.placement.left,
                bearing_y: image.placement.top,
                size: Size::new(width, height),
            },
            coverage: &self.scratch,
            stride: width as usize,
        })
    }

    fn shape(&mut self, text: &str, size_px: u16, out: &mut Vec<ShapedGlyph>) -> i32 {
        let mut width = 0;
        let mut placed: Vec<(cosmic_text::fontdb::ID, u16, i32, i32, i32)> = Vec::new();
        self.with_run(text, size_px, |_, glyph| {
            placed.push((
                glyph.font_id,
                glyph.glyph_id,
                glyph.x.round() as i32,
                glyph.y.round() as i32,
                glyph.w.round() as i32,
            ));
        });
        for (face, index, x, y, advance) in placed {
            let Some(slot) = self.slot(face) else {
                continue;
            };
            out.push(ShapedGlyph {
                id: Self::pack(slot, index),
                x,
                y,
            });
            width = width.max(x + advance);
        }
        width
    }

    fn can_shape(&self) -> bool {
        true
    }

    fn contains(&self, ch: char) -> bool {
        // Fallback means "something in the set can probably draw it", and a
        // definitive answer would need shaping the character in context.
        !ch.is_control()
    }
}

impl core::fmt::Debug for ShapedSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShapedSource")
            .field("name", &self.name)
            .field("faces_seen", &self.faces.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glyph_id_round_trips_through_its_packing() {
        let mut source = ShapedSource {
            name: "test".to_owned(),
            fonts: FontSystem::new_with_fonts(core::iter::empty()),
            cache: SwashCache::new(),
            faces: Vec::new(),
            scratch: Vec::new(),
        };
        // Two distinct faces, both with a glyph numbered 42: the packing is what
        // stops them being one cache entry and one of them being drawn wrong.
        let a = cosmic_text::fontdb::ID::dummy();
        let slot = source.slot(a).expect("first slot");
        let id = ShapedSource::pack(slot, 42);
        assert_eq!(source.unpack(id), Some((a, 42)));
        assert_ne!(id, ShapedSource::pack(slot + 1, 42));
    }

    #[test]
    fn no_fonts_is_an_error_rather_than_a_blank_screen() {
        let error = ShapedSource::from_fonts("empty", core::iter::empty())
            .expect_err("nothing to shape with");
        assert!(!error.is_empty());
    }
}
