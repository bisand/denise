//! The drawing target: a borrowed frame plus a clip rectangle.

use denise::{Color, Frame, PixelFormat, Rect, Size};

use crate::blend::{Paint, blend_pixel, blend_span};

/// A read-only view of somebody else's pixels, for [`Canvas::copy_from`].
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
        let required = denise::required_words(size, stride);
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

    #[inline]
    pub(crate) fn row(&self, y: i32, x0: i32, x1: i32) -> Option<&[u32]> {
        if y < 0 || y >= self.size.height as i32 || x0 >= x1 || x0 < 0 {
            return None;
        }
        let base = y as usize * self.stride;
        self.pixels.get(base + x0 as usize..base + x1 as usize)
    }
}

/// A clipped, writable pixel target.
///
/// Every operation is clipped to [`Canvas::clip`], which starts as the whole frame
/// and only ever shrinks. Clipping is rectangular: that covers scrolling regions,
/// damage-restricted repaint and nested panels, which is all a UI actually needs.
/// Arbitrary clip shapes are not planned.
///
/// Coordinates are physical pixels relative to the frame origin, never relative to
/// the clip.
#[derive(Debug)]
pub struct Canvas<'a> {
    pixels: &'a mut [u32],
    size: Size,
    stride: usize,
    format: PixelFormat,
    clip: Rect,
}

impl<'a> Canvas<'a> {
    /// Borrows a frame for drawing, clipped to the whole frame.
    pub fn new(frame: &'a mut Frame<'_>) -> Self {
        let size = frame.size();
        let stride = frame.stride() as usize;
        let format = frame.format();
        Self {
            pixels: frame.pixels_mut(),
            size,
            stride,
            format,
            clip: Rect::from_size(size),
        }
    }

    /// Wraps a raw pixel slice. Returns `None` if it is too small for the geometry.
    ///
    /// Prefer [`Canvas::new`]; this exists for offscreen buffers and benchmarks that
    /// have no [`Frame`] to hand.
    pub fn from_pixels(
        pixels: &'a mut [u32],
        size: Size,
        stride: u32,
        format: PixelFormat,
    ) -> Option<Self> {
        if size.is_empty() || stride < size.width {
            return None;
        }
        let stride = stride as usize;
        let required = stride * (size.height as usize - 1) + size.width as usize;
        (pixels.len() >= required).then_some(Self {
            pixels,
            size,
            stride,
            format,
            clip: Rect::from_size(size),
        })
    }

    /// Full extent of the underlying frame.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Word layout of the target.
    #[inline]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// The region operations are currently restricted to.
    #[inline]
    pub const fn clip(&self) -> Rect {
        self.clip
    }

    /// Narrows the clip in place. Never widens it.
    pub fn clip_to(&mut self, rect: Rect) {
        self.clip = self.clip.intersect(&rect).unwrap_or(Rect::ZERO);
    }

    /// A canvas over the same pixels with a tighter clip.
    ///
    /// The borrow ends when the returned canvas is dropped, so this is how a parent
    /// hands a child a region to draw in without either being able to escape it.
    pub fn with_clip(&mut self, rect: Rect) -> Canvas<'_> {
        let clip = self.clip.intersect(&rect).unwrap_or(Rect::ZERO);
        Canvas {
            pixels: self.pixels,
            size: self.size,
            stride: self.stride,
            format: self.format,
            clip,
        }
    }

    /// Returns `true` if the clip admits no pixels, so drawing can be skipped.
    #[inline]
    pub const fn is_clipped_out(&self) -> bool {
        self.clip.is_empty()
    }

    /// The clipped, visible part of `rect`.
    #[inline]
    pub fn visible(&self, rect: Rect) -> Option<Rect> {
        self.clip.intersect(&rect)
    }

    /// A writable span of row `y` from `x0` to `x1`, clipped. `None` if empty.
    #[inline]
    pub(crate) fn row_span(&mut self, y: i32, x0: i32, x1: i32) -> Option<&mut [u32]> {
        if y < self.clip.y || y >= self.clip.bottom() {
            return None;
        }
        let x0 = x0.max(self.clip.x);
        let x1 = x1.min(self.clip.right());
        if x0 >= x1 {
            return None;
        }
        let base = y as usize * self.stride;
        Some(&mut self.pixels[base + x0 as usize..base + x1 as usize])
    }

    /// Composites a paint over one pixel at `coverage` (`0..=255`), clipped.
    #[inline]
    pub(crate) fn blend_at(&mut self, x: i32, y: i32, paint: Paint, coverage: u32) {
        if coverage == 0 {
            return;
        }
        if let Some(span) = self.row_span(y, x, x + 1)
            && let Some(px) = span.first_mut()
        {
            blend_pixel(px, paint, coverage);
        }
    }

    /// Fills the entire clip with an opaque colour.
    ///
    /// This is the full-frame clear when the clip is untouched, and the
    /// damage-restricted clear when it is not.
    pub fn clear(&mut self, color: Color) {
        let paint = Paint::new(Color::rgb(color.r, color.g, color.b));
        let clip = self.clip;
        for y in clip.y..clip.bottom() {
            if let Some(span) = self.row_span(y, clip.x, clip.right()) {
                blend_span(span, paint);
            }
        }
    }

    /// Copies matching regions out of another buffer.
    ///
    /// Source and destination coordinates are the same, so this is the
    /// damage-driven "publish what changed" blit a double-buffered backend needs —
    /// not a general blitter. Regions are clipped to both buffers.
    pub fn copy_from(&mut self, src: &PixelView<'_>, regions: &[Rect]) {
        let bounds = Rect::from_size(Size::new(
            self.size.width.min(src.size.width),
            self.size.height.min(src.size.height),
        ));
        for region in regions {
            let Some(r) = region.intersect(&bounds) else {
                continue;
            };
            for y in r.y..r.bottom() {
                let Some(source) = src.row(y, r.x, r.right()) else {
                    continue;
                };
                // Re-clip through row_span so the canvas clip still applies, then
                // trim the source to whatever survived.
                if let Some(dst) = self.row_span(y, r.x, r.right()) {
                    let n = dst.len().min(source.len());
                    dst[..n].copy_from_slice(&source[..n]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;

    #[test]
    fn clip_only_narrows() {
        let mut t = TestCanvas::new(16, 16);
        let mut c = t.canvas();
        c.clip_to(Rect::new(4, 4, 8, 8));
        c.clip_to(Rect::new(0, 0, 16, 16));
        assert_eq!(c.clip(), Rect::new(4, 4, 8, 8));
    }

    #[test]
    fn disjoint_clip_is_empty_not_negative() {
        let mut t = TestCanvas::new(16, 16);
        {
            let mut c = t.canvas();
            c.clip_to(Rect::new(0, 0, 4, 4));
            c.clip_to(Rect::new(8, 8, 4, 4));
            assert!(c.is_clipped_out());
            c.clear(Color::WHITE);
        }
        assert!(t.pixels().iter().all(|&p| p == 0));
    }

    #[test]
    fn nested_clip_borrow_restores_the_parent() {
        let mut t = TestCanvas::new(16, 16);
        let mut c = t.canvas();
        {
            let mut child = c.with_clip(Rect::new(0, 0, 4, 4));
            child.clear(Color::WHITE);
        }
        assert_eq!(c.clip(), Rect::new(0, 0, 16, 16));
    }

    #[test]
    fn clear_respects_the_clip_and_the_stride() {
        let mut t = TestCanvas::with_stride(10, 4, 16);
        {
            let mut c = t.canvas();
            c.clip_to(Rect::new(2, 1, 4, 2));
            c.clear(Color::WHITE);
        }
        for (y, row) in t.pixels().chunks(16).enumerate() {
            for (x, &px) in row.iter().enumerate() {
                let inside = (2..6).contains(&x) && (1..3).contains(&y);
                assert_eq!(px == 0xFFFF_FFFF, inside, "at {x},{y}");
            }
        }
    }

    #[test]
    fn copy_from_moves_only_the_listed_regions() {
        let mut source = TestCanvas::new(8, 8);
        source.canvas().clear(Color::from_argb8888(0xFFAA_BBCC));

        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .copy_from(&source.view(), &[Rect::new(2, 2, 3, 3)]);

        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..5).contains(&x) && (2..5).contains(&y);
                let expected = if inside { 0xFFAA_BBCC } else { 0 };
                assert_eq!(t.pixels()[y * 8 + x], expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn copy_from_clips_to_the_smaller_buffer() {
        let mut source = TestCanvas::new(4, 4);
        source.canvas().clear(Color::from_argb8888(0xFFAA_BBCC));

        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .copy_from(&source.view(), &[Rect::new(0, 0, 8, 8)]);

        assert_eq!(t.pixels()[0], 0xFFAA_BBCC);
        assert_eq!(t.pixels()[3], 0xFFAA_BBCC);
        assert_eq!(t.pixels()[4], 0);
        assert_eq!(t.pixels()[4 * 8], 0);
    }

    #[test]
    fn pixel_view_rejects_undersized_slices() {
        let pixels = [0u32; 10];
        assert!(PixelView::new(&pixels, Size::new(4, 4), 4).is_none());
        assert!(PixelView::new(&pixels, Size::new(4, 2), 2).is_none());
    }
}
