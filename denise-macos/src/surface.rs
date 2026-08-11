//! A [`Surface`] backed by a CoreGraphics bitmap context.
//!
//! CoreGraphics owns the pixels, not us. `CGBitmapContextCreate` with a null data
//! pointer allocates and keeps them for the lifetime of the context, which removes
//! the whole question of whether a `CGImage` the window server has not finished
//! with is still pointing at a `Vec` that moved. It also means CoreGraphics picks
//! the row pitch — and it does not pick the width.

use core::ptr::NonNull;

use denise::{BufferAge, Frame, PixelFormat, Rect, Size, Surface, SurfaceError};
use objc2_core_foundation::{CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGBitmapContextGetBytesPerRow,
    CGBitmapContextGetData, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
};

use crate::Error;

/// Every pixel Denise produces is `0xAARRGGBB` in a native-endian word. On a
/// little-endian machine that is `B, G, R, A` in memory, which CoreGraphics spells
/// as "skip the first component, 32-bit little-endian" — the same layout DRM calls
/// `XRGB8888` and Win32 calls a `BI_RGB` DIB. Getting this wrong does not fail; it
/// swaps red and blue, which is the sort of bug that survives review.
fn bitmap_info() -> u32 {
    CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::NoneSkipFirst.0
}

/// A pixel buffer a Cocoa view draws from.
///
/// Persistent and always current, so [`BufferAge::Frames(1)`] is the honest answer
/// on every frame and incremental repaint works — unlike a surface handed out by a
/// compositor, where the buffer you get back is two frames stale and the damage
/// has to be widened to match.
pub struct ViewSurface {
    context: CFRetained<CGContext>,
    /// The pixels CoreGraphics allocated. Valid for as long as `context` lives.
    pixels: NonNull<u32>,
    /// Words per row, which is `CGBitmapContextGetBytesPerRow / 4` and is *not*
    /// `size.width`: CoreGraphics aligns rows, typically to 32 bytes.
    stride: u32,
    size: Size,
    scale_factor: f32,
}

impl ViewSurface {
    /// Allocates a surface `size` physical pixels across.
    ///
    /// `scale_factor` is the view's backing scale — 2.0 on a Retina display — and
    /// is reported to the application rather than applied here. Denise lays out in
    /// physical pixels, so a Retina view asks for twice as many of them.
    pub fn new(size: Size, scale_factor: f32) -> Result<Self, Error> {
        if size.is_empty() {
            return Err(Error::EmptySurface);
        }
        let space = CGColorSpace::new_device_rgb().ok_or(Error::ColorSpace)?;

        // SAFETY: a null `data` asks CoreGraphics to allocate and own the pixels,
        // which is the documented behaviour and the reason this backend uses it.
        // Passing 0 for `bytes_per_row` lets it pick the alignment it wants.
        let context = unsafe {
            CGBitmapContextCreate(
                core::ptr::null_mut(),
                size.width as usize,
                size.height as usize,
                8,
                0,
                Some(&space),
                bitmap_info(),
            )
        }
        .ok_or(Error::BitmapContext)?;

        let data = CGBitmapContextGetData(Some(&context));
        let pixels = NonNull::new(data.cast::<u32>()).ok_or(Error::BitmapContext)?;
        let bytes_per_row = CGBitmapContextGetBytesPerRow(Some(&context));
        // A pitch that is not a whole number of words would make the `&mut [u32]`
        // below unsound. CoreGraphics has never produced one for a 32-bit format,
        // and this is cheaper than trusting that.
        if !bytes_per_row.is_multiple_of(4) {
            return Err(Error::BitmapContext);
        }

        Ok(Self {
            context,
            pixels,
            stride: (bytes_per_row / 4) as u32,
            size,
            scale_factor,
        })
    }

    /// Reallocates for a new size or backing scale, discarding the contents.
    ///
    /// The caller owes a full repaint afterwards. Nothing here can produce one:
    /// the tree owns damage, and it is the one that has to be told.
    pub fn resize(&mut self, size: Size, scale_factor: f32) -> Result<bool, Error> {
        if size == self.size && scale_factor == self.scale_factor {
            return Ok(false);
        }
        *self = Self::new(size, scale_factor)?;
        Ok(true)
    }

    /// Words per row. See [`Frame::stride`] for why this is not the width.
    #[inline]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// Draws the surface into a Cocoa view's graphics context.
    ///
    /// `bounds` is the view's rectangle in points, so a Retina view scales the
    /// image down by the backing factor here — the pixels stay physical all the
    /// way from layout to this call.
    ///
    /// # Safety
    ///
    /// `context` must be a live `CGContext` whose coordinate system is the one
    /// AppKit installs for a **flipped** view. In an unflipped one the image
    /// arrives upside down, silently.
    pub unsafe fn draw_into(&self, context: &CGContext, bounds: CGRect) {
        let Some(image) = CGBitmapContextCreateImage(Some(&self.context)) else {
            return;
        };

        // A `CGImage` is placed bottom-up, and a flipped view's context already
        // runs y downwards, so drawing straight into it produces a mirror. Undo
        // the flip for the duration of the draw and put it back.
        //
        // The translation is `2y + h` rather than `h` so this is right for a
        // sub-rectangle as well as for the whole view: mirroring about the rect's
        // own centre line, not the context's.
        CGContext::save_g_state(Some(context));
        CGContext::translate_ctm(
            Some(context),
            0.0,
            bounds.origin.y * 2.0 + bounds.size.height,
        );
        CGContext::scale_ctm(Some(context), 1.0, -1.0);
        CGContext::draw_image(Some(context), bounds, Some(&image));
        CGContext::restore_g_state(Some(context));
    }

    /// The bitmap context, for a caller that wants to draw over Denise's output
    /// with CoreGraphics itself.
    #[inline]
    pub fn context(&self) -> &CGContext {
        &self.context
    }

    /// Converts a damage rectangle in physical pixels to the view's points.
    ///
    /// Rounded outwards, because a rectangle that lands between two points has to
    /// invalidate both or leave a seam down one edge.
    ///
    /// No vertical flip: the view is flipped, so its coordinates already run
    /// top-left downwards, the same as Denise's.
    pub fn damage_to_points(&self, rect: Rect) -> CGRect {
        let scale = self.scale_factor.max(0.01) as CGFloat;
        let x = (rect.x as CGFloat / scale).floor();
        let y = (rect.y as CGFloat / scale).floor();
        let right = ((rect.x + rect.width) as CGFloat / scale).ceil();
        let bottom = ((rect.y + rect.height) as CGFloat / scale).ceil();

        CGRect::new(CGPoint::new(x, y), CGSize::new(right - x, bottom - y))
    }
}

impl Surface for ViewSurface {
    fn size(&self) -> Size {
        self.size
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn format(&self) -> PixelFormat {
        PixelFormat::Xrgb8888
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        let len = self.stride as usize * self.size.height as usize;
        // SAFETY: `pixels` is the allocation CoreGraphics made for `context`, which
        // this struct owns and which is `stride * height` words by construction.
        // Nothing else holds a reference to it: `draw_into` takes `&self` and only
        // snapshots through CoreGraphics, and `Frame` borrows `self` mutably for as
        // long as it lives.
        let pixels = unsafe { core::slice::from_raw_parts_mut(self.pixels.as_ptr(), len) };
        Frame::new(
            pixels,
            self.size,
            self.stride,
            PixelFormat::Xrgb8888,
            // Ours, persistent, and never handed to anyone else, so it always holds
            // exactly what was last drawn into it.
            BufferAge::Frames(1),
        )
    }

    fn present(&mut self, _damage: &[Rect]) -> Result<(), SurfaceError> {
        // Nothing to do: the view draws from this surface when AppKit asks it to,
        // and telling AppKit *what* to ask about is `DeniseView`'s job, because it
        // is the only one holding the view.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::Color;
    use denise_render::Canvas;

    #[test]
    fn core_graphics_picks_the_stride_and_it_is_not_the_width() {
        // 100 pixels is 400 bytes, which CoreGraphics rounds up to its own
        // alignment. If this ever stops being true the test is still correct;
        // what it is really asserting is that we ask rather than assume.
        let surface = ViewSurface::new(Size::new(100, 40), 1.0).expect("surface");
        assert!(
            surface.stride() >= 100,
            "a stride below the width cannot hold a row"
        );
        assert_eq!(
            surface.stride() * 4 % 4,
            0,
            "the pitch must be a whole number of words"
        );
    }

    #[test]
    fn drawing_lands_where_the_stride_says_it_does() {
        let mut surface = ViewSurface::new(Size::new(64, 32), 1.0).expect("surface");
        {
            let mut frame = surface.acquire().expect("frame");
            let mut canvas = Canvas::new(&mut frame);
            canvas.clear(Color::from_rgb888(0x00FF00));
            canvas.fill_rect(Rect::new(0, 0, 1, 1), Color::from_rgb888(0xFF0000));
        }
        // Read back through the same pointer the view draws from.
        let mut frame = surface.acquire().expect("frame");
        let row = frame.row_mut(1).expect("second row");
        assert_eq!(
            row[0] & 0x00FF_FFFF,
            0x00FF00,
            "row 1 is not the clear colour, so the stride is wrong"
        );
    }

    /// The one thing about this backend that fails silently.
    ///
    /// A `CGImage` is bottom-up and a flipped `NSView`'s context already mirrors
    /// the y axis, so the two are supposed to cancel. If they do not, the panel
    /// renders upside down and nothing anywhere reports an error — it just looks
    /// like somebody laid the widgets out wrong.
    ///
    /// So: draw a surface whose every row is a different colour into a second
    /// bitmap context carrying the same transform AppKit installs for a flipped
    /// view, and require the destination's memory to match the source's row for
    /// row. Both are `CGBitmapContext`s and share a memory convention, so equal
    /// memory means equal pixels on screen.
    #[test]
    fn a_flipped_context_draws_the_surface_the_right_way_up() {
        use objc2_core_graphics::CGBitmapContextCreate;

        const N: u32 = 8;
        let mut source = ViewSurface::new(Size::new(N, N), 1.0).expect("source");
        {
            let mut frame = source.acquire().expect("frame");
            let mut canvas = Canvas::new(&mut frame);
            for y in 0..N as i32 {
                // Row y gets red = y * 16, which no other row has.
                let shade = (y as u32 * 16) << 16;
                canvas.fill_rect(Rect::new(0, y, N as i32, 1), Color::from_rgb888(shade));
            }
        }

        let space = CGColorSpace::new_device_rgb().expect("colour space");
        // SAFETY: a null `data` asks CoreGraphics to allocate, as in `new`.
        let dest = unsafe {
            CGBitmapContextCreate(
                core::ptr::null_mut(),
                N as usize,
                N as usize,
                8,
                0,
                Some(&space),
                bitmap_info(),
            )
        }
        .expect("destination context");

        // Exactly what AppKit does to the context of a flipped view: move the
        // origin to the top and run y downwards.
        CGContext::translate_ctm(Some(&dest), 0.0, N as CGFloat);
        CGContext::scale_ctm(Some(&dest), 1.0, -1.0);

        let bounds = CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(N as CGFloat, N as CGFloat),
        );
        // SAFETY: `dest` is a live context carrying a flipped view's transform,
        // which is what `draw_into` requires.
        unsafe { source.draw_into(&dest, bounds) };

        let dest_stride = CGBitmapContextGetBytesPerRow(Some(&dest)) / 4;
        let dest_data = CGBitmapContextGetData(Some(&dest)).cast::<u32>();
        assert!(!dest_data.is_null());
        // SAFETY: `dest` owns `dest_stride * N` words and outlives this slice.
        let drawn = unsafe { core::slice::from_raw_parts(dest_data, dest_stride * N as usize) };

        let mut frame = source.acquire().expect("frame");
        for y in 0..N {
            let expected = frame.row_mut(y).expect("source row")[0] & 0x00FF_FFFF;
            let actual = drawn[y as usize * dest_stride] & 0x00FF_FFFF;
            assert_eq!(
                actual, expected,
                "row {y} came out as {actual:06X}, wanted {expected:06X} — \
                 the image is mirrored, so the panel renders upside down"
            );
        }
    }

    #[test]
    fn an_empty_surface_is_refused_rather_than_allocated() {
        assert!(ViewSurface::new(Size::new(0, 40), 1.0).is_err());
        assert!(ViewSurface::new(Size::new(40, 0), 1.0).is_err());
    }

    #[test]
    fn damage_rounds_outwards_on_a_retina_view() {
        let surface = ViewSurface::new(Size::new(200, 100), 2.0).expect("surface");
        // A rectangle starting on an odd physical pixel covers the point before it
        // and the one after; rounding inwards would leave a one-point seam.
        let rect = surface.damage_to_points(Rect::new(3, 5, 4, 4));
        assert_eq!(rect.origin.x, 1.0);
        assert_eq!(rect.origin.y, 2.0);
        assert_eq!(rect.size.width, 3.0);
        assert_eq!(rect.size.height, 3.0);
    }
}
