//! The composite cursor sprite.
//!
//! Step 4 of the rendering pipeline, and the reason the project is named after a
//! display chip. Denise the 8362 overlaid eight hardware sprites on the playfield
//! after compositing it; this overlays one, in software, onto the finished scene.
//!
//! Keeping the cursor out of the scene graph is deliberate. It is not a widget: it
//! never takes input, it must draw above every scene including modals, and it moves
//! far more often than anything else on screen. As a sprite it costs two small
//! damage rectangles per move — the pixels it left and the pixels it now covers —
//! and nothing else in the tree has to know it exists.
//!
//! On DRM this is not the path taken: `denise-drm` implements
//! [`CursorPlane`](denise::CursorPlane), and the display controller composites the
//! sprite during scanout so a pointer move costs one ioctl instead of a repaint
//! and a flip. Call [`Ui::show_cursor(false)`](crate::Ui::show_cursor) and drive
//! the plane instead. What follows is the fallback for every backend without one,
//! and [`CursorImage::rasterise`] is how the same sprite reaches the plane.

use denise::{Color, Point, Rect, Role, Theme};
use denise::Pen;

/// A cursor bitmap: three levels, drawn in two theme colours.
///
/// `mask` is one ASCII byte per pixel in row-major order, which keeps the shape
/// readable in the source instead of being a wall of hex:
///
/// - `.` transparent
/// - `#` fill, painted in [`Role::BaseContent`]
/// - `+` outline, painted in [`Role::Base100`]
///
/// Both colours come from the theme, so the pointer inverts with it and stays
/// visible on a light panel and a dark one without a second asset.
#[derive(Clone, Copy, Debug)]
pub struct CursorImage {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// The pixel that sits on the pointer position.
    pub hotspot: Point,
    /// `width * height` ASCII bytes.
    pub mask: &'static [u8],
}

impl CursorImage {
    /// Returns `true` if `mask` matches the declared geometry.
    #[inline]
    pub const fn is_well_formed(&self) -> bool {
        self.width > 0 && self.height > 0 && self.mask.len() == (self.width * self.height) as usize
    }

    /// Writes the sprite into `out` as `0xAARRGGBB` words, for a hardware cursor
    /// plane.
    ///
    /// Returns the number of words written, which is `width * height`. The two
    /// colours are the theme's, exactly as the software composite uses them, so a
    /// panel that switches to the plane does not also change appearance —
    /// transparent pixels come out as a fully zero word rather than as black,
    /// because a cursor plane composites during scanout and an opaque pad would
    /// draw a rectangle around the pointer.
    ///
    /// Re-run this when the theme changes: the sprite is resolved to concrete
    /// colours here, so the plane holds pixels rather than roles.
    pub fn rasterise(&self, theme: &Theme, out: &mut [u32]) -> usize {
        let needed = (self.width.max(0) * self.height.max(0)) as usize;
        if !self.is_well_formed() || out.len() < needed {
            return 0;
        }
        let fill = theme.color(Role::BaseContent).to_argb8888();
        let outline = theme.color(Role::Base100).to_argb8888();
        for (pixel, &value) in out[..needed].iter_mut().zip(self.mask) {
            *pixel = match value {
                b'#' => fill,
                b'+' => outline,
                _ => 0,
            };
        }
        needed
    }

    /// Bounds the sprite would occupy with its hotspot at `at`.
    #[inline]
    pub fn bounds_at(&self, at: Point) -> Rect {
        Rect::new(
            at.x - self.hotspot.x,
            at.y - self.hotspot.y,
            self.width,
            self.height,
        )
    }
}

/// The standard left-pointing arrow, 12×18, hotspot at the tip.
pub const ARROW: CursorImage = CursorImage {
    width: 12,
    height: 18,
    hotspot: Point::new(0, 0),
    mask: concat!(
        "+...........",
        "++..........",
        "+#+.........",
        "+##+........",
        "+###+.......",
        "+####+......",
        "+#####+.....",
        "+######+....",
        "+#######+...",
        "+########+..",
        "+#####+++++.",
        "+##+##+.....",
        "+#+.+##+....",
        "++..+##+....",
        ".....+##+...",
        ".....+##+...",
        "......+#+...",
        "......+++...",
    )
    .as_bytes(),
};

/// A crosshair for touch calibration and precise pointing, 15×15, centred.
pub const CROSSHAIR: CursorImage = CursorImage {
    width: 15,
    height: 15,
    hotspot: Point::new(7, 7),
    mask: concat!(
        "......+#+......",
        "......+#+......",
        "......+#+......",
        "......+#+......",
        "......+#+......",
        "......+++......",
        "+++++.....+++++",
        "#####..#..#####",
        "+++++.....+++++",
        "......+++......",
        "......+#+......",
        "......+#+......",
        "......+#+......",
        "......+#+......",
        "......+#+......",
    )
    .as_bytes(),
};

/// Where the pointer is and what it looks like.
#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    /// The sprite to draw.
    pub image: &'static CursorImage,
    /// Hotspot position in surface pixels.
    pub position: Point,
    /// Whether the sprite is composited at all.
    ///
    /// Starts hidden. A panel driven only by touch should never show a pointer,
    /// so the tree reveals it on the first pointer motion and hides it again when
    /// a finger arrives.
    pub visible: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            image: &ARROW,
            position: Point::ZERO,
            visible: false,
        }
    }
}

impl Cursor {
    /// Bounds the sprite currently occupies, empty when hidden.
    #[inline]
    pub fn bounds(&self) -> Rect {
        if self.visible {
            self.image.bounds_at(self.position)
        } else {
            Rect::ZERO
        }
    }

    /// Composites the sprite onto an already-finished scene.
    pub fn paint(&self, theme: &Theme, canvas: &mut Pen<'_>) {
        if !self.visible || !self.image.is_well_formed() {
            return;
        }
        let origin = self.image.bounds_at(self.position);
        if canvas.visible(origin).is_none() {
            return;
        }
        let fill = theme.color(Role::BaseContent);
        let outline = theme.color(Role::Base100);
        paint_mask(self.image, origin, fill, outline, canvas);
    }
}

fn paint_mask(
    image: &CursorImage,
    origin: Rect,
    fill: Color,
    outline: Color,
    canvas: &mut Pen<'_>,
) {
    for row in 0..image.height {
        let base = (row * image.width) as usize;
        let y = origin.y + row;
        // Runs of one value blit as a span, which matters because the per-pixel
        // path measured fifteen times slower than the span path on a Pi 3.
        let mut x = 0;
        while x < image.width {
            let value = image.mask[base + x as usize];
            let mut end = x + 1;
            while end < image.width && image.mask[base + end as usize] == value {
                end += 1;
            }
            let color = match value {
                b'#' => Some(fill),
                b'+' => Some(outline),
                _ => None,
            };
            if let Some(color) = color {
                canvas.fill_rect(Rect::new(origin.x + x, y, end - x, 1), color);
            }
            x = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use denise_render::Canvas;
    /// The plane and the software composite must agree, or switching a panel to
    /// the hardware cursor would also change how it looks.
    #[test]
    fn the_rasterised_sprite_uses_the_same_two_theme_colours() {
        let theme = denise::theme::DARK;
        let mut pixels = vec![0xDEAD_BEEFu32; (ARROW.width * ARROW.height) as usize];
        let written = ARROW.rasterise(&theme, &mut pixels);
        assert_eq!(written, pixels.len());

        let fill = theme.color(Role::BaseContent).to_argb8888();
        let outline = theme.color(Role::Base100).to_argb8888();
        for (pixel, &value) in pixels.iter().zip(ARROW.mask) {
            match value {
                b'#' => assert_eq!(*pixel, fill),
                b'+' => assert_eq!(*pixel, outline),
                _ => assert_eq!(*pixel, 0, "transparent must be a zero word, not black"),
            }
        }
    }

    /// A cursor plane composites during scanout, so any pixel that is not the
    /// pointer has to be fully transparent — an opaque background would paint a
    /// rectangle over whatever is under it.
    #[test]
    fn every_transparent_pixel_has_zero_alpha() {
        for image in [&ARROW, &CROSSHAIR] {
            let mut pixels = vec![0u32; (image.width * image.height) as usize];
            image.rasterise(&denise::theme::LIGHT, &mut pixels);
            let transparent = pixels.iter().filter(|p| **p >> 24 == 0).count();
            let expected = image.mask.iter().filter(|b| **b == b'.').count();
            assert_eq!(transparent, expected);
            assert!(
                transparent > 0,
                "a cursor with no transparency is a rectangle"
            );
        }
    }

    /// The theme is baked in, so a theme switch has to re-upload. If both themes
    /// produced the same pixels this would be silently fine and the test would be
    /// worthless — so check they actually differ.
    #[test]
    fn a_theme_change_changes_the_pixels() {
        let mut dark = vec![0u32; (ARROW.width * ARROW.height) as usize];
        let mut light = dark.clone();
        ARROW.rasterise(&denise::theme::DARK, &mut dark);
        ARROW.rasterise(&denise::theme::LIGHT, &mut light);
        assert_ne!(
            dark, light,
            "the plane must be re-uploaded on a theme change"
        );
    }

    #[test]
    fn a_buffer_too_small_writes_nothing() {
        let mut pixels = vec![0u32; 4];
        assert_eq!(ARROW.rasterise(&denise::theme::DARK, &mut pixels), 0);
        assert!(pixels.iter().all(|&p| p == 0), "nothing partial is written");
    }

    use super::*;
    use denise::{PixelFormat, Size, theme};

    #[test]
    fn built_in_sprites_match_their_declared_geometry() {
        assert!(ARROW.is_well_formed(), "arrow mask is the wrong length");
        assert!(
            CROSSHAIR.is_well_formed(),
            "crosshair mask is the wrong length"
        );
    }

    #[test]
    fn the_hotspot_pixel_is_opaque() {
        for image in [&ARROW, &CROSSHAIR] {
            let i = (image.hotspot.y * image.width + image.hotspot.x) as usize;
            assert_ne!(
                image.mask[i], b'.',
                "the pixel under the pointer position must be drawn"
            );
        }
    }

    #[test]
    fn a_hidden_cursor_paints_nothing() {
        let mut pixels = [0u32; 64 * 64];
        let mut canvas =
            Canvas::from_pixels(&mut pixels, Size::new(64, 64), 64, PixelFormat::Xrgb8888)
                .expect("canvas");
        let cursor = Cursor::default();
        cursor.paint(&theme::DARK, &mut canvas.pen());
        assert!(pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn the_sprite_stays_inside_its_own_bounds() {
        let mut pixels = [0u32; 64 * 64];
        let cursor = Cursor {
            image: &ARROW,
            position: Point::new(20, 20),
            visible: true,
        };
        {
            let mut canvas =
                Canvas::from_pixels(&mut pixels, Size::new(64, 64), 64, PixelFormat::Xrgb8888)
                    .expect("canvas");
            cursor.paint(&theme::DARK, &mut canvas.pen());
        }
        let bounds = cursor.bounds();
        for y in 0..64i32 {
            for x in 0..64i32 {
                if !bounds.contains(Point::new(x, y)) {
                    assert_eq!(pixels[(y * 64 + x) as usize], 0, "wrote outside at {x},{y}");
                }
            }
        }
        assert_ne!(
            pixels[(20 * 64 + 20) as usize],
            0,
            "the tip should be drawn"
        );
    }
}
