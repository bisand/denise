//! A [`Surface`] over a GDI DIB section.
//!
//! A DIB section is the one Windows bitmap you get a pointer into. `BitBlt` from
//! a memory DC holding it is the fastest path from CPU pixels to a window that
//! does not involve Direct2D or a swapchain, and it is the path a control
//! embedded in somebody else's MFC application can actually take.

use core::ffi::c_void;
use core::ptr::NonNull;

use denise::{BufferAge, Frame, PixelFormat, Rect, Size, Surface, SurfaceError};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SRCCOPY, SelectObject,
};

use crate::Error;

/// A pixel buffer a window blits from.
///
/// Persistent and always current, so [`BufferAge::Frames(1)`] is honest on every
/// frame and incremental repaint works. Nothing here is double-buffered: `BitBlt`
/// from a DIB is itself the present, and the window never sees a half-drawn one
/// because GDI serialises it.
pub struct DibSurface {
    /// Memory DC with `bitmap` selected into it. The blit source.
    dc: HDC,
    bitmap: HBITMAP,
    /// What was in `dc` before, to put back before deleting it. GDI leaks the DC
    /// otherwise, and a control in a long-running host leaks one per resize.
    previous: HGDIOBJ,
    pixels: NonNull<u32>,
    size: Size,
    scale_factor: f32,
}

impl DibSurface {
    /// Allocates a surface `size` physical pixels across.
    ///
    /// `scale_factor` is the window's DPI over 96 — 1.5 at 144 DPI — and is
    /// reported to the application rather than applied here. Denise lays out in
    /// physical pixels, so a high-DPI window asks for more of them.
    pub fn new(size: Size, scale_factor: f32) -> Result<Self, Error> {
        if size.is_empty() {
            return Err(Error::EmptySurface);
        }

        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size.width as i32;
        // Negative height means a top-down DIB: row 0 is the top row, which is
        // what Denise means by row 0. A bottom-up DIB is the default, and using
        // one renders the whole panel upside down without failing anywhere.
        info.bmiHeader.biHeight = -(size.height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut c_void = core::ptr::null_mut();
        // SAFETY: `info` describes a 32-bit BI_RGB DIB, `bits` receives the
        // allocation, and passing a null DC is valid for DIB_RGB_COLORS — the
        // colour table is unused at 32 bits per pixel.
        let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(|_| Error::DibSection)?;

        let Some(pixels) = NonNull::new(bits.cast::<u32>()) else {
            // SAFETY: `bitmap` was just created and nothing else refers to it.
            unsafe {
                let _ = DeleteObject(bitmap.into());
            }
            return Err(Error::DibSection);
        };

        // SAFETY: a null argument asks for a memory DC compatible with the
        // screen, which is what a blit source needs.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            // SAFETY: as above.
            unsafe {
                let _ = DeleteObject(bitmap.into());
            }
            return Err(Error::MemoryDc);
        }
        // SAFETY: both handles are live and a bitmap is a valid object to select
        // into a memory DC.
        let previous = unsafe { SelectObject(dc, bitmap.into()) };

        Ok(Self {
            dc,
            bitmap,
            previous,
            pixels,
            size,
            scale_factor,
        })
    }

    /// Reallocates for a new size or DPI, discarding the contents.
    ///
    /// Returns `true` if anything changed, in which case the caller owes a full
    /// repaint. Nothing here can produce one: damage belongs to the tree.
    pub fn resize(&mut self, size: Size, scale_factor: f32) -> Result<bool, Error> {
        if size == self.size && scale_factor == self.scale_factor {
            return Ok(false);
        }
        *self = Self::new(size, scale_factor)?;
        Ok(true)
    }

    /// Words per row.
    ///
    /// Equal to the width, and only for 32 bits per pixel: GDI aligns DIB rows to
    /// four bytes, which a 32-bit row already is. At 24 bits it would not be, and
    /// this would have to be computed rather than stated.
    #[inline]
    pub const fn stride(&self) -> u32 {
        self.size.width
    }

    /// Copies `rects` from the surface into `target`.
    ///
    /// This is where damage stops being an optimisation and becomes bandwidth:
    /// `BitBlt` genuinely moves only what it is asked for, unlike a DRM page flip
    /// where the whole buffer goes regardless of what changed.
    ///
    /// # Safety
    ///
    /// `target` must be a live device context — the one from `BeginPaint`, or a
    /// window DC.
    pub unsafe fn blit(&self, target: HDC, rects: &[Rect]) {
        for rect in rects {
            let Some(clipped) = rect.intersect(&Rect::from_size(self.size)) else {
                continue;
            };
            // SAFETY: the caller promises `target` is live; `self.dc` holds the
            // DIB; and `clipped` is inside the surface by construction, so the
            // source rectangle is in range.
            let ok = unsafe {
                windows::Win32::Graphics::Gdi::BitBlt(
                    target,
                    clipped.x,
                    clipped.y,
                    clipped.width,
                    clipped.height,
                    Some(self.dc),
                    clipped.x,
                    clipped.y,
                    SRCCOPY,
                )
            };
            // A failed blit leaves stale pixels on screen and there is nothing
            // useful to do about it here; the next full repaint corrects it.
            let _ = ok;
        }
    }

    /// The memory DC holding the pixels, for a caller that wants to draw over
    /// Denise's output with GDI itself.
    #[inline]
    pub const fn dc(&self) -> HDC {
        self.dc
    }
}

impl Drop for DibSurface {
    fn drop(&mut self) {
        // SAFETY: every handle here was created by `new` and is dropped once.
        // The previous object goes back into the DC first, because a DC still
        // holding our bitmap will not release it.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(self.bitmap.into());
        }
    }
}

impl Surface for DibSurface {
    fn size(&self) -> Size {
        self.size
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn format(&self) -> PixelFormat {
        // A 32-bit BI_RGB DIB is `0xAARRGGBB` in a little-endian DWORD, with the
        // high byte ignored by GDI. That is exactly `Xrgb8888`, which is why this
        // backend needs no conversion pass at all.
        PixelFormat::Xrgb8888
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        let len = self.stride() as usize * self.size.height as usize;
        // SAFETY: `pixels` is the DIB section's allocation, which lives as long
        // as `bitmap` and therefore as long as `self`, and is exactly
        // `width * height` words for a top-down 32-bit DIB. `Frame` borrows
        // `self` mutably, so nothing else can reach it meanwhile.
        let pixels = unsafe { core::slice::from_raw_parts_mut(self.pixels.as_ptr(), len) };
        Frame::new(
            pixels,
            self.size,
            self.stride(),
            PixelFormat::Xrgb8888,
            // Ours, persistent, never handed to anyone else.
            BufferAge::Frames(1),
        )
    }

    fn present(&mut self, _damage: &[Rect]) -> Result<(), SurfaceError> {
        // Nothing here: the control invalidates the damaged rectangles and
        // Windows decides when to ask for them back. Blitting now would fight
        // WM_PAINT rather than serve it.
        Ok(())
    }
}

/// Screen-to-client conversion for the messages that need it.
///
/// Almost every mouse message carries client coordinates in `lParam`. The wheel
/// messages carry *screen* coordinates instead, which is a documented
/// inconsistency and the source of a scroll that works only when the window is at
/// the top-left of the display.
pub(crate) fn screen_to_client(hwnd: HWND, x: i32, y: i32) -> (i32, i32) {
    let mut point = windows::Win32::Foundation::POINT { x, y };
    // SAFETY: `hwnd` is the control's own window and `point` is a live local.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    (point.x, point.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_surface_is_refused_rather_than_allocated() {
        assert!(DibSurface::new(Size::new(0, 40), 1.0).is_err());
        assert!(DibSurface::new(Size::new(40, 0), 1.0).is_err());
    }

    #[test]
    fn a_thirty_two_bit_dib_needs_no_padding() {
        let surface = DibSurface::new(Size::new(101, 7), 1.0).expect("surface");
        // 101 pixels is 404 bytes, already a multiple of four. The odd width is
        // the point: at 24 bits per pixel this would need three bytes of padding.
        assert_eq!(surface.stride(), 101);
        assert_eq!(surface.format(), PixelFormat::Xrgb8888);
    }

    #[test]
    fn the_surface_is_addressable_to_its_last_pixel() {
        let mut surface = DibSurface::new(Size::new(16, 9), 1.0).expect("surface");
        let mut frame = surface.acquire().expect("frame");
        let last = frame.row_mut(8).expect("last row");
        assert_eq!(last.len(), 16);
        last[15] = 0x00FF_00FF;
        assert_eq!(frame.row_mut(8).expect("last row")[15], 0x00FF_00FF);
    }
}
