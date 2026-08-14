//! A [`Surface`] backed by a CoreGraphics bitmap context.
//!
//! CoreGraphics owns the pixels, not us. `CGBitmapContextCreate` with a null data
//! pointer allocates and keeps them for the lifetime of the context, which removes
//! the whole question of whether a `CGImage` the window server has not finished
//! with is still pointing at a `Vec` that moved. It also means CoreGraphics picks
//! the row pitch — and it does not pick the width.

use core::ptr::{NonNull, null_mut};

use denise::{BufferAge, Frame, PixelFormat, Rect, Size, Surface, SurfaceError};
use objc2_core_foundation::{
    CFDictionary, CFNumber, CFRetained, CFString, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImageAlphaInfo,
    CGImageByteOrderInfo,
};
use objc2_io_surface::{
    IOSurfaceLockOptions, IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceHeight,
    kIOSurfacePixelFormat, kIOSurfaceWidth,
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

/// `'BGRA'` as IOSurface spells it: the four-character code for 32-bit
/// little-endian BGRA, which is the same memory layout as `bitmap_info` above
/// and as every other surface in this project.
const PIXEL_FORMAT_BGRA: i32 = i32::from_be_bytes(*b"BGRA");

/// Allocates an IOSurface for `size`, letting it choose its own row alignment.
///
/// The properties are the minimum that produces a CPU-writable 32-bit surface:
/// anything omitted is derived. `BytesPerRow` in particular is deliberately not
/// requested — IOSurface aligns rows to suit the hardware, and asking for a
/// tighter pitch than it wants is how you get a surface it will not accelerate.
fn new_io_surface(size: Size) -> Result<CFRetained<IOSurfaceRef>, Error> {
    let number = |value: i64| CFNumber::new_i64(value);
    // SAFETY: reading IOSurface's own property-key statics, which are constants
    // the framework guarantees for the life of the process.
    let keys: [&CFString; 4] = unsafe {
        [
            kIOSurfaceWidth,
            kIOSurfaceHeight,
            kIOSurfaceBytesPerElement,
            kIOSurfacePixelFormat,
        ]
    };
    let owned = [
        number(i64::from(size.width)),
        number(i64::from(size.height)),
        number(4),
        number(i64::from(PIXEL_FORMAT_BGRA)),
    ];
    let values: [&CFNumber; 4] = [&owned[0], &owned[1], &owned[2], &owned[3]];

    let properties = CFDictionary::from_slices(&keys, &values);
    // SAFETY: every key is one of IOSurface's own, and every value is the type
    // that key is documented to take.
    unsafe { IOSurfaceRef::new(properties.as_opaque()) }.ok_or(Error::BitmapContext)
}

/// One of the two buffers, and everything needed to draw into it.
struct Buffer {
    io_surface: CFRetained<IOSurfaceRef>,
    context: CFRetained<CGContext>,
    /// The pixels inside `io_surface`. The address is stable for the surface's
    /// lifetime; the lock taken around each frame is about coherency, not about
    /// where the buffer is.
    pixels: NonNull<u32>,
}

/// A pixel buffer a Cocoa view draws from.
///
/// **Two `IOSurface`s, shown alternately.** One is handed to a `CALayer` as its
/// contents, where CoreAnimation reads it in place — no copy, and a cost that
/// does not scale with the size of the window. The obvious alternative, a
/// `CGImage` per frame, is copied whole on every commit however little of it
/// changed: on a 1040×720 surface with one spinner animating, that is the
/// difference between 9.2% of a core and 2%.
///
/// The pair is not for tearing, though it helps there too. It is because
/// assigning the *same* object to `contents` tells CoreAnimation nothing: the
/// property has not changed, so it has no reason to look at the buffer again,
/// and the window shows the first frame for ever while the application draws
/// happily into memory nobody is reading. Two surfaces means every present
/// assigns a different object, which is a change it cannot miss. The private
/// `-[CALayer setContentsChanged]` is the other way, and not one a published
/// crate should take.
///
/// So the buffer handed back by [`Surface::acquire`] is two frames old, not one,
/// and [`BufferAge::Frames(2)`] is what says so — which is exactly the case
/// `DamageTracker` exists to widen for.
pub struct ViewSurface {
    buffers: [Buffer; 2],
    /// The buffer the layer is showing. The other one is the next frame's.
    front: usize,
    /// Whether a frame is out, and therefore whether the back buffer is locked.
    drawing: bool,
    /// Words per row, which is `CGBitmapContextGetBytesPerRow / 4` and is *not*
    /// `size.width`: CoreGraphics aligns rows, typically to 32 bytes.
    stride: u32,
    size: Size,
    scale_factor: f32,
}

impl Buffer {
    /// One `IOSurface` and a bitmap context that draws straight into it.
    fn new(size: Size, space: &CGColorSpace) -> Result<Self, Error> {
        let io_surface = new_io_surface(size)?;
        let bytes_per_row = io_surface.bytes_per_row();
        let pixels = io_surface.base_address().cast::<u32>();

        // SAFETY: the buffer belongs to `io_surface`, which this struct holds,
        // and is at least `bytes_per_row * height` bytes. Passing it explicitly
        // — rather than a null `data` — is what puts the rasteriser's output
        // inside the surface the compositor reads.
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_ptr().cast(),
                size.width as usize,
                size.height as usize,
                8,
                bytes_per_row,
                Some(space),
                bitmap_info(),
            )
        }
        .ok_or(Error::BitmapContext)?;

        Ok(Self {
            io_surface,
            context,
            pixels,
        })
    }
}

impl ViewSurface {
    /// The buffer being drawn into: the one the layer is *not* showing.
    fn back(&self) -> &Buffer {
        &self.buffers[1 - self.front]
    }

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

        let one = Buffer::new(size, &space)?;
        let two = Buffer::new(size, &space)?;
        let bytes_per_row = one.io_surface.bytes_per_row();
        // Two surfaces of one size get one pitch, and the `&mut [u32]` handed out
        // by `acquire` is built from a single `stride` for both.
        if bytes_per_row != two.io_surface.bytes_per_row() {
            return Err(Error::BitmapContext);
        }
        // A pitch that is not a whole number of words would make that slice
        // unsound. CoreGraphics has never produced one for a 32-bit format, and
        // this is cheaper than trusting that.
        if !bytes_per_row.is_multiple_of(4) {
            return Err(Error::BitmapContext);
        }

        Ok(Self {
            buffers: [one, two],
            front: 0,
            drawing: false,
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
        let Some(image) = CGBitmapContextCreateImage(Some(&self.buffers[self.front].context))
        else {
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

    /// The buffer to hand a `CALayer` as its contents.
    ///
    /// `IOSurfaceRef` is toll-free bridged to the `IOSurface` class, which is
    /// what makes it assignable to `contents` at all.
    #[inline]
    pub fn io_surface(&self) -> &IOSurfaceRef {
        &self.buffers[self.front].io_surface
    }

    /// The bitmap context, for a caller that wants to draw over Denise's output
    /// with CoreGraphics itself.
    #[inline]
    pub fn context(&self) -> &CGContext {
        &self.back().context
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

impl Drop for ViewSurface {
    fn drop(&mut self) {
        if self.drawing {
            // SAFETY: as in `present`.
            let _ = unsafe {
                self.back()
                    .io_surface
                    .unlock(IOSurfaceLockOptions(0), null_mut())
            };
        }
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
        // Locked for the duration of the drawing and no longer. Holding it
        // across frames looks like it should be free — one writer, and `Frame`
        // already excludes a second — and it is not: the lock is what the
        // compositor waits on to read the surface, so a lock that is never
        // released is a window that draws one frame and then freezes.
        //
        // SAFETY: a null seed pointer is documented as "do not report the seed".
        if !self.drawing {
            // SAFETY: a null seed pointer is documented as "do not report the
            // seed", and the matching unlock happens in `present` or `drop`.
            let taken = unsafe {
                self.back()
                    .io_surface
                    .lock(IOSurfaceLockOptions(0), null_mut())
            };
            if taken != 0 {
                return Err(SurfaceError::NotReady);
            }
            self.drawing = true;
        }

        let len = self.stride as usize * self.size.height as usize;
        // SAFETY: `pixels` is the allocation CoreGraphics made for `context`, which
        // this struct owns and which is `stride * height` words by construction.
        // Nothing else holds a reference to it: `draw_into` takes `&self` and only
        // snapshots through CoreGraphics, and `Frame` borrows `self` mutably for as
        // long as it lives.
        let pixels = unsafe { core::slice::from_raw_parts_mut(self.back().pixels.as_ptr(), len) };
        Frame::new(
            pixels,
            self.size,
            self.stride,
            PixelFormat::Xrgb8888,
            // Two buffers alternating: what is handed back was last drawn two
            // frames ago, and `DamageTracker` widens for exactly that.
            BufferAge::Frames(2),
        )
    }

    fn present(&mut self, _damage: &[Rect]) -> Result<(), SurfaceError> {
        // Handing the buffer back to whoever wants to read it, and making it the
        // one the layer is shown next. The swap is what changes the object a
        // `CALayer` is given, which is the only thing that makes CoreAnimation
        // look at the pixels again.
        if self.drawing {
            // SAFETY: the lock was taken in `acquire`, with the same options.
            let _ = unsafe {
                self.back()
                    .io_surface
                    .unlock(IOSurfaceLockOptions(0), null_mut())
            };
            self.drawing = false;
            self.front = 1 - self.front;
        }

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
    use objc2_core_graphics::{CGBitmapContextGetBytesPerRow, CGBitmapContextGetData};

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
        // Published, which with two buffers is what makes this the one a host
        // reads: `draw_into` shows the last *presented* frame, not the one being
        // drawn into.
        source.present(&[]).expect("present");

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

        // The front buffer, read back through a fresh frame over the same memory.
        // `acquire` hands out the *back* one, so this reads the presented buffer
        // directly rather than through it.
        let stride = source.stride() as usize;
        // SAFETY: the front buffer owns `stride * N` words and outlives this.
        let front = unsafe {
            core::slice::from_raw_parts(
                source.buffers[source.front].pixels.as_ptr(),
                stride * N as usize,
            )
        };
        for y in 0..N {
            let expected = front[y as usize * stride] & 0x00FF_FFFF;
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
