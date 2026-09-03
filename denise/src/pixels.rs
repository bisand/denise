//! The borrowed buffers a painter is handed.
//!
//! Neither of these rasterises anything: a [`Mask`] is a rectangle of 8-bit
//! coverage — how an anti-aliased glyph arrives — and a [`PixelView`] is
//! somebody else's premultiplied pixels. Every backend is given the same two
//! shapes, so they belong beside [`Frame`] rather than inside any
//! one renderer.

use crate::geom::{Point, Rect, Size};
use crate::surface::Frame;

/// A read-only view of somebody else's pixels, for [`Canvas::copy_from`](https://docs.rs/denise-render).
#[derive(Clone, Copy, Debug)]
pub struct PixelView<'a> {
    pixels: &'a [u32],
    size: Size,
    stride: usize,
}

impl<'a> PixelView<'a> {
    /// Wraps a pixel slice. Returns `None` if `pixels` is too small for the
    /// geometry, or if `stride` is narrower than `size.width`.
    pub fn new(pixels: &'a [u32], size: Size, stride: u32) -> Option<Self> {
        if size.is_empty() || stride < size.width {
            return None;
        }
        // `required_words` computes in `u64` on purpose; see its documentation for
        // the 32-bit wrap this avoids.
        let required = crate::required_words(size, stride);
        let stride = stride as usize;
        (pixels.len() as u64 >= required).then_some(Self {
            pixels,
            size,
            stride,
        })
    }

    /// Borrows a frame's pixels for reading.
    pub fn from_frame(frame: &'a Frame<'_>) -> Option<Self> {
        Self::new(frame.pixels(), frame.size(), frame.stride())
    }

    /// Visible extent.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The pixels of row `y` from `x0` to `x1`, or `None` if that span falls
    /// outside the view.
    #[inline]
    pub fn row(&self, y: i32, x0: i32, x1: i32) -> Option<&[u32]> {
        if y < 0 || y >= self.size.height as i32 || x0 >= x1 || x0 < 0 {
            return None;
        }
        let base = y as usize * self.stride;
        self.pixels.get(base + x0 as usize..base + x1 as usize)
    }
}

/// A borrowed 8-bit coverage mask: `0` transparent, `255` fully covered.
#[derive(Clone, Copy, Debug)]
pub struct Mask<'a> {
    data: &'a [u8],
    width: i32,
    height: i32,
    stride: usize,
}

impl<'a> Mask<'a> {
    /// Wraps a coverage buffer. Returns `None` if it is too small for the
    /// geometry, or if `stride` is narrower than `width`.
    pub fn new(data: &'a [u8], width: i32, height: i32, stride: usize) -> Option<Self> {
        if width <= 0 || height <= 0 || stride < width as usize {
            return None;
        }
        let required = stride * (height as usize - 1) + width as usize;
        (data.len() >= required).then_some(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// A mask whose rows are contiguous.
    pub fn packed(data: &'a [u8], width: i32, height: i32) -> Option<Self> {
        Self::new(data, width, height, width.max(0) as usize)
    }

    /// Width in pixels.
    #[inline]
    pub const fn width(&self) -> i32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub const fn height(&self) -> i32 {
        self.height
    }

    /// Extent as a rectangle placed at `at`.
    #[inline]
    pub const fn bounds_at(&self, at: Point) -> Rect {
        Rect::new(at.x, at.y, self.width, self.height)
    }

    /// The stride between rows, in bytes.
    #[inline]
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// The part of this mask inside `rect`, sharing its bytes.
    ///
    /// `None` if `rect` is empty or reaches outside the mask. This is how a
    /// glyph is cut from an atlas page without copying it.
    pub fn sub(&self, rect: Rect) -> Option<Mask<'a>> {
        if rect.is_empty()
            || rect.x < 0
            || rect.y < 0
            || rect.right() > self.width
            || rect.bottom() > self.height
        {
            return None;
        }
        let offset = rect.y as usize * self.stride + rect.x as usize;
        Mask::new(&self.data[offset..], rect.width, rect.height, self.stride)
    }

    /// The coverage values of row `y`, which the caller must know is in range.
    #[inline]
    pub fn row(&self, y: i32) -> &'a [u8] {
        let base = y as usize * self.stride;
        &self.data[base..base + self.width as usize]
    }
}

/// A glyph atlas page, with the identity a painter needs to cache it.
///
/// A text engine keeps its glyphs on one page of coverage and hands a painter
/// a rectangle of it per glyph. The software rasteriser composites the
/// rectangle and is done; a GPU would rather upload the page once and draw
/// rectangles of it forever. `id` says which page this is and `version` says
/// whether the bytes are the ones it uploaded last time: it changes whenever a
/// glyph is packed or the page is reset, and never otherwise.
#[derive(Clone, Copy, Debug)]
pub struct AtlasPage<'a> {
    /// Unique per atlas for the life of the process.
    pub id: u64,
    /// Changes whenever the page's bytes do.
    pub version: u64,
    /// The whole page.
    pub mask: Mask<'a>,
}
