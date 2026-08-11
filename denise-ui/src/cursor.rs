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
//! On DRM this should eventually become the hardware cursor plane, which vc4 has
//! and which makes pointer movement cost no redraw at all. The software composite
//! then stays as the fallback for backends without one.

use denise::{Color, Point, Rect, Role, Theme};
use denise_render::Canvas;

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
    pub fn paint(&self, theme: &Theme, canvas: &mut Canvas<'_>) {
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
    canvas: &mut Canvas<'_>,
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
        cursor.paint(&theme::DARK, &mut canvas);
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
            cursor.paint(&theme::DARK, &mut canvas);
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
