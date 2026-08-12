//! The hardware cursor plane.
//!
//! vc4 has one, and it is the difference between a pointer that costs a full
//! repaint and page flip on every report and one that costs a single ioctl.
//!
//! # Why the deprecated ioctls
//!
//! The `drm` crate marks `set_cursor` and `move_cursor` deprecated in favour of
//! setting a `DRM_PLANE_TYPE_CURSOR` plane through an atomic commit. This backend
//! is legacy by design — see the README on why M2 shipped legacy modesetting —
//! and these are exactly the "legacy equivalent" that decision counted on. An
//! atomic path can be added behind the same trait when planes earn it; until then
//! the deprecation is about the API this backend deliberately does not use.

use denise::{CursorPlane, Point, Size, SurfaceError, cursor::pad_into};
use drm::control::{Device as ControlDevice, dumbbuffer::DumbBuffer};
use drm_fourcc::DrmFourcc;

use crate::device::Card;
use crate::error::DrmError;
use crate::surface::DrmSurface;

/// Bits per pixel. `ARGB8888`, unlike the scanout buffer's `XRGB8888`: a cursor
/// without alpha is a rectangle.
const BPP: u32 = 32;

/// What to assume when the driver will not say how large a cursor it takes.
/// Every driver that has a cursor plane at all takes at least this.
const FALLBACK: Size = Size::new(64, 64);

/// The plane's buffer: a fixed-size ARGB allocation the display controller
/// composites during scanout.
#[derive(Debug)]
pub(crate) struct CursorBuffer {
    dumb: DumbBuffer,
    /// Start of the mapping, as `u32` words.
    ptr: *mut u32,
    /// Length of the mapping in words.
    words: usize,
    /// Length of the mapping in bytes, for `munmap`.
    bytes: usize,
    /// The hardware's fixed buffer geometry, not the sprite's.
    limit: Size,
    /// The pixel of the sprite that sits on the pointer position.
    hotspot: Point,
}

impl CursorBuffer {
    /// Allocates the plane's buffer at the size the driver reports.
    pub(crate) fn new(card: &Card) -> Result<Self, DrmError> {
        let limit = limit_of(card);

        let mut dumb = card
            .create_dumb_buffer((limit.width, limit.height), DrmFourcc::Argb8888, BPP)
            .map_err(|source| DrmError::Allocate {
                width: limit.width,
                height: limit.height,
                source,
            })?;

        // As in `Scanout::new`: the mapping has to outlive the call that made it,
        // so the guard is forgotten and this code owns the `munmap`.
        let (ptr, bytes) = {
            let mut mapping = card.map_dumb_buffer(&mut dumb).map_err(DrmError::Map)?;
            let slice: &mut [u8] = &mut mapping;
            let ptr = slice.as_mut_ptr();
            let bytes = slice.len();
            core::mem::forget(mapping);
            (ptr, bytes)
        };

        let mut buffer = Self {
            dumb,
            // SAFETY: `mmap` returns page-aligned memory, which satisfies `u32`
            // alignment. The cast does not change the region's extent; `words`
            // accounts for the narrower element type.
            ptr: ptr.cast::<u32>(),
            words: bytes / 4,
            bytes,
            limit,
            hotspot: Point::ZERO,
        };

        // A fresh dumb buffer holds whatever was in that memory, and this one is
        // composited with alpha. Without clearing it the first `set_cursor` shows
        // a square of stale kernel memory over the display.
        buffer.pixels_mut().fill(0);
        Ok(buffer)
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        // SAFETY: `ptr` and `words` come from a single successful mapping of this
        // buffer, which stays mapped until `release` unmaps it. `&mut self` rules
        // out any other live reference to the region.
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.words) }
    }

    /// Writes a sprite into the buffer, padded and transparent everywhere else.
    fn upload(&mut self, pixels: &[u32], size: Size, hotspot: Point) -> Result<(), SurfaceError> {
        let limit = self.limit;
        pad_into(pixels, size, self.pixels_mut(), limit)?;
        self.hotspot = hotspot;
        Ok(())
    }

    /// Unmaps and destroys. Called from `DrmSurface::drop`.
    pub(crate) fn release(self, card: &Card) {
        // SAFETY: `ptr`/`bytes` describe exactly the mapping made in `new`, whose
        // guard was forgotten so this code owns it. `self` is taken by value, so
        // nothing else can reference the region.
        unsafe {
            let _ = rustix::mm::munmap(self.ptr.cast::<core::ffi::c_void>(), self.bytes);
        }
        let _ = card.destroy_dumb_buffer(self.dumb);
    }
}

/// The largest cursor the driver accepts, or [`FALLBACK`] if it will not say.
fn limit_of(card: &Card) -> Size {
    use drm::Device as _;
    let width = card
        .get_driver_capability(drm::DriverCapability::CursorWidth)
        .ok()
        .filter(|w| *w > 0)
        .unwrap_or(FALLBACK.width as u64);
    let height = card
        .get_driver_capability(drm::DriverCapability::CursorHeight)
        .ok()
        .filter(|h| *h > 0)
        .unwrap_or(FALLBACK.height as u64);
    Size::new(width as u32, height as u32)
}

impl CursorPlane for DrmSurface {
    fn cursor_limit(&self) -> Size {
        match self.cursor.as_ref() {
            Some(cursor) => cursor.limit,
            // Nothing is allocated until the first `set_cursor`, so answer from
            // the driver rather than allocating just to be asked.
            None => limit_of(&self.card),
        }
    }

    fn set_cursor(
        &mut self,
        pixels: &[u32],
        size: Size,
        hotspot: Point,
    ) -> Result<(), SurfaceError> {
        if self.cursor.is_none() {
            self.cursor = Some(CursorBuffer::new(&self.card).map_err(SurfaceError::backend)?);
        }
        let cursor = self.cursor.as_mut().expect("just created");
        cursor.upload(pixels, size, hotspot)?;

        // The buffer handle goes straight to the CRTC; a cursor needs no
        // framebuffer object, unlike a scanout buffer.
        #[allow(deprecated)]
        self.card
            .set_cursor(self.crtc, Some(&cursor.dumb))
            .map_err(SurfaceError::backend)
    }

    fn move_cursor(&mut self, position: Point) -> Result<(), SurfaceError> {
        let Some(cursor) = self.cursor.as_ref() else {
            // Moving a cursor that was never set is a no-op rather than an error:
            // an application that has not chosen a sprite has not asked for one.
            return Ok(());
        };
        // `drmModeMoveCursor` positions the buffer's top-left corner, so the
        // hotspot offset is applied here. `set_cursor2` can carry a hotspot
        // instead, but real hardware ignores it for positioning — it exists for
        // virtualised drivers to tell the host — and a driver that honoured both
        // would offset twice.
        let at = (position.x - cursor.hotspot.x, position.y - cursor.hotspot.y);
        #[allow(deprecated)]
        self.card
            .move_cursor(self.crtc, at)
            .map_err(SurfaceError::backend)
    }

    fn hide_cursor(&mut self) -> Result<(), SurfaceError> {
        if self.cursor.is_none() {
            return Ok(());
        }
        // The allocation stays: showing it again should be an ioctl, not a
        // reallocation and a re-upload.
        #[allow(deprecated)]
        self.card
            .set_cursor(self.crtc, None::<&DumbBuffer>)
            .map_err(SurfaceError::backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every driver with a cursor plane takes at least 64×64, and a zero from a
    /// driver that answers the capability query wrongly would allocate nothing and
    /// then reject every sprite.
    #[test]
    fn the_fallback_is_the_size_every_driver_supports() {
        assert_eq!(FALLBACK, Size::new(64, 64));
        assert!(!FALLBACK.is_empty());
    }

    /// The scanout buffer is XRGB and ignores the high byte; the cursor is
    /// composited over live pixels and must not.
    #[test]
    fn the_cursor_format_carries_alpha() {
        assert_eq!(BPP, 32);
        assert_eq!(DrmFourcc::Argb8888 as u32, DrmFourcc::Argb8888 as u32);
        assert_ne!(DrmFourcc::Argb8888, DrmFourcc::Xrgb8888);
    }
}
