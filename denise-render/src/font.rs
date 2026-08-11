//! The built-in bitmap font.
//!
//! Five pixels wide, seven tall, in an eight-row cell, one row per byte with bit 7
//! on the left. Monospace, with a six-pixel advance and integer scaling — at 3× on
//! a 1080p panel that is a comfortable 15×21 px of text, which is what an HMI read
//! at arm's length actually wants.
//!
//! # Why this is here in M3 rather than M4
//!
//! M4 owns text properly: `denise-text`, `cosmic-text` behind a feature flag, a
//! glyph atlas, real shaping and proportional metrics. But a Label, a Button and a
//! TextInput without glyphs are three rectangles, so the milestone that ships them
//! needs *some* font. This is the "built-in 8×8 bitmap font" M4 already promised,
//! brought forward and no more than that. It is deliberately not extensible: there
//! is no font loading here, and there will not be.
//!
//! # Coverage
//!
//! Printable ASCII, plus `ÆØÅ æøå ÄÖÜ äöü Éé ß °`. Anything else draws as an empty
//! box, which is a visible defect rather than a silent gap. Combining marks, RTL
//! and complex shaping are not supported and cannot be — this is a fixed grid.
//!
//! # The art is the source
//!
//! Glyphs are written as ASCII art and packed into bits by a `const fn` at compile
//! time. A hand-maintained table of hex bytes is unreviewable; a picture of a `Ø`
//! is not.

#[cfg(test)]
extern crate alloc;

use denise::{Point, Rect, Size};

use crate::blend::Paint;
use crate::canvas::Canvas;

/// Glyph box width in pixels.
pub const CELL_WIDTH: i32 = 5;
/// Glyph cell height in pixels, descender row included.
pub const CELL_HEIGHT: i32 = 8;
/// Horizontal distance between glyph origins, at scale 1.
pub const ADVANCE: i32 = 6;
/// Vertical distance between baselines, at scale 1.
pub const LINE_HEIGHT: i32 = 9;

/// One glyph: eight rows of five bits, bit 7 leftmost.
pub type Glyph = [u8; CELL_HEIGHT as usize];

/// Packs `CELL_WIDTH * CELL_HEIGHT` ASCII bytes into a glyph.
///
/// `#` sets a pixel; anything else clears it.
const fn pack(art: &str) -> Glyph {
    let bytes = art.as_bytes();
    assert!(
        bytes.len() == (CELL_WIDTH * CELL_HEIGHT) as usize,
        "glyph art must be exactly CELL_WIDTH by CELL_HEIGHT characters"
    );
    let mut rows = [0u8; CELL_HEIGHT as usize];
    let mut y = 0;
    while y < CELL_HEIGHT as usize {
        let mut x = 0;
        while x < CELL_WIDTH as usize {
            if bytes[y * CELL_WIDTH as usize + x] == b'#' {
                rows[y] |= 0x80 >> x;
            }
            x += 1;
        }
        y += 1;
    }
    rows
}

/// A fixed-pitch bitmap font.
#[derive(Clone, Copy, Debug)]
pub struct BitmapFont {
    ascii: &'static [Glyph; 95],
    /// Sorted by code point.
    extras: &'static [(char, Glyph)],
    fallback: Glyph,
}

/// The one font that ships with Denise.
pub static BUILT_IN: BitmapFont = BitmapFont {
    ascii: &ASCII,
    extras: &EXTRAS,
    fallback: pack(concat!(
        "#####", "#...#", "#...#", "#...#", "#...#", "#...#", "#####", ".....",
    )),
};

impl BitmapFont {
    /// The glyph for `ch`, or the missing-character box.
    pub fn glyph(&self, ch: char) -> &Glyph {
        let code = ch as u32;
        if (0x20..0x7F).contains(&code) {
            return &self.ascii[(code - 0x20) as usize];
        }
        match self.extras.binary_search_by_key(&ch, |&(c, _)| c) {
            Ok(index) => &self.extras[index].1,
            Err(_) => &self.fallback,
        }
    }

    /// Returns `true` if `ch` has a glyph of its own.
    pub fn contains(&self, ch: char) -> bool {
        let code = ch as u32;
        (0x20..0x7F).contains(&code) || self.extras.binary_search_by_key(&ch, |&(c, _)| c).is_ok()
    }

    /// Width of one line of text, excluding the trailing inter-glyph gap.
    ///
    /// The gap is excluded so that centring text in a button actually centres the
    /// ink rather than leaving it a pixel left of true.
    pub fn line_width(&self, line: &str, scale: i32) -> i32 {
        let scale = scale.max(1);
        let count = line.chars().count() as i32;
        if count == 0 {
            0
        } else {
            (count * ADVANCE - (ADVANCE - CELL_WIDTH)) * scale
        }
    }

    /// Extent of `text`, honouring `\n`.
    pub fn measure(&self, text: &str, scale: i32) -> Size {
        let scale = scale.max(1);
        let mut widest = 0;
        let mut lines = 0;
        for line in text.split('\n') {
            widest = widest.max(self.line_width(line, scale));
            lines += 1;
        }
        Size::new(
            widest.max(0) as u32,
            ((lines - 1) * LINE_HEIGHT * scale + CELL_HEIGHT * scale).max(0) as u32,
        )
    }

    /// Horizontal offset of the glyph at character index `index`.
    ///
    /// Fixed pitch, so this is multiplication rather than a layout pass — which is
    /// exactly why a text field's caret arithmetic is trivial here and will stop
    /// being trivial when M4 brings proportional fonts.
    #[inline]
    pub const fn caret_offset(&self, index: usize, scale: i32) -> i32 {
        index as i32 * ADVANCE * if scale > 1 { scale } else { 1 }
    }
}

impl Canvas<'_> {
    /// Draws one glyph with its cell's top-left corner at `at`.
    pub fn draw_glyph(&mut self, glyph: &Glyph, at: Point, scale: i32, color: impl Into<Paint>) {
        let scale = scale.max(1);
        let cell = Rect::new(at.x, at.y, CELL_WIDTH * scale, CELL_HEIGHT * scale);
        if self.visible(cell).is_none() {
            return;
        }
        let paint = color.into();
        for (row, bits) in glyph.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            let y = at.y + row as i32 * scale;
            // Runs of set bits blit as one span. The per-pixel path measured
            // fifteen times slower than the span path on a Pi 3, and glyphs are
            // where that difference will be paid most often.
            let mut x = 0;
            while x < CELL_WIDTH {
                if bits & (0x80 >> x) == 0 {
                    x += 1;
                    continue;
                }
                let mut end = x + 1;
                while end < CELL_WIDTH && bits & (0x80 >> end) != 0 {
                    end += 1;
                }
                self.fill_rect(
                    Rect::new(at.x + x * scale, y, (end - x) * scale, scale),
                    paint,
                );
                x = end;
            }
        }
    }

    /// Draws `text` with the first cell's top-left corner at `at`, honouring `\n`.
    ///
    /// Returns the extent actually laid out, whether or not it was clipped.
    pub fn draw_text(
        &mut self,
        font: &BitmapFont,
        at: Point,
        scale: i32,
        text: &str,
        color: impl Into<Paint>,
    ) -> Size {
        let scale = scale.max(1);
        let paint = color.into();
        let mut pen = at;
        for ch in text.chars() {
            if ch == '\n' {
                pen = Point::new(at.x, pen.y + LINE_HEIGHT * scale);
                continue;
            }
            self.draw_glyph(font.glyph(ch), pen, scale, paint);
            pen.x += ADVANCE * scale;
        }
        font.measure(text, scale)
    }
}

/// Printable ASCII, `0x20..=0x7E`, in code-point order.
const ASCII_ART: [&str; 95] = [
    //
    concat!(
        ".....", ".....", ".....", ".....", ".....", ".....", ".....", ".....",
    ),
    // !
    concat!(
        "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..", ".....",
    ),
    // double quote
    concat!(
        ".#.#.", ".#.#.", ".....", ".....", ".....", ".....", ".....", ".....",
    ),
    // #
    concat!(
        ".#.#.", ".#.#.", "#####", ".#.#.", "#####", ".#.#.", ".#.#.", ".....",
    ),
    // $
    concat!(
        "..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#..", ".....",
    ),
    // %
    concat!(
        "##...", "##..#", "...#.", "..#..", ".#...", "#..##", "...##", ".....",
    ),
    // &
    concat!(
        ".##..", "#..#.", "#.#..", ".#...", "#.#.#", "#..#.", ".##.#", ".....",
    ),
    // '
    concat!(
        "..#..", "..#..", ".....", ".....", ".....", ".....", ".....", ".....",
    ),
    // (
    concat!(
        "...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#.", ".....",
    ),
    // )
    concat!(
        ".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#...", ".....",
    ),
    // *
    concat!(
        ".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", ".....", ".....",
    ),
    // +
    concat!(
        ".....", "..#..", "..#..", "#####", "..#..", "..#..", ".....", ".....",
    ),
    // ,
    concat!(
        ".....", ".....", ".....", ".....", ".....", "..##.", "..#..", ".#...",
    ),
    // -
    concat!(
        ".....", ".....", ".....", ".###.", ".....", ".....", ".....", ".....",
    ),
    // .
    concat!(
        ".....", ".....", ".....", ".....", ".....", ".##..", ".##..", ".....",
    ),
    // /
    concat!(
        "....#", "....#", "...#.", "..#..", ".#...", "#....", "#....", ".....",
    ),
    // 0
    concat!(
        ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.", ".....",
    ),
    // 1
    concat!(
        "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.", ".....",
    ),
    // 2
    concat!(
        ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####", ".....",
    ),
    // 3
    concat!(
        "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.", ".....",
    ),
    // 4
    concat!(
        "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.", ".....",
    ),
    // 5
    concat!(
        "#####", "#....", "####.", "....#", "....#", "#...#", ".###.", ".....",
    ),
    // 6
    concat!(
        "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.", ".....",
    ),
    // 7
    concat!(
        "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...", ".....",
    ),
    // 8
    concat!(
        ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.", ".....",
    ),
    // 9
    concat!(
        ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..", ".....",
    ),
    // :
    concat!(
        ".....", ".##..", ".##..", ".....", ".##..", ".##..", ".....", ".....",
    ),
    // ;
    concat!(
        ".....", ".##..", ".##..", ".....", ".##..", "..#..", ".#...", ".....",
    ),
    // <
    concat!(
        "...#.", "..#..", ".#...", "#....", ".#...", "..#..", "...#.", ".....",
    ),
    // =
    concat!(
        ".....", ".....", "#####", ".....", "#####", ".....", ".....", ".....",
    ),
    // >
    concat!(
        ".#...", "..#..", "...#.", "....#", "...#.", "..#..", ".#...", ".....",
    ),
    // ?
    concat!(
        ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..", ".....",
    ),
    // @
    concat!(
        ".###.", "#...#", "#.###", "#.#.#", "#.###", "#....", ".###.", ".....",
    ),
    // A
    concat!(
        ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#", ".....",
    ),
    // B
    concat!(
        "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.", ".....",
    ),
    // C
    concat!(
        ".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.", ".....",
    ),
    // D
    concat!(
        "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.", ".....",
    ),
    // E
    concat!(
        "#####", "#....", "#....", "####.", "#....", "#....", "#####", ".....",
    ),
    // F
    concat!(
        "#####", "#....", "#....", "####.", "#....", "#....", "#....", ".....",
    ),
    // G
    concat!(
        ".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###.", ".....",
    ),
    // H
    concat!(
        "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#", ".....",
    ),
    // I
    concat!(
        ".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.", ".....",
    ),
    // J
    concat!(
        "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..", ".....",
    ),
    // K
    concat!(
        "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#", ".....",
    ),
    // L
    concat!(
        "#....", "#....", "#....", "#....", "#....", "#....", "#####", ".....",
    ),
    // M
    concat!(
        "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#", ".....",
    ),
    // N
    concat!(
        "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#", ".....",
    ),
    // O
    concat!(
        ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.", ".....",
    ),
    // P
    concat!(
        "####.", "#...#", "#...#", "####.", "#....", "#....", "#....", ".....",
    ),
    // Q
    concat!(
        ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#", ".....",
    ),
    // R
    concat!(
        "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#", ".....",
    ),
    // S
    concat!(
        ".####", "#....", "#....", ".###.", "....#", "....#", "####.", ".....",
    ),
    // T
    concat!(
        "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..", ".....",
    ),
    // U
    concat!(
        "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.", ".....",
    ),
    // V
    concat!(
        "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..", ".....",
    ),
    // W
    concat!(
        "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#", ".....",
    ),
    // X
    concat!(
        "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#", ".....",
    ),
    // Y
    concat!(
        "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..", ".....",
    ),
    // Z
    concat!(
        "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####", ".....",
    ),
    // [
    concat!(
        ".###.", ".#...", ".#...", ".#...", ".#...", ".#...", ".###.", ".....",
    ),
    // backslash
    concat!(
        "#....", "#....", ".#...", "..#..", "...#.", "....#", "....#", ".....",
    ),
    // ]
    concat!(
        ".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###.", ".....",
    ),
    // ^
    concat!(
        "..#..", ".#.#.", "#...#", ".....", ".....", ".....", ".....", ".....",
    ),
    // _
    concat!(
        ".....", ".....", ".....", ".....", ".....", ".....", ".....", "#####",
    ),
    // `
    concat!(
        "..#..", "...#.", ".....", ".....", ".....", ".....", ".....", ".....",
    ),
    // a
    concat!(
        ".....", ".....", ".###.", "....#", ".####", "#...#", ".####", ".....",
    ),
    // b
    concat!(
        "#....", "#....", "####.", "#...#", "#...#", "#...#", "####.", ".....",
    ),
    // c
    concat!(
        ".....", ".....", ".###.", "#....", "#....", "#....", ".###.", ".....",
    ),
    // d
    concat!(
        "....#", "....#", ".####", "#...#", "#...#", "#...#", ".####", ".....",
    ),
    // e
    concat!(
        ".....", ".....", ".###.", "#...#", "#####", "#....", ".###.", ".....",
    ),
    // f
    concat!(
        "..##.", ".#...", ".#...", "####.", ".#...", ".#...", ".#...", ".....",
    ),
    // g
    concat!(
        ".....", ".....", ".####", "#...#", "#...#", ".####", "....#", ".###.",
    ),
    // h
    concat!(
        "#....", "#....", "####.", "#...#", "#...#", "#...#", "#...#", ".....",
    ),
    // i
    concat!(
        "..#..", ".....", ".##..", "..#..", "..#..", "..#..", ".###.", ".....",
    ),
    // j
    concat!(
        "...#.", ".....", "..##.", "...#.", "...#.", "...#.", "#..#.", ".##..",
    ),
    // k
    concat!(
        "#....", "#....", "#..#.", "#.#..", "##...", "#.#..", "#..#.", ".....",
    ),
    // l
    concat!(
        ".##..", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.", ".....",
    ),
    // m
    concat!(
        ".....", ".....", "##.#.", "#.#.#", "#.#.#", "#...#", "#...#", ".....",
    ),
    // n
    concat!(
        ".....", ".....", "####.", "#...#", "#...#", "#...#", "#...#", ".....",
    ),
    // o
    concat!(
        ".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###.", ".....",
    ),
    // p
    concat!(
        ".....", ".....", "####.", "#...#", "#...#", "####.", "#....", "#....",
    ),
    // q
    concat!(
        ".....", ".....", ".####", "#...#", "#...#", ".####", "....#", "....#",
    ),
    // r
    concat!(
        ".....", ".....", "#.##.", "##..#", "#....", "#....", "#....", ".....",
    ),
    // s
    concat!(
        ".....", ".....", ".####", "#....", ".###.", "....#", "####.", ".....",
    ),
    // t
    concat!(
        ".#...", ".#...", "####.", ".#...", ".#...", ".#..#", "..##.", ".....",
    ),
    // u
    concat!(
        ".....", ".....", "#...#", "#...#", "#...#", "#..##", ".##.#", ".....",
    ),
    // v
    concat!(
        ".....", ".....", "#...#", "#...#", "#...#", ".#.#.", "..#..", ".....",
    ),
    // w
    concat!(
        ".....", ".....", "#...#", "#...#", "#.#.#", "#.#.#", ".#.#.", ".....",
    ),
    // x
    concat!(
        ".....", ".....", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", ".....",
    ),
    // y
    concat!(
        ".....", ".....", "#...#", "#...#", "#...#", ".####", "....#", ".###.",
    ),
    // z
    concat!(
        ".....", ".....", "#####", "...#.", "..#..", ".#...", "#####", ".....",
    ),
    // {
    concat!(
        "...##", "..#..", "..#..", ".#...", "..#..", "..#..", "...##", ".....",
    ),
    // |
    concat!(
        "..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..", ".....",
    ),
    // }
    concat!(
        "##...", "..#..", "..#..", "...#.", "..#..", "..#..", "##...", ".....",
    ),
    // ~
    concat!(
        ".....", ".....", ".#..#", "#.#.#", "#..#.", ".....", ".....", ".....",
    ),
];

/// Glyphs outside ASCII, **sorted by code point** so lookup can bisect.
const EXTRA_ART: [(char, &str); 23] = [
    // «
    (
        '\u{00ab}',
        concat!(
            ".....", ".....", "..#.#", ".#.#.", "#.#..", ".#.#.", "..#.#", ".....",
        ),
    ),
    // °
    (
        '\u{00b0}',
        concat!(
            ".##..", "#..#.", ".##..", ".....", ".....", ".....", ".....", ".....",
        ),
    ),
    // ±
    (
        '\u{00b1}',
        concat!(
            "..#..", "..#..", "#####", "..#..", "..#..", ".....", "#####", ".....",
        ),
    ),
    // µ
    (
        '\u{00b5}',
        concat!(
            ".....", ".....", "#...#", "#...#", "#...#", "#..##", "#.##.", "#....",
        ),
    ),
    // »
    (
        '\u{00bb}',
        concat!(
            ".....", ".....", "#.#..", ".#.#.", "..#.#", ".#.#.", "#.#..", ".....",
        ),
    ),
    // Ä
    (
        '\u{00c4}',
        concat!(
            ".#.#.", ".###.", "#...#", "#...#", "#####", "#...#", "#...#", ".....",
        ),
    ),
    // Å
    (
        '\u{00c5}',
        concat!(
            "..#..", ".###.", "#...#", "#...#", "#####", "#...#", "#...#", ".....",
        ),
    ),
    // Æ
    (
        '\u{00c6}',
        concat!(
            ".####", "#.#..", "#.#..", "#####", "#.#..", "#.#..", "#.###", ".....",
        ),
    ),
    // É
    (
        '\u{00c9}',
        concat!(
            "...#.", "#####", "#....", "####.", "#....", "#....", "#####", ".....",
        ),
    ),
    // Ö
    (
        '\u{00d6}',
        concat!(
            ".#.#.", ".###.", "#...#", "#...#", "#...#", "#...#", ".###.", ".....",
        ),
    ),
    // ×
    (
        '\u{00d7}',
        concat!(
            ".....", ".....", ".....", ".#.#.", "..#..", ".#.#.", ".....", ".....",
        ),
    ),
    // Ø
    (
        '\u{00d8}',
        concat!(
            ".####", "#..##", "#..##", "#.#.#", "##..#", "##..#", "####.", ".....",
        ),
    ),
    // Ü
    (
        '\u{00dc}',
        concat!(
            ".#.#.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.", ".....",
        ),
    ),
    // ß
    (
        '\u{00df}',
        concat!(
            ".....", ".##..", "#..#.", "#.#..", "#..#.", "#..#.", "#.##.", ".....",
        ),
    ),
    // ä
    (
        '\u{00e4}',
        concat!(
            ".#.#.", ".....", ".###.", "....#", ".####", "#...#", ".####", ".....",
        ),
    ),
    // å
    (
        '\u{00e5}',
        concat!(
            "..#..", ".....", ".###.", "....#", ".####", "#...#", ".####", ".....",
        ),
    ),
    // æ
    (
        '\u{00e6}',
        concat!(
            ".....", ".....", "##.##", "..#.#", ".####", "#.#..", ".####", ".....",
        ),
    ),
    // é
    (
        '\u{00e9}',
        concat!(
            "...#.", ".....", ".###.", "#...#", "#####", "#....", ".###.", ".....",
        ),
    ),
    // ö
    (
        '\u{00f6}',
        concat!(
            ".#.#.", ".....", ".###.", "#...#", "#...#", "#...#", ".###.", ".....",
        ),
    ),
    // ø
    (
        '\u{00f8}',
        concat!(
            ".....", ".....", ".####", "#..##", "#.#.#", "##..#", "####.", ".....",
        ),
    ),
    // ü
    (
        '\u{00fc}',
        concat!(
            ".#.#.", ".....", "#...#", "#...#", "#...#", "#..##", ".##.#", ".....",
        ),
    ),
    // en dash
    (
        '\u{2013}',
        concat!(
            ".....", ".....", ".....", "#####", ".....", ".....", ".....", ".....",
        ),
    ),
    // em dash
    (
        '\u{2014}',
        concat!(
            ".....", ".....", ".....", "#####", ".....", ".....", ".....", ".....",
        ),
    ),
];

const ASCII: [Glyph; 95] = {
    let mut packed = [[0u8; CELL_HEIGHT as usize]; 95];
    let mut i = 0;
    while i < 95 {
        packed[i] = pack(ASCII_ART[i]);
        i += 1;
    }
    packed
};

const EXTRAS: [(char, Glyph); EXTRA_ART.len()] = {
    let mut packed = [('\0', [0u8; CELL_HEIGHT as usize]); EXTRA_ART.len()];
    let mut i = 0;
    while i < EXTRA_ART.len() {
        packed[i] = (EXTRA_ART[i].0, pack(EXTRA_ART[i].1));
        // Lookup bisects, so an unsorted table would silently miss glyphs.
        assert!(
            i == 0 || (EXTRA_ART[i - 1].0 as u32) < (EXTRA_ART[i].0 as u32),
            "EXTRA_ART must be sorted by code point"
        );
        i += 1;
    }
    packed
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;
    use denise::Color;

    /// Pairs that genuinely cannot be told apart in five columns. Anything not
    /// listed here being identical is a mistake in the art, not a limit of the
    /// grid.
    const TIED: [(char, char); 1] = [('\u{2013}', '\u{2014}')];

    #[test]
    fn every_glyph_is_distinct() {
        // Catches the copy-paste that leaves `Q` looking exactly like `O`, which is
        // the failure mode of hand-authored bitmap art and is otherwise only found
        // by someone squinting at a panel.
        let mut seen: alloc::vec::Vec<(char, Glyph)> = alloc::vec::Vec::new();
        let chars = (0x20u32..0x7F)
            .filter_map(char::from_u32)
            .chain(EXTRAS.iter().map(|&(c, _)| c));
        for ch in chars {
            let glyph = *BUILT_IN.glyph(ch);
            if ch == ' ' {
                assert_eq!(glyph, [0; 8], "space must be blank");
                continue;
            }
            assert_ne!(glyph, [0; 8], "{ch:?} has no ink");
            if let Some((other, _)) = seen.iter().find(|(_, g)| *g == glyph) {
                assert!(
                    TIED.contains(&(*other, ch)),
                    "{ch:?} and {other:?} are the same picture"
                );
            }
            seen.push((ch, glyph));
        }
    }

    #[test]
    fn nordic_letters_are_present_and_ascii_is_complete() {
        for ch in "ÆØÅæøåÄÖÜäöüÉéß°".chars() {
            assert!(BUILT_IN.contains(ch), "{ch:?} is missing");
        }
        for code in 0x20u32..0x7F {
            let ch = char::from_u32(code).expect("ascii");
            assert!(BUILT_IN.contains(ch), "{ch:?} is missing");
        }
    }

    #[test]
    fn an_unmapped_character_draws_a_visible_box() {
        assert!(!BUILT_IN.contains('\u{4e2d}'));
        assert_eq!(*BUILT_IN.glyph('\u{4e2d}'), BUILT_IN.fallback);
        assert_ne!(BUILT_IN.fallback, [0; 8], "a missing glyph must be visible");
    }

    #[test]
    fn extras_are_sorted_so_lookup_can_bisect() {
        assert!(EXTRAS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn measurement_matches_what_is_drawn() {
        let size = BUILT_IN.measure("Hi", 2);
        // Two glyphs: 5 + gap 1 + 5 = 11 columns at 2×.
        assert_eq!(size, Size::new(22, 16));
        assert_eq!(BUILT_IN.measure("", 1), Size::new(0, 8));
        assert_eq!(BUILT_IN.measure("a\nbb", 1).height, LINE_HEIGHT as u32 + 8);
        assert_eq!(BUILT_IN.measure("a\nbb", 1).width, 11);
    }

    #[test]
    fn scale_is_clamped_to_at_least_one() {
        assert_eq!(BUILT_IN.line_width("abc", 0), BUILT_IN.line_width("abc", 1));
        assert_eq!(BUILT_IN.caret_offset(3, -4), BUILT_IN.caret_offset(3, 1));
    }

    #[test]
    fn text_stays_inside_the_rectangle_it_measures() {
        let mut t = TestCanvas::new(80, 40);
        let text = "Wg|";
        let scale = 2;
        let extent = {
            let mut c = t.canvas();
            c.draw_text(&BUILT_IN, Point::new(4, 4), scale, text, Color::WHITE)
        };
        let bounds = Rect::new(4, 4, extent.width as i32, extent.height as i32);
        for y in 0..40i32 {
            for x in 0..80i32 {
                if !bounds.contains(Point::new(x, y)) {
                    assert_eq!(
                        t.pixels()[(y * 80 + x) as usize],
                        0,
                        "ink outside the measured extent at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn drawing_is_clipped_like_everything_else() {
        let mut t = TestCanvas::new(64, 16);
        {
            let mut c = t.canvas();
            let mut clipped = c.with_clip(Rect::new(0, 0, 12, 16));
            clipped.draw_text(&BUILT_IN, Point::new(0, 0), 1, "MMMMMMMM", Color::WHITE);
        }
        for y in 0..16usize {
            for x in 12..64usize {
                assert_eq!(t.pixels()[y * 64 + x], 0, "drew past the clip at {x},{y}");
            }
        }
        assert!(
            t.pixels().iter().any(|&p| p != 0),
            "nothing was drawn at all"
        );
    }

    #[test]
    fn a_newline_starts_a_second_line() {
        let mut t = TestCanvas::new(40, 32);
        {
            let mut c = t.canvas();
            c.draw_text(&BUILT_IN, Point::new(0, 0), 1, "A\nA", Color::WHITE);
        }
        let row_of = |y: usize| t.pixels()[y * 40..y * 40 + 5].to_vec();
        assert_eq!(row_of(1), row_of(1 + LINE_HEIGHT as usize));
    }
}
