//! A bounded glyph cache.
//!
//! One buffer of coverage bytes, shelf-packed, with a fixed size chosen at
//! construction. That last part is the point: a panel with 512 MB of RAM and a
//! twenty-year service life wants "the glyph cache is exactly 512 KB", not
//! "however many glyphs the user has typed since Tuesday, times whatever size they
//! happened to be".
//!
//! # Eviction is a reset
//!
//! When the atlas fills it is cleared wholesale and repacked on demand, rather
//! than freeing individual rectangles. Shelf packing cannot reclaim a hole in the
//! middle of a shelf, so per-glyph eviction would need a different packer and a
//! free list to buy something a UI does not need: the working set of a panel is
//! its labels, it is small, and it stops changing about a second after boot. A
//! reset costs one frame of re-rasterisation and is counted, so a cache that is
//! genuinely too small shows up as a rising `resets` rather than as a mystery.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use denise::{Rect, Size};
use denise_render::Mask;

use crate::source::{FontId, GlyphId, GlyphMetrics, GlyphSource};

/// Identifies one rasterised glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphKey {
    /// Which font.
    pub font: FontId,
    /// At which pixel size.
    pub size_px: u16,
    /// Which glyph. Not a character: after shaping there is no longer one of
    /// those per glyph.
    pub glyph: GlyphId,
}

impl GlyphKey {
    /// The key for a character in a source that maps them directly.
    pub const fn from_char(font: FontId, size_px: u16, ch: char) -> Self {
        Self {
            font,
            size_px,
            glyph: GlyphId::from_char(ch),
        }
    }
}

/// A glyph's place in the atlas, and how to position it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    /// Where the coverage lives in the atlas. Empty for a glyph with no ink.
    pub rect: Rect,
    /// Where it sits relative to the pen and baseline.
    pub metrics: GlyphMetrics,
}

/// What the cache has been doing, for benches and for diagnosing a panel that
/// stutters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that had to rasterise.
    pub misses: u64,
    /// Times the atlas filled and was cleared.
    ///
    /// Should settle at zero once a panel's labels are on screen. A number that
    /// keeps climbing means the atlas is too small for the working set, and every
    /// frame is paying to rasterise glyphs it had a moment ago.
    pub resets: u64,
    /// Glyphs that no source could rasterise, or that were larger than the atlas.
    pub failures: u64,
}

impl AtlasStats {
    /// Hit rate in percent, or `None` before anything has been looked up.
    pub fn hit_rate(&self) -> Option<u32> {
        let total = self.hits + self.misses;
        (total > 0).then(|| (self.hits * 100 / total) as u32)
    }
}

/// A row of the atlas that glyphs of a similar height are packed into.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    next_x: u32,
}

/// A fixed-size cache of rasterised glyphs.
pub struct GlyphAtlas {
    coverage: Vec<u8>,
    size: Size,
    shelves: Vec<Shelf>,
    used_height: u32,
    entries: BTreeMap<GlyphKey, Placed>,
    stats: AtlasStats,
}

impl GlyphAtlas {
    /// Creates an atlas of exactly `size` bytes of coverage.
    ///
    /// 256×256 is 64 KB and holds several hundred glyphs at panel sizes, which is
    /// more than a kiosk shows at once.
    pub fn new(size: Size) -> Self {
        let size = Size::new(size.width.max(1), size.height.max(1));
        Self {
            coverage: vec![0; size.area() as usize],
            size,
            shelves: Vec::new(),
            used_height: 0,
            entries: BTreeMap::new(),
            stats: AtlasStats::default(),
        }
    }

    /// An atlas of a size that suits a panel: 256×256, 64 KB.
    pub fn with_default_size() -> Self {
        Self::new(Size::new(256, 256))
    }

    /// Extent of the coverage buffer.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Bytes of coverage held, cached or not.
    #[inline]
    pub fn capacity_bytes(&self) -> usize {
        self.coverage.len()
    }

    /// Glyphs currently cached.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if nothing is cached.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cache statistics.
    #[inline]
    pub const fn stats(&self) -> AtlasStats {
        self.stats
    }

    /// Empties the cache. The next lookup for every glyph will miss.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.shelves.clear();
        self.used_height = 0;
    }

    /// Looks a glyph up, rasterising it through `source` on a miss.
    ///
    /// Returns `None` only if the source has no such glyph, or the glyph is larger
    /// than the whole atlas.
    pub fn get_or_insert(&mut self, key: GlyphKey, source: &mut dyn GlyphSource) -> Option<Placed> {
        if let Some(placed) = self.entries.get(&key) {
            self.stats.hits += 1;
            return Some(*placed);
        }
        self.stats.misses += 1;

        let Some(glyph) = source.rasterise(key.glyph, key.size_px) else {
            self.stats.failures += 1;
            return None;
        };
        let metrics = glyph.metrics;

        // A space has advance but no ink, and needs no atlas space at all.
        if metrics.is_blank() {
            let placed = Placed {
                rect: Rect::ZERO,
                metrics,
            };
            self.entries.insert(key, placed);
            return Some(placed);
        }

        let width = metrics.size.width;
        let height = metrics.size.height;
        let rect = match self.pack(width, height) {
            Some(rect) => rect,
            None => {
                // Full. Start over rather than carry a fragmented shelf list, and
                // count it so a too-small atlas is visible instead of merely slow.
                self.clear();
                self.stats.resets += 1;
                self.pack(width, height).or_else(|| {
                    self.stats.failures += 1;
                    None
                })?
            }
        };

        // `glyph` borrows `source`; the copy target borrows `self`. Disjoint, so
        // this needs no intermediate buffer.
        for row in 0..height as usize {
            let from = row * glyph.stride;
            let to = (rect.y as usize + row) * self.size.width as usize + rect.x as usize;
            self.coverage[to..to + width as usize]
                .copy_from_slice(&glyph.coverage[from..from + width as usize]);
        }

        let placed = Placed { rect, metrics };
        self.entries.insert(key, placed);
        Some(placed)
    }

    /// The coverage of a placed glyph, as a mask over the atlas buffer.
    pub fn mask(&self, placed: &Placed) -> Option<Mask<'_>> {
        if placed.rect.is_empty() {
            return None;
        }
        let offset = placed.rect.y as usize * self.size.width as usize + placed.rect.x as usize;
        Mask::new(
            &self.coverage[offset..],
            placed.rect.width,
            placed.rect.height,
            self.size.width as usize,
        )
    }

    /// Finds room for a `width` by `height` glyph.
    ///
    /// Best fit by shelf height, so a 10 px glyph does not open a 40 px shelf and
    /// waste three quarters of it.
    fn pack(&mut self, width: u32, height: u32) -> Option<Rect> {
        if width > self.size.width || height > self.size.height {
            return None;
        }

        let mut best: Option<usize> = None;
        for (index, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < height || shelf.next_x + width > self.size.width {
                continue;
            }
            let better = best.is_none_or(|b| shelf.height < self.shelves[b].height);
            if better {
                best = Some(index);
            }
        }

        if let Some(index) = best {
            let shelf = &mut self.shelves[index];
            let rect = Rect::new(
                shelf.next_x as i32,
                shelf.y as i32,
                width as i32,
                height as i32,
            );
            shelf.next_x += width;
            return Some(rect);
        }

        if self.used_height + height > self.size.height {
            return None;
        }
        let shelf = Shelf {
            y: self.used_height,
            height,
            next_x: width,
        };
        let rect = Rect::new(0, shelf.y as i32, width as i32, height as i32);
        self.used_height += height;
        self.shelves.push(shelf);
        Some(rect)
    }
}

impl core::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("size", &self.size)
            .field("glyphs", &self.entries.len())
            .field("shelves", &self.shelves.len())
            .field("stats", &self.stats)
            .finish()
    }
}
