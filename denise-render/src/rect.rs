//! Axis-aligned rectangles.

use denise::Rect;

use crate::blend::{Paint, blend_span};
use crate::canvas::Canvas;

impl Canvas<'_> {
    /// Fills `rect`, compositing with the destination if the paint has alpha.
    pub fn fill_rect(&mut self, rect: Rect, color: impl Into<Paint>) {
        let paint = color.into();
        if paint.is_invisible() {
            return;
        }
        let Some(r) = self.visible(rect) else {
            return;
        };
        for y in r.y..r.bottom() {
            if let Some(span) = self.row_span(y, r.x, r.right()) {
                blend_span(span, paint);
            }
        }
    }

    /// Fills every rectangle. Overlapping rectangles composite twice.
    pub fn fill_rects(&mut self, rects: &[Rect], color: impl Into<Paint>) {
        let paint = color.into();
        for rect in rects {
            self.fill_rect(*rect, paint);
        }
    }

    /// Draws a border of `thickness` pixels *inside* `rect`.
    ///
    /// The four bands are cut so they do not overlap. Drawing them as four
    /// full-length rectangles would composite the corners twice, which is invisible
    /// at full alpha and obvious at anything less.
    pub fn stroke_rect(&mut self, rect: Rect, thickness: i32, color: impl Into<Paint>) {
        let paint = color.into();
        let t = thickness.max(0);
        if t == 0 || rect.is_empty() || paint.is_invisible() {
            return;
        }
        if t * 2 >= rect.width.min(rect.height) {
            self.fill_rect(rect, paint);
            return;
        }

        let inner_height = rect.height - 2 * t;
        self.fill_rect(Rect::new(rect.x, rect.y, rect.width, t), paint);
        self.fill_rect(Rect::new(rect.x, rect.bottom() - t, rect.width, t), paint);
        self.fill_rect(Rect::new(rect.x, rect.y + t, t, inner_height), paint);
        self.fill_rect(
            Rect::new(rect.right() - t, rect.y + t, t, inner_height),
            paint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestCanvas;
    use denise::Color;

    #[test]
    fn fill_clips_to_the_canvas() {
        let mut t = TestCanvas::new(8, 8);
        t.canvas().fill_rect(Rect::new(-4, -4, 8, 8), Color::WHITE);
        assert_eq!(t.at(0, 0), 0xFFFF_FFFF);
        assert_eq!(t.at(3, 3), 0xFFFF_FFFF);
        assert_eq!(t.at(4, 0), 0);
        assert_eq!(t.at(0, 4), 0);
    }

    #[test]
    fn fill_never_touches_stride_padding() {
        let mut t = TestCanvas::with_stride(10, 4, 16);
        t.canvas().fill_rect(Rect::new(0, 0, 10, 4), Color::WHITE);
        for row in t.pixels().chunks(16) {
            assert!(row[..10].iter().all(|&p| p == 0xFFFF_FFFF));
            assert!(row[10..].iter().all(|&p| p == 0));
        }
    }

    #[test]
    fn wholly_offscreen_fill_is_a_noop() {
        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .fill_rect(Rect::new(100, 100, 8, 8), Color::WHITE);
        assert!(t.pixels().iter().all(|&p| p == 0));
    }

    #[test]
    fn half_alpha_twice_is_not_full_alpha() {
        let mut t = TestCanvas::new(4, 4);
        t.canvas()
            .fill_rect(Rect::new(0, 0, 4, 4), Color::rgba(255, 255, 255, 128));
        let once = t.at(0, 0);
        t.canvas()
            .fill_rect(Rect::new(0, 0, 4, 4), Color::rgba(255, 255, 255, 128));
        let twice = t.at(0, 0);
        assert!(twice > once, "second coat must lighten further");
        assert!(twice < 0xFFFF_FFFF, "two half coats must not reach opaque");
    }

    #[test]
    fn stroke_corners_are_not_composited_twice() {
        let mut t = TestCanvas::new(16, 16);
        t.canvas()
            .stroke_rect(Rect::new(0, 0, 16, 16), 2, Color::rgba(255, 255, 255, 128));
        // A corner pixel and an edge pixel get exactly one coat each.
        assert_eq!(t.at(0, 0), t.at(8, 0));
        assert_eq!(t.at(0, 0), t.at(0, 8));
    }

    #[test]
    fn stroke_leaves_the_interior_alone() {
        let mut t = TestCanvas::new(16, 16);
        t.canvas()
            .stroke_rect(Rect::new(2, 2, 12, 12), 3, Color::WHITE);
        assert_eq!(t.at(2, 2), 0xFFFF_FFFF);
        assert_eq!(t.at(4, 4), 0xFFFF_FFFF);
        assert_eq!(t.at(5, 5), 0, "interior must be untouched");
        assert_eq!(t.at(1, 1), 0, "outside must be untouched");
    }

    #[test]
    fn stroke_thicker_than_the_rect_becomes_a_fill() {
        let mut t = TestCanvas::new(8, 8);
        t.canvas()
            .stroke_rect(Rect::new(1, 1, 4, 4), 9, Color::WHITE);
        assert_eq!(t.at(2, 2), 0xFFFF_FFFF);
        assert_eq!(t.at(0, 0), 0);
    }
}
