//! An owned buffer to draw into, for tests.

use denise::{PixelFormat, Size};

use crate::canvas::{Canvas, PixelView};

/// A zeroed pixel buffer plus the geometry needed to wrap it in a [`Canvas`].
///
/// The stride is deliberately settable and deliberately not equal to the width in
/// several tests: the padding between rows is where clipping bugs land, and a test
/// suite that only ever uses `stride == width` cannot see them.
pub struct TestCanvas {
    pixels: Vec<u32>,
    size: Size,
    stride: u32,
}

impl TestCanvas {
    /// A tightly packed buffer.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_stride(width, height, width)
    }

    /// A buffer with `stride - width` words of padding after each row.
    pub fn with_stride(width: u32, height: u32, stride: u32) -> Self {
        assert!(stride >= width);
        Self {
            pixels: vec![0; (stride * height) as usize],
            size: Size::new(width, height),
            stride,
        }
    }

    /// Borrows the buffer for drawing.
    pub fn canvas(&mut self) -> Canvas<'_> {
        Canvas::from_pixels(
            &mut self.pixels,
            self.size,
            self.stride,
            PixelFormat::Argb8888,
        )
        .expect("test geometry fits")
    }

    /// Borrows the buffer for reading.
    pub fn view(&self) -> PixelView<'_> {
        PixelView::new(&self.pixels, self.size, self.stride).expect("test geometry fits")
    }

    /// The whole backing slice, padding included.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// One pixel.
    pub fn at(&self, x: i32, y: i32) -> u32 {
        self.pixels[y as usize * self.stride as usize + x as usize]
    }
}
