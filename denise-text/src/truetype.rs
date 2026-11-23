//! Real TrueType and OpenType fonts, behind the `truetype` feature.
//!
//! Anti-aliased, proportionally spaced, at any size — everything the built-in
//! bitmap font cannot do, for about 571 KB of static binary. What it still does
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
use skrifa::{DrawMemory, Font, PathSink, ScalerFont};
#[cfg(feature = "hinting")]
use skrifa::{HintOptions, RasterTarget};

use crate::fill::{Bounds, Outline};
use crate::source::{FontMetrics, GlyphId, GlyphMetrics, GlyphSource, Rasterised};

/// A TrueType or OpenType face, read glyph by glyph as it is drawn.
pub struct TrueTypeSource {
    name: String,
    /// The face, parsed once and holding its own bytes.
    ///
    /// This used to be the bytes alone, re-parsed for every question asked of
    /// the face, because a parsed face borrowed those bytes and a struct that
    /// owns bytes and borrows them at once is `unsafe` however it is dressed.
    /// Skrifa's own font model solves that with `yoke` — which is most of what
    /// the tier pays for its size — so the table directory is now read once
    /// instead of per glyph, per metric and per lookup.
    font: Font,
    /// The glyph of every ASCII character, looked up once. Layout asks for a
    /// glyph per character per measurement, nearly all of them these, and
    /// finding the character map in the file again for each would cost four
    /// times what the lookup does.
    ascii: [u32; 128],
    outline: Outline,
    scratch: Vec<u8>,
    /// The scaler for each size drawn at, built the first time that size is
    /// asked for. A panel draws at a handful of sizes — the text, the chrome,
    /// a size down for the tabs — and the oldest goes when a fifth arrives.
    ///
    /// A scaler is needed to draw at all, hinted or not, so this is not behind
    /// the `hinting` feature: without it the scalers are simply unhinted ones.
    scalers: Vec<(u16, ScalerFont)>,
}

/// How many sizes' scaler state is kept. A size costs a few kilobytes and a
/// fraction of a millisecond to build.
const HINTED_SIZES: usize = 4;

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
        Self::with(name, Font::new(data, 0))
    }

    /// Reads a font from bytes that live as long as the program — an
    /// `include_bytes!` — which are neither copied nor held twice.
    pub fn from_static(name: &str, data: &'static [u8]) -> Result<Self, String> {
        Self::with(name, Font::new(data, 0))
    }

    fn with(name: &str, font: Option<Font>) -> Result<Self, String> {
        // `Font::new` reads the table directory and refuses anything that is
        // not a font in a format it supports, which is the `head`/`maxp` check
        // this used to do by hand. It answers yes or no rather than saying what
        // was wrong, so the reason below is ours; a panel still learns that the
        // file was the problem and falls back to the built-in face.
        let font = font.ok_or_else(|| "not a font in any supported format".to_string())?;
        // A face whose em is zero divides every metric by nothing.
        if font.units_per_em() == 0 {
            return Err("the face declares an em of zero".to_string());
        }
        let charmap = font.charmap();
        let mut ascii = [0; 128];
        for (code, glyph) in (0u8..).zip(&mut ascii) {
            *glyph = charmap.map_unicode(code).map_or(0, skrifa::GlyphId::to_u32);
        }
        Ok(Self {
            name: name.to_owned(),
            font,
            ascii,
            outline: Outline::default(),
            scratch: Vec::new(),
            scalers: Vec::new(),
        })
    }

    /// The glyph `ch` maps to, where zero is `.notdef`: the face has no such
    /// character.
    fn lookup(&self, ch: char) -> u32 {
        if let Some(&glyph) = self.ascii.get(ch as usize) {
            return glyph;
        }
        self.font
            .charmap()
            .map_unicode(ch)
            .map_or(0, skrifa::GlyphId::to_u32)
    }

    /// `glyph`'s advance at `size_px`, and the pixels it covers if it has ink —
    /// a space has none — with its outline left in `self.outline`. `None` for an
    /// id the face does not have.
    fn outline(&mut self, glyph: GlyphId, size_px: u16) -> Option<(i32, Option<Bounds>)> {
        // A glyph the face does not have has no advance to report. The scaler
        // answers for any id, so the face's own count is what says no — which
        // `advance_width` used to do by returning `None`.
        if glyph.0 >= self.font.num_glyphs() {
            return None;
        }
        let id = skrifa::GlyphId::new(glyph.0);
        // Borrowed field by field: the outline is written while the scaler
        // cache is read, and they are separate fields.
        let Self {
            font,
            scalers,
            outline,
            ..
        } = self;
        let scaler = scaler(scalers, font, size_px);
        // Advances are fractional and pixels are not. Rounding here rather
        // than accumulating in floats keeps a line's width reproducible and
        // keeps the rasteriser's no-floating-point promise intact everywhere
        // except the one module that has to break it.
        //
        // Linear, which is to say unhinted: hinting moves the outline, not the
        // grid, so a monospaced face stays monospaced and a line measures the
        // same as it did before the hinter arrived.
        let advance = scaler.linear_glyph_metrics().h_advance(id);
        outline.clear();
        if let Some(glyph) = scaler.outline(id) {
            // The scaler carries the hinting, so there is nothing to choose
            // here any more: see `scaler` below for what it was built with.
            //
            // Working memory comes from skrifa's own short-lived scratch
            // rather than `self.scratch`, which is where the coverage mask
            // being built for this very glyph lives.
            if glyph.draw(DrawMemory::Scratch, &mut Pen(outline)).is_err() {
                // Half an outline is worse than none: the box is at least honest.
                outline.clear();
            }
            outline.close();
        }
        Some((round(advance), self.outline.bounds()))
    }
}

/// The scaler for `size_px`, built and kept the first time that size is drawn
/// at.
///
/// Pixels to the em, which is what this trait's sizes are, as a CSS
/// `font-size` is. A variable face is drawn at its default instance.
fn scaler<'a>(cache: &'a mut Vec<(u16, ScalerFont)>, font: &Font, size_px: u16) -> &'a ScalerFont {
    if let Some(at) = cache.iter().position(|(size, _)| *size == size_px) {
        return &cache[at].1;
    }
    let ppem = f32::from(size_px);
    // Hinted, where the `hinting` feature is on: the face's own instructions,
    // and nothing for a face that carries none. Light, so only the horizontal
    // edges are moved — a baseline, an x-height and a crossbar land on a pixel
    // boundary instead of across two, which is the whole difference between
    // crisp text and grey text at the sizes code is read at. Nothing moves
    // sideways, so the advances and the grid a monospaced face draws on are
    // the ones measured unhinted.
    //
    // `interpreter`, not `auto_fallback`: the face's own bytecode, and nothing
    // for a face that carries none. The interpreter is already linked into
    // this tier — 48 KB of it, which the linker could never prove was
    // unreachable — while skrifa's autohinter is another 320 KB, and over the
    // faces code is read in the two draw the same. A face with no hints is
    // drawn as it always was.
    //
    // The two `with_` calls are not decoration: `HintOptions` defaults to
    // symmetric rendering on and linear metrics off, which is the opposite of
    // what this tier asked the old API for on both counts.
    #[cfg(feature = "hinting")]
    let built = ScalerFont::hinted(
        font.clone(),
        ppem,
        HintOptions::interpreter()
            .with_target(RasterTarget::Light)
            .with_symmetric_rendering(false)
            .with_preserve_linear_metrics(true),
    );
    #[cfg(not(feature = "hinting"))]
    let built = ScalerFont::unhinted(font.clone(), Some(ppem));
    if cache.len() >= HINTED_SIZES {
        cache.remove(0);
    }
    cache.push((size_px, built));
    &cache[cache.len() - 1].1
}

/// Skrifa's curves, passed on to be flattened.
struct Pen<'a>(&'a mut Outline);

impl PathSink for Pen<'_> {
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

/// A pixel's coverage, widened.
///
/// A stem a pixel and a bit wide — which is every stem of a face read at the
/// size code is read at — lands across two pixels and covers neither of them,
/// and an exact rasteriser says so honestly: two pixels of half ink, which is
/// a grey smear where the reader wanted a stroke. Every rasteriser a desktop
/// uses widens the coverage to answer it, FreeType's stem darkening and the
/// font smoothing on a Mac being the same idea, and this is the curve. It
/// gives a pixel back what its neighbour would have lent it had the two been
/// one stroke, so it lifts hardest where a stem lands and hardly at all at
/// the edge of a glyph: two thirds covered comes out all but solid, half
/// covered at 0.78, a tenth covered at 0.12. Nothing and everything come out as
/// themselves, no pixel is ever dimmed, and a glyph ends up on slightly
/// fewer pixels than it started on rather than more — a stroke with a core,
/// not a fatter glyph.
///
/// It is applied to every face this tier reads, in the mask the atlas keeps,
/// so it is paid once per glyph per size and nothing per frame.
///
/// shortcut: the same curve whatever the text is drawn on. Leaving the faint
/// end alone is most of what a panel drawn dark on light needs — what grows
/// a halo there is lifting the pixels a glyph barely touches, and this curve
/// does not — but a stroke still ends up a shade heavier than its outline
/// asked for. Choosing the curve from the colour the glyph is drawn in means
/// doing it where the mask is composited rather than where it is made: a
/// table per painter rather than a pass per glyph, and a cost on every pixel
/// of every frame instead of none.
const fn dilate(ink: u8) -> u8 {
    // `c + 2.2c²(1 - c)`, in whole numbers: 11/5 over 255², the squaring and
    // the room above it fitting a `u32` with four bits to spare. The
    // rasteriser promises no floating point outside the outline itself, and
    // a cubic needs none.
    let c = ink as u32;
    let widened = c + 11 * c * c * (255 - c) / (5 * 255 * 255);
    // The curve crosses one a shade over two thirds covered and stays above
    // it, so everything from there up is solid.
    if widened > 255 { 255 } else { widened as u8 }
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
        // Scaled from the face's own units rather than through a scaler: the
        // horizontal line box is all this wants, and building a scaler for it
        // would resolve every outline in the face to answer three numbers.
        let line = self.font.metrics().scaled_f32(f32::from(size_px)).h_line();
        match line {
            Some(line) if line.ascender != 0.0 || line.descender != 0.0 => FontMetrics {
                ascent: round(line.ascender),
                // A face's descent is negative going down; this trait wants a
                // positive distance from the baseline.
                descent: round(-line.descender),
                line_gap: round(line.line_gap),
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
            Some(bounds) => {
                self.outline.fill(bounds, &mut self.scratch);
                for ink in &mut self.scratch {
                    *ink = dilate(*ink);
                }
            }
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

    /// Widening coverage keeps the ends where they are, never dims a pixel
    /// and never leaves the byte: a mask that came out solid stays solid, and
    /// one that was empty stays empty, or the glyph would grow a halo.
    #[test]
    fn coverage_is_widened_between_nothing_and_everything() {
        assert_eq!(dilate(0), 0);
        assert_eq!(dilate(255), 255);
        assert_eq!(dilate(128), 198, "half covered comes out at 0.78");
        assert_eq!(dilate(26), 31, "the faint edge of a glyph stays faint");
        assert_eq!(dilate(172), 255, "and a stem reads as a stroke");
        let mut last = 0;
        for ink in 0..=255u8 {
            let out = dilate(ink);
            assert!(out >= ink, "{ink} dimmed to {out}");
            assert!(out >= last, "{ink} went backwards");
            last = out;
        }
    }

    #[cfg(all(feature = "std", feature = "hinting"))]
    /// Hinting moves the outline, not the grid: a face's advances are the
    /// ones the metrics report, so a monospaced face stays monospaced and a
    /// line measures the same as it did before the hinter arrived.
    #[test]
    fn hinting_leaves_the_advances_alone() {
        let Some(mut face) = system_face() else {
            return;
        };
        for size in [11, 13, 20] {
            let advance = |face: &mut TrueTypeSource, ch: char| {
                let id = face.glyph_id(ch).expect("a glyph");
                face.glyph_metrics(id, size).expect("metrics").advance
            };
            for ch in ['H', 'n', 'i', '.'] {
                let hinted = advance(&mut face, ch);
                let id = skrifa::GlyphId::new(face.lookup(ch));
                // A scaler of its own, with no hinting at all, so this is the
                // advance the face declares rather than one the hinted scaler
                // happens to agree with.
                let unhinted = ScalerFont::unhinted(face.font.clone(), Some(f32::from(size)))
                    .linear_glyph_metrics()
                    .h_advance(id);
                assert_eq!(hinted, round(unhinted), "{ch} at {size}");
            }
        }
    }

    #[cfg(all(feature = "std", feature = "hinting"))]
    /// The hinting state is built once for a size and kept, and a fifth size
    /// pushes the oldest out rather than growing without end.
    #[test]
    fn a_handful_of_sizes_are_hinted_and_no_more() {
        let Some(mut face) = system_face() else {
            return;
        };
        let id = face.glyph_id('n').expect("n");
        for size in 10..=10 + HINTED_SIZES as u16 {
            let _ = face.rasterise(id, size);
        }
        assert_eq!(face.scalers.len(), HINTED_SIZES);
        assert!(
            !face.scalers.iter().any(|(size, _)| *size == 10),
            "the oldest went"
        );
    }

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
