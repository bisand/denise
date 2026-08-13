//! Drawing somebody else's pixels: the positioned, blended blit.
//!
//! [`Canvas::copy_from`] is the damage-publish blit — same coordinates both
//! sides, no blending, not for pictures. These are the picture operations: a
//! borrowed block of pixels drawn at a position, composited per pixel, clipped
//! like everything else.
//!
//! # The source is premultiplied
//!
//! Source pixels are `0xAARRGGBB` with the colour channels already multiplied
//! by alpha — the same convention [`Paint`](crate::Paint) uses internally, and
//! for the same reason: the multiply happens once when the image is prepared,
//! not once per pixel per frame. Decoders produce straight alpha;
//! [`blend::premultiply`](crate::blend::premultiply) converts a buffer in
//! place, once. Fully opaque pixels are identical in both conventions, so an
//! image with no transparency needs no conversion at all.
//!
//! # Scaling is nearest-neighbour
//!
//! [`Canvas::blit_scaled`] samples at pixel centres, integer arithmetic only —
//! exact for icons, QR codes and pixel art, and for integer upscales each
//! source pixel becomes a crisp block. Bilinear is what a photo wants and is
//! deliberately absent until it has been benched on the Pi-class targets;
//! pre-sizing assets is the embedded answer in the meantime.

use denise::{Point, Rect, Size};

use crate::blend::source_over;
use crate::canvas::{Canvas, PixelView};

/// Composites one premultiplied source word over a destination pixel, with the
/// two cheap exits a picture is mostly made of.
#[inline(always)]
fn blend_word(dst: &mut u32, src: u32) {
    match src >> 24 {
        0 => {}
        255 => *dst = src,
        a => *dst = source_over(*dst, src, a),
    }
}

/// Maps a destination index to a source index by pixel centre: the nearest
/// source pixel to `(d + 0.5) * src_len / dst_len`, in integers.
#[inline(always)]
fn nearest(d: i64, src_len: i64, dst_len: i64) -> i64 {
    (2 * d + 1) * src_len / (2 * dst_len)
}

impl Canvas<'_> {
    /// Draws `src` with its top-left corner at `at`, one source pixel per
    /// destination pixel, composited with source-over alpha.
    ///
    /// Source pixels are premultiplied `0xAARRGGBB` — see the
    /// [module docs](self). Clipped to the canvas clip like every other
    /// operation; any part of the image outside it is simply not drawn.
    pub fn blit(&mut self, src: &PixelView<'_>, at: Point) {
        let Size { width, height } = src.size();
        let dest = Rect::new(at.x, at.y, width as i32, height as i32);
        let Some(visible) = self.visible(dest) else {
            return;
        };
        for y in visible.y..visible.bottom() {
            let Some(srow) = src.row(y - at.y, visible.x - at.x, visible.right() - at.x) else {
                continue;
            };
            let Some(drow) = self.row_span(y, visible.x, visible.right()) else {
                continue;
            };
            for (d, &s) in drow.iter_mut().zip(srow) {
                blend_word(d, s);
            }
        }
    }

    /// Draws all of `src` into `dest`, nearest-neighbour resampled, composited
    /// with source-over alpha.
    ///
    /// Source pixels are premultiplied `0xAARRGGBB` — see the
    /// [module docs](self). When `dest` is exactly the source size this is
    /// [`Canvas::blit`]. Sampling is at pixel centres, so an integer upscale
    /// turns each source pixel into an even block, and a downscale picks
    /// representative pixels rather than always the top-left ones.
    pub fn blit_scaled(&mut self, src: &PixelView<'_>, dest: Rect) {
        // A `PixelView` is never empty by construction, and an empty or
        // negative `dest` never survives `visible`, so the divisions in
        // `nearest` cannot see a zero.
        let Size { width, height } = src.size();
        let (sw, sh) = (width as i64, height as i64);
        if dest.width == width as i32 && dest.height == height as i32 {
            return self.blit(src, Point::new(dest.x, dest.y));
        }
        let Some(visible) = self.visible(dest) else {
            return;
        };
        for y in visible.y..visible.bottom() {
            let sy = nearest((y - dest.y) as i64, sh, dest.height as i64);
            let Some(srow) = src.row(sy as i32, 0, width as i32) else {
                continue;
            };
            let Some(drow) = self.row_span(y, visible.x, visible.right()) else {
                continue;
            };
            for (i, d) in drow.iter_mut().enumerate() {
                let dx = (visible.x - dest.x) as i64 + i as i64;
                let sx = nearest(dx, sw, dest.width as i64);
                blend_word(d, srow[sx as usize]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::{Paint, premultiply};
    use crate::testing::TestCanvas;
    use denise::Color;

    /// A source whose every pixel encodes its own coordinates, so any
    /// misplacement is visible in the value itself.
    fn coordinate_source(width: u32, height: u32) -> Vec<u32> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| 0xFF00_0000 | (x << 8) | y))
            .collect()
    }

    #[test]
    fn a_blit_lands_exactly_where_it_is_put() {
        let pixels = coordinate_source(3, 2);
        let src = PixelView::new(&pixels, Size::new(3, 2), 3).unwrap();
        let mut t = TestCanvas::new(8, 8);
        t.canvas().blit(&src, Point::new(2, 3));
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..5).contains(&x) && (3..5).contains(&y);
                let expected = if inside {
                    0xFF00_0000 | (((x - 2) as u32) << 8) | (y - 3) as u32
                } else {
                    0
                };
                assert_eq!(t.at(x, y), expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn a_blit_is_clipped_at_every_edge() {
        // Hang the image off each corner in turn; only the overlap may change.
        let pixels = coordinate_source(4, 4);
        let src = PixelView::new(&pixels, Size::new(4, 4), 4).unwrap();
        for at in [
            Point::new(-2, -2),
            Point::new(6, -2),
            Point::new(-2, 6),
            Point::new(6, 6),
        ] {
            let mut t = TestCanvas::new(8, 8);
            t.canvas().blit(&src, at);
            for y in 0..8i32 {
                for x in 0..8i32 {
                    let (sx, sy) = (x - at.x, y - at.y);
                    let inside = (0..4).contains(&sx) && (0..4).contains(&sy);
                    let expected = if inside {
                        0xFF00_0000 | ((sx as u32) << 8) | sy as u32
                    } else {
                        0
                    };
                    assert_eq!(t.at(x, y), expected, "at {x},{y} for corner {at:?}");
                }
            }
        }
    }

    #[test]
    fn a_blit_respects_the_canvas_clip() {
        let pixels = vec![0xFFFF_FFFF; 64];
        let src = PixelView::new(&pixels, Size::new(8, 8), 8).unwrap();
        let mut t = TestCanvas::new(8, 8);
        {
            let mut c = t.canvas();
            c.clip_to(Rect::new(2, 2, 4, 4));
            c.blit(&src, Point::new(0, 0));
        }
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (2..6).contains(&y);
                assert_eq!(t.at(x, y) != 0, inside, "at {x},{y}");
            }
        }
    }

    #[test]
    fn a_blit_respects_the_stride_padding() {
        let pixels = vec![0xFFFF_FFFF; 4];
        let src = PixelView::new(&pixels, Size::new(2, 2), 2).unwrap();
        let mut t = TestCanvas::with_stride(4, 4, 9);
        t.canvas().blit(&src, Point::new(2, 0));
        // The two padding words after the first drawn row stay untouched.
        assert_eq!(t.at(3, 0), 0xFFFF_FFFF);
        assert_eq!(t.pixels()[4], 0, "padding written");
        assert_eq!(t.pixels()[8], 0, "padding written");
        assert_eq!(t.at(3, 1), 0xFFFF_FFFF);
    }

    #[test]
    fn alpha_pixels_composite_by_the_blend_rules() {
        // A premultiplied half-alpha source pixel must land exactly where the
        // rasteriser's own arithmetic puts it — one rule, not two.
        let color = Color::rgba(200, 100, 50, 128);
        let paint = Paint::new(color);
        let mut pixels = vec![u32::from_be_bytes([color.a, color.r, color.g, color.b])];
        premultiply(&mut pixels);
        assert_eq!(pixels[0], paint.premultiplied());

        let src = PixelView::new(&pixels, Size::new(1, 1), 1).unwrap();
        let mut t = TestCanvas::new(1, 1);
        t.canvas().clear(Color::rgb(0, 0, 64));
        let background = t.at(0, 0);
        t.canvas().blit(&src, Point::new(0, 0));
        assert_eq!(
            t.at(0, 0),
            source_over(background, paint.premultiplied(), paint.alpha())
        );
    }

    #[test]
    fn transparent_pixels_leave_the_destination_alone() {
        let pixels = vec![0u32; 4];
        let src = PixelView::new(&pixels, Size::new(2, 2), 2).unwrap();
        let mut t = TestCanvas::new(2, 2);
        t.canvas().clear(Color::rgb(10, 20, 30));
        let before = t.pixels().to_vec();
        t.canvas().blit(&src, Point::new(0, 0));
        assert_eq!(t.pixels(), &before[..]);
    }

    #[test]
    fn scaling_to_the_source_size_is_a_plain_blit() {
        let pixels = coordinate_source(3, 3);
        let src = PixelView::new(&pixels, Size::new(3, 3), 3).unwrap();
        let mut plain = TestCanvas::new(8, 8);
        plain.canvas().blit(&src, Point::new(2, 2));
        let mut scaled = TestCanvas::new(8, 8);
        scaled.canvas().blit_scaled(&src, Rect::new(2, 2, 3, 3));
        assert_eq!(plain.pixels(), scaled.pixels());
    }

    #[test]
    fn an_integer_upscale_makes_even_blocks() {
        let pixels = coordinate_source(2, 2);
        let src = PixelView::new(&pixels, Size::new(2, 2), 2).unwrap();
        let mut t = TestCanvas::new(4, 4);
        t.canvas().blit_scaled(&src, Rect::new(0, 0, 4, 4));
        for y in 0..4 {
            for x in 0..4 {
                let expected = 0xFF00_0000 | (((x / 2) as u32) << 8) | (y / 2) as u32;
                assert_eq!(t.at(x, y), expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn a_downscale_samples_pixel_centres_not_corners() {
        // Halving 8 wide to 4 must pick columns 1, 3, 5, 7 — the centres —
        // not 0, 2, 4, 6, which a floor-of-left-edge mapping would give.
        let pixels = coordinate_source(8, 1);
        let src = PixelView::new(&pixels, Size::new(8, 1), 8).unwrap();
        let mut t = TestCanvas::new(4, 1);
        t.canvas().blit_scaled(&src, Rect::new(0, 0, 4, 1));
        for x in 0..4 {
            let expected = 0xFF00_0000 | (((2 * x + 1) as u32) << 8);
            assert_eq!(t.at(x, 0), expected, "at {x}");
        }
    }

    #[test]
    fn a_clipped_scaled_blit_samples_as_if_unclipped() {
        // The part of a scaled image that survives clipping must be the same
        // pixels it would have been without the clip.
        let pixels = coordinate_source(5, 5);
        let src = PixelView::new(&pixels, Size::new(5, 5), 5).unwrap();
        let dest = Rect::new(-3, -3, 13, 13);

        let mut whole = TestCanvas::new(16, 16);
        whole.canvas().blit_scaled(&src, Rect::new(5, 5, 13, 13));
        let mut clipped = TestCanvas::new(8, 8);
        clipped.canvas().blit_scaled(&src, dest);

        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(clipped.at(x, y), whole.at(x + 8, y + 8), "at {x},{y}");
            }
        }
    }

    #[test]
    fn absurd_rectangles_neither_read_nor_write_out_of_bounds() {
        let pixels = coordinate_source(4, 4);
        let src = PixelView::new(&pixels, Size::new(4, 4), 4).unwrap();
        let mut t = TestCanvas::new(8, 8);
        for dest in [
            Rect::new(-1_000_000, -1_000_000, 3_000_000, 3_000_000),
            Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX, i32::MAX),
            Rect::new(0, 0, i32::MAX, 1),
            Rect::new(4, 4, 0, 5),
            Rect::new(4, 4, 5, 0),
            Rect::new(100, 100, 4, 4),
        ] {
            t.canvas().blit_scaled(&src, dest);
        }
        let empty = PixelView::new(&pixels, Size::new(4, 4), 4).unwrap();
        t.canvas()
            .blit(&empty, Point::new(i32::MAX - 1, i32::MAX - 1));
        t.canvas().blit(&empty, Point::new(i32::MIN, i32::MIN));
    }

    #[test]
    fn premultiply_is_exact_and_paints_agree() {
        for (r, g, b, a) in [
            (255, 255, 255, 255),
            (0xAB, 0xCD, 0xEF, 0),
            (200, 100, 50, 128),
        ] {
            let mut px = [u32::from_be_bytes([a, r, g, b])];
            premultiply(&mut px);
            assert_eq!(
                px[0],
                Paint::new(Color::rgba(r, g, b, a)).premultiplied(),
                "rgba({r},{g},{b},{a})"
            );
        }
    }
}
