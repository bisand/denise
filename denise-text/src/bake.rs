//! Turns a face into the tables [`BakedSource`](crate::BakedSource) draws from.
//!
//! Behind the `bake` feature, which is for a build script or a tool and never
//! for a panel: it needs `std` to write files and, for a real face, the
//! `truetype` tier to read one. The panel links neither. The usual shape is a
//! `build.rs` that names a font file, the sizes the UI uses and the characters
//! it can be asked to draw:
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     denise_text::bake::to_out_dir(
//!         "fonts/Inter-Regular.ttf",
//!         "INTER",
//!         &[14, 18, 28],
//!         denise_text::bake::LATIN.iter().copied(),
//!     )
//!     .unwrap();
//! }
//! ```
//!
//! and, in the program, `include!(concat!(env!("OUT_DIR"), "/INTER.rs"))`,
//! which declares `static INTER: BakedFont`. A tool that wants the file
//! committed instead calls [`bake`] and writes [`Baked::to_source`] wherever
//! it likes; the same text either way.
//!
//! Anything that is a [`GlyphSource`] can be baked, the built-in bitmap font
//! included, so a project without a font file still has something to try the
//! tier on. The result draws exactly what the source would have: the bake is
//! the source's own `rasterise`, kept.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::source::{GlyphSource, Rasterised};

/// Printable ASCII and Latin-1: `' '..='~'` and `'\u{a0}'..='ÿ'`, which covers
/// English, the Nordic and Germanic languages, French, Spanish and Portuguese.
/// A starting point, not a limit: pass any characters.
pub const LATIN: &[std::ops::RangeInclusive<char>] = &[' '..='~', '\u{a0}'..='ÿ'];

/// Every character in `ranges`, for passing to [`bake`].
pub fn chars(ranges: &[std::ops::RangeInclusive<char>]) -> impl Iterator<Item = char> + '_ {
    ranges.iter().flat_map(|range| range.clone())
}

/// One glyph as baked: what [`BakedGlyph`](crate::BakedGlyph) holds, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The character, or `'\0'` for the box.
    pub ch: char,
    /// How far the pen moves on, in pixels.
    pub advance: i32,
    /// From the pen to the left of the ink.
    pub bearing_x: i32,
    /// From the baseline up to the top of the ink.
    pub bearing_y: i32,
    /// The ink's box.
    pub width: u32,
    /// The ink's box.
    pub height: u32,
    /// One byte per pixel of the box, row after row.
    pub coverage: Vec<u8>,
}

/// One size as baked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtSize {
    /// Pixels to the em, as the text engine asks for it.
    pub size_px: u16,
    /// The face's line metrics at this size.
    pub metrics: crate::FontMetrics,
    /// Ascending by character, the box first as `'\0'`.
    pub glyphs: Vec<Glyph>,
}

/// A face as baked: the tables, before they are written out as Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Baked {
    /// What the source called itself.
    pub name: String,
    /// Ascending by size.
    pub sizes: Vec<AtSize>,
}

/// Rasterises every character in `chars` at every size in `sizes_px`, plus
/// the source's fallback glyph — the box — at each, so that a character the
/// face has not got draws as something.
///
/// A character the source does not contain is left out rather than baked as
/// the box, so `contains` on the result answers as it did on the source. Sizes
/// are deduplicated and a size of zero is ignored. Returns an error only if
/// nothing could be baked at all, which means the source has no sizes or no
/// characters worth the name.
pub fn bake(
    source: &mut dyn GlyphSource,
    sizes_px: &[u16],
    chars: impl IntoIterator<Item = char>,
) -> Result<Baked, String> {
    let mut sizes: Vec<u16> = sizes_px.iter().copied().filter(|&size| size > 0).collect();
    sizes.sort_unstable();
    sizes.dedup();
    if sizes.is_empty() {
        return Err(String::from("no sizes to bake"));
    }
    let mut chars: Vec<char> = chars.into_iter().filter(|&ch| ch != '\0').collect();
    chars.sort_unstable();
    chars.dedup();
    if chars.is_empty() {
        return Err(String::from("no characters to bake"));
    }

    let mut baked = Baked {
        name: source.name().to_owned(),
        sizes: Vec::with_capacity(sizes.len()),
    };
    for size_px in sizes {
        // What the source would really draw at this size — a bitmap font snaps
        // to whole scales — is what gets baked, under the size that was asked
        // for, so the panel's request and the bake's answer line up.
        let mut glyphs = Vec::with_capacity(chars.len() + 1);
        if let Some(glyph) = source
            .fallback_id('\0')
            .and_then(|fallback| source.rasterise(fallback, size_px))
            .map(|drawn| keep('\0', drawn))
        {
            glyphs.push(glyph);
        }
        for &ch in &chars {
            // `contains` rather than `glyph_id` alone: a source may answer
            // every character with the box, and the box is baked once.
            if !source.contains(ch) {
                continue;
            }
            let Some(id) = source.glyph_id(ch) else {
                continue;
            };
            if let Some(glyph) = source.rasterise(id, size_px).map(|drawn| keep(ch, drawn)) {
                glyphs.push(glyph);
            }
        }
        if glyphs.iter().all(|glyph| glyph.ch == '\0') {
            continue;
        }
        baked.sizes.push(AtSize {
            size_px,
            metrics: source.metrics(size_px),
            glyphs,
        });
    }
    if baked.sizes.is_empty() {
        return Err(format!("{}: nothing to bake", baked.name));
    }
    Ok(baked)
}

/// A drawn glyph, copied out of the source's scratch buffer with its rows
/// packed: the source's stride may be wider than the glyph.
fn keep(ch: char, drawn: Rasterised<'_>) -> Glyph {
    let (width, height) = (drawn.metrics.size.width, drawn.metrics.size.height);
    let mut coverage = Vec::with_capacity((width * height) as usize);
    for row in 0..height as usize {
        let start = row * drawn.stride;
        coverage.extend_from_slice(&drawn.coverage[start..start + width as usize]);
    }
    Glyph {
        ch,
        advance: drawn.metrics.advance,
        bearing_x: drawn.metrics.bearing_x,
        bearing_y: drawn.metrics.bearing_y,
        width,
        height,
        coverage,
    }
}

impl Baked {
    /// Bytes of coverage across every size: what the tables cost, roughly, in
    /// flash — the per-glyph entries add about twenty bytes each.
    pub fn coverage_len(&self) -> usize {
        self.sizes
            .iter()
            .flat_map(|size| &size.glyphs)
            .map(|glyph| glyph.coverage.len())
            .sum()
    }

    /// The tables as Rust: `static <ident>: BakedFont = ...;`, with the paths
    /// spelt out so it compiles wherever it is included. Glyphs with identical
    /// coverage share bytes, which is every blank glyph and, at small sizes,
    /// a fair few others.
    pub fn to_source(&self, ident: &str) -> String {
        let mut coverage: Vec<u8> = Vec::new();
        let mut seen = HashMap::new();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "// Generated by `denise_text::bake` from \"{}\". Do not edit.",
            self.name.escape_debug()
        );
        let _ = writeln!(out, "#[allow(clippy::all, unused)]");
        let _ = writeln!(
            out,
            "static {ident}: ::denise_text::BakedFont = ::denise_text::BakedFont {{"
        );
        let _ = writeln!(out, "    name: \"{}\",", self.name.escape_debug());
        let _ = writeln!(out, "    sizes: &[");
        for size in &self.sizes {
            let m = size.metrics;
            let _ = writeln!(out, "        ::denise_text::BakedSize {{");
            let _ = writeln!(out, "            size_px: {},", size.size_px);
            let _ = writeln!(
                out,
                "            metrics: ::denise_text::FontMetrics {{ ascent: {}, descent: {}, line_gap: {} }},",
                m.ascent, m.descent, m.line_gap
            );
            let _ = writeln!(out, "            glyphs: &[");
            for glyph in &size.glyphs {
                let offset = place(&mut coverage, &mut seen, &glyph.coverage);
                let _ = writeln!(
                    out,
                    "                ::denise_text::BakedGlyph {{ ch: {:?}, advance: {}, bearing_x: {}, bearing_y: {}, width: {}, height: {}, offset: {} }},",
                    glyph.ch,
                    glyph.advance.clamp(i16::MIN.into(), i16::MAX.into()),
                    glyph.bearing_x.clamp(i16::MIN.into(), i16::MAX.into()),
                    glyph.bearing_y.clamp(i16::MIN.into(), i16::MAX.into()),
                    glyph.width.min(u16::MAX.into()),
                    glyph.height.min(u16::MAX.into()),
                    offset
                );
            }
            let _ = writeln!(out, "            ],");
            let _ = writeln!(out, "        }},");
        }
        let _ = writeln!(out, "    ],");
        // A byte string is the compact spelling: two characters per byte that
        // is printable, four otherwise, against at least two and a comma for a
        // number — and a hundred kilobytes of numbers is slow to parse.
        out.push_str("    coverage: b\"");
        for &byte in &coverage {
            match byte {
                b' '..=b'~' if byte != b'"' && byte != b'\\' => out.push(byte as char),
                _ => {
                    let _ = write!(out, "\\x{byte:02x}");
                }
            }
        }
        out.push_str("\",\n};\n");
        out
    }
}

/// Where `bytes` are in `pool`, adding them if they are not: a glyph that draws
/// exactly like an earlier one points at the earlier one's bytes.
fn place(pool: &mut Vec<u8>, seen: &mut HashMap<Vec<u8>, usize>, bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    if let Some(&at) = seen.get(bytes) {
        return at;
    }
    let at = pool.len();
    pool.extend_from_slice(bytes);
    seen.insert(bytes.to_vec(), at);
    at
}

/// For a `build.rs`: reads the font at `path` with the `truetype` tier, bakes
/// it, and writes `$OUT_DIR/<ident>.rs` declaring `static <ident>: BakedFont`.
/// Tells Cargo to rerun when the font file changes. The error says which file
/// and why, for a `panic!` in the build script to show.
pub fn to_out_dir(
    path: impl AsRef<std::path::Path>,
    ident: &str,
    sizes_px: &[u16],
    chars: impl IntoIterator<Item = char>,
) -> Result<std::path::PathBuf, String> {
    let path = path.as_ref();
    println!("cargo:rerun-if-changed={}", path.display());
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(ident);
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut face = crate::TrueTypeSource::from_vec(name, bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let baked = bake(&mut face, sizes_px, chars).map_err(|e| format!("{}: {e}", path.display()))?;
    let out = std::path::PathBuf::from(
        std::env::var("OUT_DIR")
            .map_err(|_| String::from("OUT_DIR is not set; this is for a build script"))?,
    )
    .join(format!("{ident}.rs"));
    std::fs::write(&out, baked.to_source(ident)).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BitmapSource;

    #[test]
    fn the_built_in_font_bakes_to_what_it_draws() {
        let mut bitmap = BitmapSource::new();
        let baked = bake(&mut bitmap, &[16, 8, 8, 0], chars(LATIN)).expect("bakes");
        assert_eq!(baked.name, bitmap.name());
        let sizes: Vec<u16> = baked.sizes.iter().map(|size| size.size_px).collect();
        assert_eq!(sizes, [8, 16], "sorted, deduplicated, zero dropped");
        let at8 = &baked.sizes[0];
        assert_eq!(at8.glyphs[0].ch, '\0', "the box comes first");
        assert!(at8.glyphs.windows(2).all(|pair| pair[0].ch < pair[1].ch));
        let a = at8.glyphs.iter().find(|glyph| glyph.ch == 'a').expect("a");
        let id = bitmap.glyph_id('a').expect("a");
        let drawn = bitmap.rasterise(id, 8).expect("ink");
        assert_eq!(a.coverage, drawn.coverage);
        assert_eq!(
            (a.width, a.height),
            (drawn.metrics.size.width, drawn.metrics.size.height)
        );
        assert_eq!(a.advance, drawn.metrics.advance);
        assert!(baked.coverage_len() > 0);
    }

    #[test]
    fn a_character_the_face_lacks_is_not_baked_as_the_box() {
        let mut bitmap = BitmapSource::new();
        let baked = bake(&mut bitmap, &[8], ['a', 'Ж', '\0']).expect("bakes");
        let chars: Vec<char> = baked.sizes[0].glyphs.iter().map(|glyph| glyph.ch).collect();
        assert_eq!(chars, ['\0', 'a']);
    }

    #[test]
    fn nothing_to_bake_is_an_error_not_an_empty_table() {
        let mut bitmap = BitmapSource::new();
        assert!(bake(&mut bitmap, &[], ['a']).is_err());
        assert!(bake(&mut bitmap, &[8], []).is_err());
        assert!(
            bake(&mut bitmap, &[8], ['Ж']).is_err(),
            "only the box would be left"
        );
    }

    #[test]
    fn the_source_is_a_static_with_the_paths_spelt_out() {
        let mut bitmap = BitmapSource::new();
        let baked = bake(&mut bitmap, &[8], ['a', 'b']).expect("bakes");
        let source = baked.to_source("FONT");
        assert!(source.starts_with("// Generated by `denise_text::bake`"));
        assert!(
            source.contains("static FONT: ::denise_text::BakedFont = ::denise_text::BakedFont {")
        );
        assert!(source.contains("size_px: 8,"));
        assert!(source.contains("ch: 'a',"));
        assert!(source.contains("ch: '\\0',"));
        assert!(source.contains("coverage: b\""));
        assert!(source.ends_with("\",\n};\n"));
    }

    /// A `Baked` as the `static` the generated source would declare, leaked:
    /// a test cannot compile what it generated, but it can build the same
    /// tables in memory and draw from them.
    fn leaked(baked: &Baked) -> &'static crate::BakedFont {
        let mut coverage = Vec::new();
        let mut seen = HashMap::new();
        let sizes: Vec<crate::BakedSize> = baked
            .sizes
            .iter()
            .map(|size| {
                let glyphs: Vec<crate::BakedGlyph> = size
                    .glyphs
                    .iter()
                    .map(|glyph| crate::BakedGlyph {
                        ch: glyph.ch,
                        advance: glyph.advance as i16,
                        bearing_x: glyph.bearing_x as i16,
                        bearing_y: glyph.bearing_y as i16,
                        width: glyph.width as u16,
                        height: glyph.height as u16,
                        offset: place(&mut coverage, &mut seen, &glyph.coverage) as u32,
                    })
                    .collect();
                crate::BakedSize {
                    size_px: size.size_px,
                    metrics: size.metrics,
                    glyphs: Box::leak(glyphs.into_boxed_slice()),
                }
            })
            .collect();
        Box::leak(Box::new(crate::BakedFont {
            name: Box::leak(baked.name.clone().into_boxed_str()),
            sizes: Box::leak(sizes.into_boxed_slice()),
            coverage: Box::leak(coverage.into_boxed_slice()),
        }))
    }

    #[test]
    fn a_baked_face_draws_byte_for_byte_what_the_truetype_tier_draws() {
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
        let mut face = crate::TrueTypeSource::from_vec("system", bytes).expect("a font");
        let sizes = [12, 16, 28];
        let baked = bake(&mut face, &sizes, chars(LATIN)).expect("bakes");
        let mut baked = crate::BakedSource::new(leaked(&baked));

        let mut compared = 0;
        for size in sizes {
            assert_eq!(baked.metrics(size), face.metrics(size));
            assert_eq!(baked.snap_size(size), size);
            for ch in chars(LATIN).chain(['\u{10FFFD}']) {
                assert_eq!(baked.contains(ch), face.contains(ch), "{ch:?}");
                let (Some(id), Some(from)) = (
                    baked.glyph_id(ch).or_else(|| baked.fallback_id(ch)),
                    face.glyph_id(ch).or_else(|| face.fallback_id(ch)),
                ) else {
                    panic!("{ch:?} has no glyph and no fallback");
                };
                let expected = face.rasterise(from, size).expect("drawn");
                let (metrics, coverage) = (expected.metrics, expected.coverage.to_vec());
                let drawn = baked.rasterise(id, size).expect("baked");
                assert_eq!(drawn.metrics, metrics, "{ch:?} at {size}");
                assert_eq!(drawn.coverage, coverage, "{ch:?} at {size}");
                compared += 1;
            }
        }
        assert!(compared > 500, "{compared} glyphs compared");

        // What was asked for that was not baked: the nearest size, the box.
        assert_eq!(baked.snap_size(20), 16);
        assert_eq!(baked.snap_size(14), 12);
        assert_eq!(baked.glyph_id('Ж'), None);
        let box_id = baked.fallback_id('Ж').expect("the box");
        assert!(
            !baked
                .rasterise(box_id, 16)
                .expect("the box")
                .metrics
                .is_blank()
        );
    }

    #[test]
    fn identical_glyphs_share_their_bytes() {
        let (mut pool, mut seen) = (Vec::new(), HashMap::new());
        assert_eq!(place(&mut pool, &mut seen, &[1, 2, 3]), 0);
        assert_eq!(place(&mut pool, &mut seen, &[4]), 3);
        assert_eq!(place(&mut pool, &mut seen, &[1, 2, 3]), 0, "seen before");
        assert_eq!(place(&mut pool, &mut seen, &[]), 0, "nothing is anywhere");
        assert_eq!(pool, [1, 2, 3, 4]);
    }
}
