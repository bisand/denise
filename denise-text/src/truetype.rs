//! Real TrueType and OpenType fonts, behind the `truetype` feature.
//!
//! Anti-aliased, proportionally spaced, at any size — everything the built-in
//! bitmap font cannot do, for about 270 KB of static binary. What it still does
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
//! **Embed a font you are allowed to ship.** The face a desktop draws itself in
//! — Arial, Helvetica, San Francisco, Segoe — is licensed for use on that
//! machine, not for redistribution inside a product, and `include_bytes!` of
//! one is the obvious thing to try and the one thing not to ship. A face under
//! the SIL Open Font License — Inter, DejaVu, Noto, Liberation, Fira — may be
//! embedded freely; a commercial one needs an embedding licence, which is a
//! different thing from a desktop one. The examples in this workspace load
//! whatever the machine they run on has, at run time, precisely so that
//! nothing ships. The same applies to a face baked with
//! [`bake`](crate::bake): the tables are a rendering of the font, and a
//! rendering is the font's licence's business too.
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
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use denise::Size;
use skrifa::instance::LocationRef;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

use crate::fill::{Bounds, Outline};
use crate::source::{FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised};

/// What a face is read from: bytes the source owns, or bytes that live as long
/// as the program does and need no copy.
enum Face {
    Owned(Vec<u8>),
    Static(&'static [u8]),
}

/// A TrueType or OpenType face, read glyph by glyph as it is drawn.
pub struct TrueTypeSource {
    name: String,
    face: Face,
    /// The glyph of every ASCII character, looked up once. Layout asks for a
    /// glyph per character per measurement, nearly all of them these, and
    /// finding the character map in the file again for each would cost four
    /// times what the lookup does.
    ascii: [u32; 128],
    outline: Outline,
    scratch: Vec<u8>,
}

impl Face {
    fn bytes(&self) -> &[u8] {
        match self {
            Face::Owned(data) => data,
            Face::Static(data) => data,
        }
    }
}

/// The first face in `data`, which is the only one in anything but a collection.
///
/// This reads the table directory — a few dozen bytes — and nothing else, so it
/// is done again for every question asked of the face rather than kept: a parsed
/// face borrows its bytes, and a struct that owns bytes and borrows them at once
/// is `unsafe` however it is dressed.
fn parse(data: &[u8]) -> Result<FontRef<'_>, String> {
    FontRef::from_index(data, 0).map_err(|e| e.to_string())
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
        Self::with(name, Face::Owned(data))
    }

    /// Reads a font from bytes that live as long as the program — an
    /// `include_bytes!` — which are neither copied nor held twice.
    pub fn from_static(name: &str, data: &'static [u8]) -> Result<Self, String> {
        Self::with(name, Face::Static(data))
    }

    fn with(name: &str, face: Face) -> Result<Self, String> {
        // A table directory is all `parse` reads, and plenty of files that are
        // not fonts have a plausible one. A face with no `head` or no `maxp`
        // can answer nothing this trait asks.
        let font = parse(face.bytes())?;
        font.head().map_err(|e| e.to_string())?;
        font.maxp().map_err(|e| e.to_string())?;
        let charmap = font.charmap();
        let mut ascii = [0; 128];
        for (code, glyph) in (0u8..).zip(&mut ascii) {
            *glyph = charmap.map(code).map_or(0, skrifa::GlyphId::to_u32);
        }
        Ok(Self {
            name: name.to_owned(),
            face,
            ascii,
            outline: Outline::default(),
            scratch: Vec::new(),
        })
    }

    /// The face, which parsed when the source was made and whose bytes have not
    /// changed since. `None` would be a glyph not drawn rather than a panic.
    fn font(&self) -> Option<FontRef<'_>> {
        parse(self.face.bytes()).ok()
    }

    /// The glyph `ch` maps to, where zero is `.notdef`: the face has no such
    /// character.
    fn lookup(&self, ch: char) -> u32 {
        if let Some(&glyph) = self.ascii.get(ch as usize) {
            return glyph;
        }
        self.font()
            .and_then(|font| font.charmap().map(ch))
            .map_or(0, skrifa::GlyphId::to_u32)
    }

    /// `glyph`'s advance at `size_px`, and the pixels it covers if it has ink —
    /// a space has none — with its outline left in `self.outline`. `None` for an
    /// id the face does not have.
    fn outline(&mut self, glyph: GlyphId, size_px: u16) -> Option<(i32, Option<Bounds>)> {
        // `self.face` rather than `self.font()`: the outline is written while
        // the face is read, and they are separate fields.
        let font = parse(self.face.bytes()).ok()?;
        let id = skrifa::GlyphId::new(glyph.0);
        // Pixels to the em, which is what this trait's sizes are, as a CSS
        // `font-size` is. A variable face is drawn at its default instance.
        let size = skrifa::instance::Size::new(f32::from(size_px));
        // Advances are fractional and pixels are not. Rounding here rather
        // than accumulating in floats keeps a line's width reproducible and
        // keeps the rasteriser's no-floating-point promise intact everywhere
        // except the one module that has to break it.
        let advance = font
            .glyph_metrics(size, LocationRef::default())
            .advance_width(id)?;
        self.outline.clear();
        if let Some(glyph) = font.outline_glyphs().get(id) {
            let settings = DrawSettings::unhinted(size, LocationRef::default());
            if glyph.draw(settings, &mut Pen(&mut self.outline)).is_err() {
                // Half an outline is worse than none: the box is at least honest.
                self.outline.clear();
            }
            self.outline.close();
        }
        Some((round(advance), self.outline.bounds()))
    }
}

/// Skrifa's curves, passed on to be flattened.
struct Pen<'a>(&'a mut Outline);

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, cy0, x, y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(cx0, cy0, cx1, cy1, x, y);
    }

    fn close(&mut self) {
        self.0.close();
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

/// The metrics of a glyph from the pixels its outline covers, which grow
/// downwards from the baseline. This trait wants the top as a distance above it.
fn convert(advance: i32, bounds: Option<Bounds>) -> GlyphMetrics {
    let Some(bounds) = bounds else {
        return GlyphMetrics {
            advance,
            bearing_x: 0,
            bearing_y: 0,
            size: Size::new(0, 0),
        };
    };
    GlyphMetrics {
        advance,
        bearing_x: bounds.x,
        bearing_y: -bounds.y,
        size: Size::new(bounds.width, bounds.height),
    }
}

impl GlyphSource for TrueTypeSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn metrics(&self, size_px: u16) -> FontMetrics {
        let size = skrifa::instance::Size::new(f32::from(size_px));
        let metrics = self
            .font()
            .map(|font| font.metrics(size, LocationRef::default()));
        match metrics {
            Some(metrics) if metrics.ascent != 0.0 || metrics.descent != 0.0 => FontMetrics {
                ascent: round(metrics.ascent),
                // A face's descent is negative going down; this trait wants a
                // positive distance from the baseline.
                descent: round(-metrics.descent),
                line_gap: round(metrics.leading),
            },
            // A face with no vertical metrics is broken, but guessing from the
            // requested size beats returning zeroes and stacking every line on
            // top of the last.
            _ => FontMetrics {
                ascent: i32::from(size_px) * 4 / 5,
                descent: i32::from(size_px) / 5,
                line_gap: i32::from(size_px) / 8,
            },
        }
    }

    fn glyph_id(&self, ch: char) -> Option<GlyphId> {
        // Indexed by glyph, and index 0 is `.notdef` — the box. Keeping the
        // index rather than the character means the cache holds one entry for
        // every glyph the face actually has, not one per code point that maps to
        // the same one.
        Some(GlyphId(self.lookup(ch)))
    }

    fn glyph_metrics(&mut self, glyph: GlyphId, size_px: u16) -> Option<GlyphMetrics> {
        let (advance, bounds) = self.outline(glyph, size_px)?;
        Some(convert(advance, bounds))
    }

    fn rasterise(&mut self, glyph: GlyphId, size_px: u16) -> Option<Rasterised<'_>> {
        let (advance, bounds) = self.outline(glyph, size_px)?;
        match bounds {
            Some(bounds) => self.outline.fill(bounds, &mut self.scratch),
            None => self.scratch.clear(),
        }
        let metrics = convert(advance, bounds);
        Some(Rasterised {
            metrics,
            coverage: &self.scratch,
            stride: metrics.size.width as usize,
        })
    }

    fn contains(&self, ch: char) -> bool {
        self.lookup(ch) != 0
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
    /// A variable face this machine has, or `None` where it has none.
    fn variable_face() -> Option<TrueTypeSource> {
        [
            // What a Mac draws its own chrome in, and the face this is here
            // for: read without its variations, it draws nothing at all.
            "/System/Library/Fonts/SFNS.ttf",
            // Windows 10 and later ship this one.
            "C:\\Windows\\Fonts\\bahnschrift.ttf",
        ]
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(|bytes| TrueTypeSource::from_vec("variable", bytes).expect("a font"))
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_variable_face_has_ink_in_it() {
        // A variable face's outlines are its default ones plus deltas, and a
        // reader that skips the deltas does not fail: under ab_glyph without
        // `variable-fonts` such a face parsed, reported a glyph for every
        // character it had, and rasterised all of them empty. Nothing returned
        // an error and nothing logged: the window simply had no words in it,
        // which is a long way from the cause. Skrifa needs nothing asked for,
        // and this stays so that stays true. Machines without one of these
        // faces have nothing to check.
        let Some(mut face) = variable_face() else {
            return;
        };
        let a = face.glyph_id('a').expect("a");
        let a = face.rasterise(a, 16).expect("an outline");
        assert!(!a.metrics.is_blank(), "no mask at all: {:?}", a.metrics);
        assert!(a.coverage.iter().any(|&ink| ink > 0), "no ink in the mask");
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_damaged_face_is_refused_or_drawn_wrong_and_never_panics() {
        // A font is bytes somebody else wrote. Cut short or corrupted, a face
        // may fail to load, or load and draw boxes and nonsense; what it may
        // not do is take the panel down. The damage is the same on every run.
        let Some(bytes) = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ]
        .iter()
        .find_map(|path| std::fs::read(path).ok()) else {
            return;
        };
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut loaded = 0;
        for round in 0..400 {
            let mut damaged = bytes.clone();
            if round % 4 == 0 {
                damaged.truncate(next() as usize % bytes.len());
            }
            for _ in 0..1 + next() % 64 {
                if !damaged.is_empty() {
                    // Early bytes are the table directory and the tables that
                    // say where everything else is: damage there goes furthest.
                    let reach = if round % 2 == 0 {
                        damaged.len().min(4096)
                    } else {
                        damaged.len()
                    };
                    let at = next() as usize % reach;
                    damaged[at] = next() as u8;
                }
            }
            let Ok(mut face) = TrueTypeSource::from_vec("damaged", damaged) else {
                continue;
            };
            loaded += 1;
            let _ = face.metrics(16);
            for ch in ['a', 'g', '@', 'Ω', '\u{10FFFD}'] {
                let id = face.glyph_id(ch).expect("always some glyph");
                let _ = face.glyph_metrics(id, 16);
                if let Some(drawn) = face.rasterise(id, 48) {
                    let size = drawn.metrics.size;
                    assert_eq!(drawn.coverage.len(), (size.width * size.height) as usize);
                }
            }
            let _ = face.rasterise(GlyphId(u32::MAX), 16);
        }
        assert!(
            loaded > 0,
            "every damaged face was refused, so nothing was tested"
        );
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
