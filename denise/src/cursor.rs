//! The hardware cursor plane, for backends that have one.
//!
//! A pointer sprite composited into the scanout buffer costs a repaint and a
//! present every time it moves. On a double-buffered display that is a full page
//! flip per pointer report, and the sprite appears at the *next* vblank — up to a
//! frame after the hand moved.
//!
//! A hardware cursor plane is a separate overlay the display controller composites
//! during scanout. Moving it is one ioctl setting a position: no repaint, no
//! flip, and the new position takes effect at the next scanout of those lines
//! rather than the next frame. vc4 has one, which is why this trait exists.
//!
//! # Using it
//!
//! The tree and the plane must not both draw a pointer, so tell the tree to stop:
//!
//! ```no_run
//! # use denise::{CursorPlane, Point, Size};
//! # fn demo(plane: &mut impl CursorPlane, sprite: &[u32]) -> Result<(), denise::SurfaceError> {
//! // ui.show_cursor(false) — a decision that sticks, so no later pointer motion
//! // brings the software sprite back.
//! plane.set_cursor(sprite, Size::new(16, 24), Point::new(1, 1))?;
//! plane.move_cursor(Point::new(400, 300))?;
//! # Ok(())
//! # }
//! ```
//!
//! Then call [`move_cursor`](CursorPlane::move_cursor) whenever the pointer moves.
//! It is cheap enough to call on every input event and does not need a frame.
//!
//! # What it is not
//!
//! Not a fallback for backends without one. A window system already draws a
//! pointer, and a bare framebuffer has no plane to draw it on, so neither
//! implements this; the composited sprite in `denise-ui` remains the answer
//! everywhere else.

use crate::geom::{Point, Size};
use crate::surface::SurfaceError;

/// A display that can overlay a pointer sprite during scanout.
///
/// Implemented by backends that own a real display controller. The positions are
/// physical pixels in the same space as [`InputEvent`](crate::InputEvent), so
/// nothing between input and this call has to convert anything.
pub trait CursorPlane {
    /// The largest sprite the hardware will accept.
    ///
    /// Cursor planes are fixed-size — 64×64 is near universal, and the buffer is
    /// that size whatever the sprite is. A sprite larger than this is refused
    /// rather than cropped, because a silently cropped pointer looks like a
    /// rendering bug rather than a limit.
    fn cursor_limit(&self) -> Size;

    /// Uploads a sprite and shows it.
    ///
    /// `pixels` is `size.width * size.height` words of `0xAARRGGBB` in native
    /// endianness, and **alpha is honoured** — unlike the scanout buffer, where
    /// the high byte is ignored. A cursor with no transparent pixels is a
    /// rectangle.
    ///
    /// `hotspot` is the pixel that sits on the pointer position, so a caller never
    /// does that arithmetic itself; [`move_cursor`](Self::move_cursor) takes the
    /// pointer position directly.
    fn set_cursor(
        &mut self,
        pixels: &[u32],
        size: Size,
        hotspot: Point,
    ) -> Result<(), SurfaceError>;

    /// Moves the sprite so its hotspot lands on `position`.
    ///
    /// The cheap call, and the whole point: no repaint, no present, no frame.
    /// Positions partly or wholly off-screen are the driver's business to clamp.
    fn move_cursor(&mut self, position: Point) -> Result<(), SurfaceError>;

    /// Removes the sprite from the display.
    ///
    /// The sprite stays uploaded, so showing it again is a
    /// [`set_cursor`](Self::set_cursor) and not a reallocation.
    fn hide_cursor(&mut self) -> Result<(), SurfaceError>;
}

/// Fits `sprite` into a `limit`-sized buffer of transparent pixels, top-left.
///
/// Cursor planes take a fixed-size buffer whatever the sprite is, so every
/// implementation needs this and none of it is platform-specific. Returns the
/// number of words written, which is always `limit.width * limit.height`.
///
/// Fails with [`SurfaceError::CursorTooLarge`] rather than cropping: a pointer
/// missing its bottom half looks like a rendering bug, and this is a limit.
pub fn pad_into(
    sprite: &[u32],
    size: Size,
    out: &mut [u32],
    limit: Size,
) -> Result<usize, SurfaceError> {
    if size.width > limit.width || size.height > limit.height {
        return Err(SurfaceError::CursorTooLarge {
            limit,
            requested: size,
        });
    }
    let needed = (size.width as usize) * (size.height as usize);
    if sprite.len() < needed {
        return Err(SurfaceError::BufferTooSmall {
            required: needed,
            actual: sprite.len(),
        });
    }
    let capacity = (limit.width as usize) * (limit.height as usize);
    if out.len() < capacity {
        return Err(SurfaceError::BufferTooSmall {
            required: capacity,
            actual: out.len(),
        });
    }

    // Fully transparent, not black: the buffer is larger than the sprite in both
    // directions and every pixel outside it has to disappear rather than paint.
    out[..capacity].fill(0);
    for row in 0..size.height as usize {
        let from = row * size.width as usize;
        let to = row * limit.width as usize;
        out[to..to + size.width as usize]
            .copy_from_slice(&sprite[from..from + size.width as usize]);
    }
    Ok(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const LIMIT: Size = Size::new(4, 4);

    #[test]
    fn a_sprite_lands_top_left_and_the_rest_is_transparent() {
        let sprite = [0xFF00_0001, 0xFF00_0002, 0xFF00_0003, 0xFF00_0004];
        let mut out = vec![0xDEAD_BEEF; 16];
        let written = pad_into(&sprite, Size::new(2, 2), &mut out, LIMIT).expect("pad");
        assert_eq!(written, 16);

        // Row 0 of the sprite, then padding.
        assert_eq!(&out[0..4], &[0xFF00_0001, 0xFF00_0002, 0, 0]);
        // Row 1 starts a full `limit.width` later, not `size.width`.
        assert_eq!(&out[4..8], &[0xFF00_0003, 0xFF00_0004, 0, 0]);
        assert!(
            out[8..].iter().all(|&p| p == 0),
            "padding must be transparent"
        );
    }

    /// Zero alpha, not black. A cursor plane composites during scanout, so an
    /// opaque black pad would draw a 64×64 square around the pointer.
    #[test]
    fn padding_is_transparent_rather_than_black() {
        let sprite = [0xFFFF_FFFF];
        let mut out = vec![0u32; 16];
        pad_into(&sprite, Size::new(1, 1), &mut out, LIMIT).expect("pad");
        assert_eq!(out[0] >> 24, 0xFF, "the sprite keeps its alpha");
        assert!(
            out[1..].iter().all(|&p| p >> 24 == 0),
            "every padded pixel must be fully transparent"
        );
    }

    #[test]
    fn a_sprite_larger_than_the_plane_is_refused_rather_than_cropped() {
        let sprite = vec![0u32; 25];
        let mut out = vec![0u32; 16];
        let result = pad_into(&sprite, Size::new(5, 5), &mut out, LIMIT);
        assert!(matches!(result, Err(SurfaceError::CursorTooLarge { .. })));

        // One pixel over in a single dimension is still over.
        let sprite = vec![0u32; 20];
        assert!(pad_into(&sprite, Size::new(5, 4), &mut out, LIMIT).is_err());
        assert!(pad_into(&sprite, Size::new(4, 5), &mut out, LIMIT).is_err());
    }

    #[test]
    fn a_sprite_that_lies_about_its_size_is_refused() {
        let sprite = [0u32; 3];
        let mut out = vec![0u32; 16];
        assert!(matches!(
            pad_into(&sprite, Size::new(2, 2), &mut out, LIMIT),
            Err(SurfaceError::BufferTooSmall {
                required: 4,
                actual: 3
            })
        ));
    }

    #[test]
    fn exactly_filling_the_plane_is_allowed() {
        let sprite = vec![0xFF12_3456u32; 16];
        let mut out = vec![0u32; 16];
        pad_into(&sprite, LIMIT, &mut out, LIMIT).expect("a sprite may fill the plane");
        assert!(out.iter().all(|&p| p == 0xFF12_3456));
    }
}
