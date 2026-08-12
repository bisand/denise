//! The pixel buffer contract every backend implements.

use alloc::boxed::Box;

use crate::geom::{Rect, Size};

/// Layout of one `u32` in a [`Frame`]'s pixel slice.
///
/// The word is always `0xAARRGGBB` in *native* endianness. On little-endian targets
/// that is byte order `B, G, R, A`, which is what DRM's `ARGB8888`/`XRGB8888`
/// fourccs and Win32 `BI_RGB` DIB sections both mean. It is deliberately *not*
/// tiny-skia's `Pixmap` layout, which is `R, G, B, A` in byte order; anything
/// bridging the two owes a swizzle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// `0xAARRGGBB`. The alpha byte is meaningful.
    Argb8888,
    /// `0xXXRRGGBB`. The high byte is ignored by the scanout hardware.
    Xrgb8888,
}

impl PixelFormat {
    /// Bytes per pixel.
    #[inline]
    pub const fn bytes_per_pixel(self) -> usize {
        4
    }

    /// Returns `true` if the high byte is honoured on present.
    #[inline]
    pub const fn has_alpha(self) -> bool {
        matches!(self, PixelFormat::Argb8888)
    }
}

/// How stale the contents of the buffer just acquired are.
///
/// Modelled on `EGL_EXT_buffer_age`. With N-buffering the buffer handed back holds
/// the contents of frame `current - age`, so a correct incremental repaint must
/// cover the union of the last `age` frames' damage — not just this frame's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BufferAge {
    /// Contents are undefined. Repaint everything.
    ///
    /// Returned after a resize, on the first frame, and by backends that cannot
    /// track age (most compositor-mediated ones).
    Undefined,
    /// Contents are those of the frame `n` presents ago. `1` means the buffer we
    /// most recently presented, i.e. single buffering or a persistent shadow.
    Frames(u32),
}

/// A borrowed, writable pixel buffer for exactly one frame.
///
/// Obtained from [`Surface::acquire`] and released by dropping it, after which the
/// same surface must be told what changed via [`Surface::present`].
///
/// `stride` is in **pixels**, not bytes, and may exceed `size.width`: DRM
/// framebuffers are pitch-aligned (64 bytes on vc4, more elsewhere) and fbdev has
/// its own `line_length`. Never assume `stride == width` — index rows through
/// [`Frame::row_mut`] or [`Frame::rows_mut`].
#[derive(Debug)]
#[must_use = "a Frame must be drawn into and dropped before Surface::present"]
pub struct Frame<'a> {
    pixels: &'a mut [u32],
    size: Size,
    stride: u32,
    format: PixelFormat,
    age: BufferAge,
}

impl<'a> Frame<'a> {
    /// Wraps a backend buffer.
    ///
    /// Returns [`SurfaceError::BufferTooSmall`] unless `stride >= size.width` and
    /// `pixels` covers `stride * (height - 1) + width` words.
    pub fn new(
        pixels: &'a mut [u32],
        size: Size,
        stride: u32,
        format: PixelFormat,
        age: BufferAge,
    ) -> Result<Self, SurfaceError> {
        if size.is_empty() {
            return Err(SurfaceError::NotReady);
        }
        if stride < size.width {
            return Err(SurfaceError::BufferTooSmall {
                required: size.width as usize,
                actual: stride as usize,
            });
        }
        let required = stride as usize * (size.height as usize - 1) + size.width as usize;
        if pixels.len() < required {
            return Err(SurfaceError::BufferTooSmall {
                required,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            pixels,
            size,
            stride,
            format,
            age,
        })
    }

    /// Visible extent in physical pixels.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Distance between the starts of consecutive rows, in pixels.
    #[inline]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// Word layout of the buffer.
    #[inline]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// How stale these contents are. Feed to [`crate::DamageTracker::resolve`].
    #[inline]
    pub const fn age(&self) -> BufferAge {
        self.age
    }

    /// Full backing slice, including any inter-row padding.
    #[inline]
    pub fn pixels(&self) -> &[u32] {
        self.pixels
    }

    /// Full backing slice, including any inter-row padding.
    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        self.pixels
    }

    /// One row, trimmed to the visible width. Returns `None` past the bottom edge.
    #[inline]
    pub fn row(&self, y: u32) -> Option<&[u32]> {
        if y >= self.size.height {
            return None;
        }
        let start = y as usize * self.stride as usize;
        Some(&self.pixels[start..start + self.size.width as usize])
    }

    /// One row, trimmed to the visible width. Returns `None` past the bottom edge.
    #[inline]
    pub fn row_mut(&mut self, y: u32) -> Option<&mut [u32]> {
        if y >= self.size.height {
            return None;
        }
        let start = y as usize * self.stride as usize;
        Some(&mut self.pixels[start..start + self.size.width as usize])
    }

    /// Every visible row, top to bottom, each trimmed to the visible width.
    #[inline]
    pub fn rows_mut(&mut self) -> impl Iterator<Item = &mut [u32]> {
        let width = self.size.width as usize;
        let height = self.size.height as usize;
        self.pixels
            .chunks_mut(self.stride as usize)
            .take(height)
            .map(move |row| &mut row[..width])
    }
}

/// A backend that owns a presentable pixel buffer.
///
/// The contract is strictly alternating: [`acquire`](Surface::acquire), draw, drop
/// the frame, [`present`](Surface::present). Implementations should return
/// [`SurfaceError::FrameInFlight`] or [`SurfaceError::NoFrame`] rather than
/// silently tolerating a violation.
pub trait Surface {
    /// Visible extent in physical pixels.
    fn size(&self) -> Size;

    /// Physical pixels per logical pixel. `1.0` on a typical panel.
    fn scale_factor(&self) -> f32;

    /// Word layout the backend will scan out.
    fn format(&self) -> PixelFormat;

    /// Takes the next drawable buffer.
    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError>;

    /// Publishes the frame, telling the backend which regions changed.
    ///
    /// `damage` must be in physical pixels relative to the surface origin, and
    /// should already be clipped to [`size`](Surface::size). An empty slice means
    /// nothing changed; a backend may still be obliged to flip.
    ///
    /// How much this buys varies enormously. `BitBlt`, X11 and Wayland genuinely
    /// upload only the listed regions. A DRM page flip swaps whole buffers and will
    /// ignore the damage unless the driver honours `FB_DAMAGE_CLIPS`; there the win
    /// is upstream, in not rasterising the untouched pixels at all.
    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError>;
}

/// Failures from [`Surface`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SurfaceError {
    /// The surface has no valid size yet, or is not currently displayable.
    #[error("surface is not ready to render")]
    NotReady,

    /// [`Surface::acquire`] was called twice without an intervening present.
    #[error("a frame is already in flight; present it before acquiring another")]
    FrameInFlight,

    /// [`Surface::present`] was called without a preceding acquire.
    #[error("no frame has been acquired")]
    NoFrame,

    /// The backend handed over a buffer too small for the geometry it declared.
    #[error("buffer too small: need {required} pixels, got {actual}")]
    BufferTooSmall {
        /// Words the declared geometry requires.
        required: usize,
        /// Words actually available.
        actual: usize,
    },

    /// A cursor sprite larger than the plane can hold.
    ///
    /// Cursor planes are fixed-size, so this is a limit rather than a shortage.
    /// Refused rather than cropped: a pointer missing its lower half reads as a
    /// rendering bug, not as a hardware constraint.
    #[error(
        "cursor sprite is {}x{} but the plane holds at most {}x{}",
        requested.width, requested.height, limit.width, limit.height
    )]
    CursorTooLarge {
        /// The largest sprite the hardware accepts.
        limit: crate::geom::Size,
        /// The sprite that was offered.
        requested: crate::geom::Size,
    },

    /// A platform-specific failure.
    #[error("backend error: {0}")]
    Backend(Box<dyn core::error::Error + Send + Sync + 'static>),
}

impl SurfaceError {
    /// Wraps a platform error without leaking its type into the core.
    pub fn backend<E: core::error::Error + Send + Sync + 'static>(err: E) -> Self {
        SurfaceError::Backend(Box::new(err))
    }

    /// Wraps a platform error that is not `Send + Sync`, keeping only its message.
    ///
    /// [`SurfaceError`] is deliberately thread-safe so applications can put it in
    /// an `anyhow::Error` or return it from `main`. Some platform errors are not —
    /// softbuffer's, for one — so they get flattened to text here rather than
    /// infecting every caller.
    pub fn backend_msg(err: impl core::fmt::Display) -> Self {
        use alloc::string::ToString;
        SurfaceError::Backend(Box::new(BackendMessage(err.to_string())))
    }
}

/// A platform error reduced to its message.
#[derive(Debug)]
struct BackendMessage(alloc::string::String);

impl core::fmt::Display for BackendMessage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for BackendMessage {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn rejects_stride_below_width() {
        let mut buf = vec![0u32; 64];
        let err = Frame::new(
            &mut buf,
            Size::new(8, 8),
            4,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        );
        assert!(matches!(err, Err(SurfaceError::BufferTooSmall { .. })));
    }

    #[test]
    fn rejects_short_buffer() {
        let mut buf = vec![0u32; 10];
        let err = Frame::new(
            &mut buf,
            Size::new(8, 8),
            8,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        );
        assert!(matches!(err, Err(SurfaceError::BufferTooSmall { .. })));
    }

    #[test]
    fn accepts_exactly_sized_padded_buffer() {
        // 4 rows of stride 10, but the last row only needs its 6 visible pixels.
        let mut buf = vec![0u32; 10 * 3 + 6];
        let mut frame = Frame::new(
            &mut buf,
            Size::new(6, 4),
            10,
            PixelFormat::Xrgb8888,
            BufferAge::Frames(1),
        )
        .expect("geometry fits");
        assert_eq!(frame.rows_mut().count(), 4);
        assert!(frame.rows_mut().all(|r| r.len() == 6));
    }

    #[test]
    fn rows_skip_padding() {
        let mut buf = vec![0u32; 10 * 3];
        let mut frame = Frame::new(
            &mut buf,
            Size::new(6, 3),
            10,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .expect("geometry fits");
        for row in frame.rows_mut() {
            row.fill(0xFFFF_FFFF);
        }
        // Padding words 6..10 of each row must be untouched.
        assert!(buf.chunks(10).all(|c| c[6..].iter().all(|&p| p == 0)));
    }

    #[test]
    fn zero_size_is_not_ready() {
        let mut buf = vec![0u32; 4];
        assert!(matches!(
            Frame::new(
                &mut buf,
                Size::ZERO,
                0,
                PixelFormat::Xrgb8888,
                BufferAge::Undefined
            ),
            Err(SurfaceError::NotReady)
        ));
    }
}
