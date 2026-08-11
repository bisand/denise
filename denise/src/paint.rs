//! Provisional pixel operations.
//!
//! Just enough to prove the [`Frame`] contract end to end in M0. The real
//! rasteriser — clipping, rounded rects, lines, source-over blending, and the
//! benches that justify their implementation — lands in `denise-render` in M1, and
//! this module goes away.

use crate::color::Color;
use crate::geom::Rect;
use crate::surface::Frame;

/// Fills the whole frame with an opaque colour.
pub fn clear(frame: &mut Frame<'_>, color: Color) {
    let word = color.to_argb8888();
    for row in frame.rows_mut() {
        row.fill(word);
    }
}

/// Fills `rect`, clipped to the frame, with an opaque colour.
///
/// Alpha is ignored. Blending is M1's problem.
pub fn fill_rect(frame: &mut Frame<'_>, rect: Rect, color: Color) {
    let Some(r) = rect.clip_to_size(frame.size()) else {
        return;
    };
    let word = color.to_argb8888();
    let x0 = r.x as usize;
    let x1 = r.right() as usize;
    for y in r.y as u32..r.bottom() as u32 {
        // `y` is inside the clipped rect, so the row exists.
        if let Some(row) = frame.row_mut(y) {
            row[x0..x1].fill(word);
        }
    }
}

/// Fills every rectangle in `rects`.
pub fn fill_rects(frame: &mut Frame<'_>, rects: &[Rect], color: Color) {
    for r in rects {
        fill_rect(frame, *r, color);
    }
}

/// Draws a one-pixel outline just inside `rect`.
pub fn stroke_rect(frame: &mut Frame<'_>, rect: Rect, color: Color) {
    if rect.is_empty() {
        return;
    }
    let w = rect.width;
    let h = rect.height;
    fill_rect(frame, Rect::new(rect.x, rect.y, w, 1), color);
    fill_rect(frame, Rect::new(rect.x, rect.bottom() - 1, w, 1), color);
    fill_rect(frame, Rect::new(rect.x, rect.y, 1, h), color);
    fill_rect(frame, Rect::new(rect.right() - 1, rect.y, 1, h), color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;
    use crate::surface::{BufferAge, PixelFormat};
    use alloc::vec;

    fn frame(buf: &mut [u32], w: u32, h: u32, stride: u32) -> Frame<'_> {
        Frame::new(
            buf,
            Size::new(w, h),
            stride,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .expect("geometry fits")
    }

    #[test]
    fn fill_respects_stride_padding() {
        let mut buf = vec![0u32; 16 * 4];
        let mut f = frame(&mut buf, 10, 4, 16);
        fill_rect(&mut f, Rect::new(0, 0, 10, 4), Color::WHITE);
        for row in buf.chunks(16) {
            assert!(row[..10].iter().all(|&p| p == 0xFFFF_FFFF));
            assert!(row[10..].iter().all(|&p| p == 0));
        }
    }

    #[test]
    fn fill_clips_to_frame() {
        let mut buf = vec![0u32; 8 * 8];
        let mut f = frame(&mut buf, 8, 8, 8);
        fill_rect(&mut f, Rect::new(-4, -4, 8, 8), Color::WHITE);
        assert_eq!(buf[0], 0xFFFF_FFFF);
        assert_eq!(buf[3], 0xFFFF_FFFF);
        assert_eq!(buf[4], 0);
        assert_eq!(buf[8 * 4], 0);
    }

    #[test]
    fn fully_offscreen_fill_is_a_noop() {
        let mut buf = vec![0u32; 8 * 8];
        let mut f = frame(&mut buf, 8, 8, 8);
        fill_rect(&mut f, Rect::new(100, 100, 8, 8), Color::WHITE);
        assert!(buf.iter().all(|&p| p == 0));
    }

    #[test]
    fn stroke_touches_only_the_border() {
        let mut buf = vec![0u32; 8 * 8];
        let mut f = frame(&mut buf, 8, 8, 8);
        stroke_rect(&mut f, Rect::new(1, 1, 4, 4), Color::WHITE);
        assert_eq!(buf[8 + 1], 0xFFFF_FFFF);
        assert_eq!(buf[8 + 4], 0xFFFF_FFFF);
        assert_eq!(buf[4 * 8 + 1], 0xFFFF_FFFF);
        assert_eq!(buf[2 * 8 + 2], 0); // interior untouched
    }
}
