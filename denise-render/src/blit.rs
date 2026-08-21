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
    /// Source pixels are premultiplied `0xAARRGGBB` — the same convention
    /// [`Paint`](crate::Paint) uses. Clipped to the canvas clip like every other
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
    /// Source pixels are premultiplied `0xAARRGGBB` — the same convention
    /// [`Paint`](crate::Paint) uses. When `dest` is exactly the source size this is
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

    /// Draws all of `src` into `dest`, masked to `shape` with rounded corners
    /// of `radius`, anti-aliased.
    ///
    /// `shape` is the rectangle whose corners are rounded and outside which
    /// nothing is drawn; sampling is still mapped from the whole of `dest`.
    /// They are separate arguments because they genuinely differ in the *Cover*
    /// case — an image scaled past its box so the box is filled edge to edge —
    /// where `dest` overflows and `shape` is the box. When the picture and the
    /// mask are the same rectangle, pass it twice. `radius` is clamped to half
    /// of `shape`'s shorter side, so a full radius on a square shape is a
    /// circle — the avatar crop. Zero draws exactly [`Canvas::blit_scaled`]
    /// restricted to `shape`.
    ///
    /// The mask must not come from the clip: the clip is damage, and a
    /// damage-restricted repaint of half an image has to round the image's
    /// corners, never the damage rectangle's.
    pub fn blit_rounded(&mut self, src: &PixelView<'_>, dest: Rect, shape: Rect, radius: i32) {
        use crate::blend::scale_premul;
        use crate::rounded::{Scan, ceil_px, floor_px};

        let Size { width, height } = src.size();
        let (sw, sh) = (width as i64, height as i64);
        let radius = radius.clamp(0, shape.width.min(shape.height) / 2);
        let Some(painted) = dest.intersect(&shape) else {
            return;
        };
        let Some(visible) = self.visible(painted) else {
            return;
        };
        for y in visible.y..visible.bottom() {
            let sy = nearest((y - dest.y) as i64, sh, dest.height as i64);
            let Some(srow) = src.row(sy as i32, 0, width as i32) else {
                continue;
            };
            // Everything between the deepest left inset and the shallowest
            // right one is fully covered, so only the fringes pay for coverage.
            let scan = (radius > 0).then(|| Scan::new(shape, radius, y));
            let (solid0, solid1) = match &scan {
                None => (visible.x, visible.right()),
                Some(scan) => (ceil_px(scan.max_left()), floor_px(scan.min_right())),
            };
            let Some(drow) = self.row_span(y, visible.x, visible.right()) else {
                continue;
            };
            for (i, d) in drow.iter_mut().enumerate() {
                let x = visible.x + i as i32;
                let coverage = if (solid0..solid1).contains(&x) {
                    255
                } else if let Some(scan) = &scan {
                    scan.coverage(x)
                } else {
                    255
                };
                if coverage == 0 {
                    continue;
                }
                let sx = nearest((x - dest.x) as i64, sw, dest.width as i64);
                let s = srow[sx as usize];
                blend_word(
                    d,
                    if coverage == 255 {
                        s
                    } else {
                        scale_premul(s, coverage)
                    },
                );
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
    fn a_rounded_blit_of_solid_white_is_a_rounded_fill() {
        // The mask arithmetic must be the same arithmetic the rounded fill
        // uses, not a lookalike: a solid white image drawn through the mask
        // has to produce fill_rounded_rect's pixels exactly, fringes included.
        let pixels = vec![0xFFFF_FFFFu32; 32 * 32];
        let src = PixelView::new(&pixels, Size::new(32, 32), 32).unwrap();
        let shape = Rect::new(2, 2, 28, 28);

        let mut blitted = TestCanvas::new(32, 32);
        blitted.canvas().blit_rounded(&src, shape, shape, 8);
        let mut filled = TestCanvas::new(32, 32);
        filled.canvas().fill_rounded_rect(shape, 8, Color::WHITE);

        assert_eq!(blitted.pixels(), filled.pixels());
    }

    #[test]
    fn a_zero_radius_rounded_blit_is_a_scaled_blit() {
        let pixels = coordinate_source(5, 5);
        let src = PixelView::new(&pixels, Size::new(5, 5), 5).unwrap();
        let dest = Rect::new(1, 1, 10, 10);

        let mut rounded = TestCanvas::new(12, 12);
        rounded.canvas().blit_rounded(&src, dest, dest, 0);
        let mut scaled = TestCanvas::new(12, 12);
        scaled.canvas().blit_scaled(&src, dest);

        assert_eq!(rounded.pixels(), scaled.pixels());
    }

    #[test]
    fn a_full_radius_on_a_square_is_the_avatar_circle() {
        let pixels = vec![0xFFFF_FFFFu32; 16 * 16];
        let src = PixelView::new(&pixels, Size::new(16, 16), 16).unwrap();
        let shape = Rect::new(0, 0, 16, 16);
        let mut t = TestCanvas::new(16, 16);
        t.canvas().blit_rounded(&src, shape, shape, 999);
        assert_eq!(t.at(0, 0), 0, "corner outside the circle");
        assert_eq!(t.at(15, 15), 0, "corner outside the circle");
        assert_eq!(t.at(8, 8), 0xFFFF_FFFF, "centre inside the circle");
        assert_eq!(t.at(8, 0) >> 24, 255, "top of the circle touches the edge");
    }

    #[test]
    fn the_shape_crops_an_overflowing_dest_the_cover_case() {
        // A 2x-scaled image mapped past its box: pixels must stop at the
        // shape, and the ones inside must be the same pixels the unmasked
        // mapping would have put there.
        let pixels = coordinate_source(8, 8);
        let src = PixelView::new(&pixels, Size::new(8, 8), 8).unwrap();
        let dest = Rect::new(-4, -4, 16, 16);
        let shape = Rect::new(2, 2, 4, 4);

        let mut masked = TestCanvas::new(8, 8);
        masked.canvas().blit_rounded(&src, dest, shape, 0);
        let mut unmasked = TestCanvas::new(8, 8);
        unmasked.canvas().blit_scaled(&src, dest);

        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (2..6).contains(&y);
                let expected = if inside { unmasked.at(x, y) } else { 0 };
                assert_eq!(masked.at(x, y), expected, "at {x},{y}");
            }
        }
    }

    #[test]
    fn damage_clipping_rounds_the_image_corners_not_the_damage_rect() {
        // Repainting half the image through a clip must reproduce exactly the
        // pixels a full repaint puts there — the mask follows the shape, and
        // the clip must not create its own corners.
        let pixels = vec![0xFFFF_FFFFu32; 24 * 24];
        let src = PixelView::new(&pixels, Size::new(24, 24), 24).unwrap();
        let shape = Rect::new(0, 0, 24, 24);

        let mut whole = TestCanvas::new(24, 24);
        whole.canvas().blit_rounded(&src, shape, shape, 8);

        let mut damaged = TestCanvas::new(24, 24);
        {
            let mut c = damaged.canvas();
            c.clip_to(Rect::new(0, 0, 12, 24));
            c.blit_rounded(&src, shape, shape, 8);
        }
        for y in 0..24 {
            for x in 0..12 {
                assert_eq!(damaged.at(x, y), whole.at(x, y), "at {x},{y}");
            }
            for x in 12..24 {
                assert_eq!(damaged.at(x, y), 0, "leaked past the clip at {x},{y}");
            }
        }
    }

    #[test]
    fn rounded_blits_survive_absurd_rectangles() {
        let pixels = coordinate_source(4, 4);
        let src = PixelView::new(&pixels, Size::new(4, 4), 4).unwrap();
        let mut t = TestCanvas::new(8, 8);
        for (dest, shape) in [
            (
                Rect::new(-1_000_000, -1_000_000, 3_000_000, 3_000_000),
                Rect::new(0, 0, 8, 8),
            ),
            (Rect::new(0, 0, 8, 8), Rect::new(100, 100, 4, 4)),
            (Rect::new(0, 0, 0, 0), Rect::new(0, 0, 8, 8)),
            (
                Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX, i32::MAX),
                Rect::new(2, 2, 4, 4),
            ),
        ] {
            t.canvas().blit_rounded(&src, dest, shape, 3);
        }
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
