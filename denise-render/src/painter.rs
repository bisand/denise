//! `Canvas` wearing the painting contract.

pub use denise::{ClipToken, Clipped, Painter, PainterExt, Pen};

use denise::{Color, Mask, PixelFormat, PixelView, Point, Rect, Size};

use crate::blend::Paint;
use crate::canvas::Canvas;

impl Painter for Canvas<'_> {
    #[inline]
    fn size(&self) -> Size {
        Canvas::size(self)
    }

    #[inline]
    fn format(&self) -> PixelFormat {
        Canvas::format(self)
    }

    #[inline]
    fn clip(&self) -> Rect {
        Canvas::clip(self)
    }

    fn push_clip(&mut self, rect: Rect) -> ClipToken {
        let previous = Canvas::clip(self);
        self.clip_to(rect);
        ClipToken::restoring(previous)
    }

    fn pop_clip(&mut self, token: ClipToken) {
        self.restore_clip(token.previous());
    }

    fn clear(&mut self, color: Color) {
        Canvas::clear(self, color);
    }

    fn fill_rect(&mut self, rect: Rect, paint: Paint) {
        Canvas::fill_rect(self, rect, paint);
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, paint: Paint) {
        Canvas::fill_rounded_rect(self, rect, radius, paint);
    }

    fn stroke_rounded_rect(&mut self, rect: Rect, radius: i32, thickness: i32, paint: Paint) {
        Canvas::stroke_rounded_rect(self, rect, radius, thickness, paint);
    }

    fn fill_circle(&mut self, centre: Point, radius: i32, paint: Paint) {
        Canvas::fill_circle(self, centre, radius, paint);
    }

    fn stroke_circle(&mut self, centre: Point, radius: i32, thickness: i32, paint: Paint) {
        Canvas::stroke_circle(self, centre, radius, thickness, paint);
    }

    fn stroke_arc(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        start: i32,
        sweep: i32,
        paint: Paint,
    ) {
        Canvas::stroke_arc(self, centre, radius, thickness, start, sweep, paint);
    }

    fn draw_line(&mut self, a: Point, b: Point, paint: Paint) {
        Canvas::draw_line(self, a, b, paint);
    }

    fn fill_polygon_fx(&mut self, points: &[(i32, i32)], paint: Paint) {
        Canvas::fill_polygon_fx(self, points, paint);
    }

    fn blit_mask(&mut self, at: Point, mask: &Mask<'_>, paint: Paint) {
        Canvas::blit_mask(self, at, mask, paint);
    }

    fn blit(&mut self, src: &PixelView<'_>, at: Point) {
        Canvas::blit(self, src, at);
    }

    fn blit_scaled(&mut self, src: &PixelView<'_>, dest: Rect) {
        Canvas::blit_scaled(self, src, dest);
    }

    fn blit_rounded(&mut self, src: &PixelView<'_>, dest: Rect, shape: Rect, radius: i32) {
        Canvas::blit_rounded(self, src, dest, shape, radius);
    }

    // `fill_rects`, `stroke_rect`, `fill_star` and `draw_icon` are the trait's
    // provided bodies -- the same arithmetic that used to live in `rect.rs`,
    // `polygon.rs` and `icon.rs`, now inherited by every backend. The inherent
    // methods of those names delegate here.
}
