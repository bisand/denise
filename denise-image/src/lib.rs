//! Image decoding for Denise: bytes in, premultiplied pixels out.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("logo.png")?;
//! let picture = denise_image::decode(&bytes)?;
//! let (pixels, size) = picture.into_parts();
//! // denise_ui::widgets::Image::new(pixels, size)
//! # Ok(())
//! # }
//! ```
//!
//! [`decode`] recognises the format from the bytes; the per-format functions
//! exist for callers that already know. Every decoder produces the same thing:
//! tightly packed rows of **premultiplied** `0xAARRGGBB`, which is exactly what
//! [`Canvas::blit`](denise_render::Canvas::blit) and the `Image` widget in
//! `denise-ui` consume. The multiply by alpha happens here, once, so drawing
//! never pays it.
//!
//! # Formats, and what each costs
//!
//! | Format | Decoder | Feature |
//! |---|---|---|
//! | PNG (including APNG's first frame) | the [`png`] crate | `png`, default |
//! | JPEG | the [`zune-jpeg`] crate | `jpeg`, default |
//! | GIF (first frame) | the [`gif`] crate | `gif`, default |
//! | BMP, uncompressed 24/32-bit | this crate, ~100 lines | always |
//!
//! Each decoder is a cargo feature so a panel pays binary size only for the
//! formats it ships — the same arrangement as `truetype`/`shaping` in
//! `denise-text`. The measured costs are in the README. BMP is not gated
//! because the hand-rolled decoder is smaller than the gate would be.
//!
//! Animated GIFs decode to their first frame — deliberately. Playback is a
//! frame cache times the animation clock, and belongs to a later issue; the
//! [`gif`] crate underneath streams frames, so nothing here forecloses it.
//!
//! # What this crate refuses to do
//!
//! No file I/O — the application reads bytes and passes them, because a
//! decoder that opens paths is unusable over the FFI and wrong in an embedded
//! toolkit. No scaling — that is the rasteriser's job, at draw time. And
//! nothing decodes to more than [`MAX_PIXELS`] pixels: a panel toolkit has no
//! business allocating a third of a small board's RAM because a file's header
//! asked it to.
//!
//! [`png`]: https://crates.io/crates/png
//! [`zune-jpeg`]: https://crates.io/crates/zune-jpeg
//! [`gif`]: https://crates.io/crates/gif

// `chunks_exact` over `as_chunks`, against clippy 1.98's advice: `as_chunks`
// stabilised in 1.98 and this workspace supports 1.95, so taking the advice
// would trade a style lint for a compile error on every older toolchain. Revisit
// when the MSRV passes 1.98. `unknown_lints` because the lint does not exist
// before 1.98 either, and naming an absent lint is itself a warning.
#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

use denise::Size;
use denise_render::blend::premultiply;

/// The most pixels a decode is willing to produce: 32 megapixels, which is
/// 128 MiB of `u32` — past every real panel asset and comfortably inside what
/// a header lying about its dimensions could otherwise make [`decode`]
/// allocate.
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;

/// Decoded pixels: tightly packed premultiplied `0xAARRGGBB` rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pixels: Vec<u32>,
    size: Size,
}

impl Picture {
    /// Builds a picture, or fails if the buffer is not exactly the size claimed.
    ///
    /// Every decoder goes through here. The invariant this enforces —
    /// `pixels.len() == width * height` — is what [`Picture::pixels`] promises
    /// and what [`PixelView`](denise_render::PixelView) checks before it will
    /// draw anything, so a mismatch that got this far would not crash: it would
    /// silently render nothing, from a decode that returned `Ok`. A file that
    /// cannot honour its own header is malformed, and saying so is more use than
    /// an invisible image.
    fn checked(pixels: Vec<u32>, size: Size) -> Result<Self, DecodeError> {
        let expected = size.width as usize * size.height as usize;
        if pixels.len() != expected {
            return Err(DecodeError::Malformed(format!(
                "the decoder produced {} pixels for a {}x{} image, which needs {expected}",
                pixels.len(),
                size.width,
                size.height,
            )));
        }
        Ok(Self { pixels, size })
    }

    /// Width and height in pixels.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The pixel rows, `size().width` words each, premultiplied.
    #[inline]
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Surrenders the buffer, in the shape `Image::new` in `denise-ui` takes.
    #[inline]
    pub fn into_parts(self) -> (Vec<u32>, Size) {
        (self.pixels, self.size)
    }
}

/// Why a decode failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The bytes match no format this crate knows.
    Unrecognised,
    /// The format was recognised, but its decoder is compiled out — the named
    /// cargo feature would enable it.
    Disabled(&'static str),
    /// The file is damaged, truncated, or not what its header claims. The
    /// message is the underlying decoder's.
    Malformed(String),
    /// The header asks for more than [`MAX_PIXELS`] pixels. Reported before
    /// anything is allocated.
    TooLarge {
        /// Claimed width in pixels.
        width: u32,
        /// Claimed height in pixels.
        height: u32,
    },
    /// A valid file in a variant this crate does not support, such as a
    /// compressed or 16-colour BMP.
    Unsupported(&'static str),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unrecognised => write!(f, "not a PNG, JPEG, GIF or BMP"),
            Self::Disabled(feature) => write!(
                f,
                "recognised the format, but the `{feature}` feature of denise-image is compiled out"
            ),
            Self::Malformed(why) => write!(f, "malformed image: {why}"),
            Self::TooLarge { width, height } => write!(
                f,
                "{width}x{height} exceeds the {MAX_PIXELS}-pixel decode limit"
            ),
            Self::Unsupported(what) => write!(f, "unsupported image variant: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes an image, recognising the format from the bytes themselves.
///
/// File extensions are not consulted — there is no file. The magic numbers at
/// the front of the data decide, so a PNG renamed `.jpg` decodes as the PNG it
/// is.
pub fn decode(bytes: &[u8]) -> Result<Picture, DecodeError> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        #[cfg(feature = "png")]
        return decode_png(bytes);
        #[cfg(not(feature = "png"))]
        return Err(DecodeError::Disabled("png"));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        #[cfg(feature = "jpeg")]
        return decode_jpeg(bytes);
        #[cfg(not(feature = "jpeg"))]
        return Err(DecodeError::Disabled("jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        #[cfg(feature = "gif")]
        return decode_gif(bytes);
        #[cfg(not(feature = "gif"))]
        return Err(DecodeError::Disabled("gif"));
    }
    if bytes.starts_with(b"BM") {
        return decode_bmp(bytes);
    }
    Err(DecodeError::Unrecognised)
}

/// Refuses dimensions that are zero or would decode past [`MAX_PIXELS`],
/// before anything is allocated.
fn checked_size(width: u32, height: u32) -> Result<Size, DecodeError> {
    if width == 0 || height == 0 {
        return Err(DecodeError::Malformed("zero-sized image".into()));
    }
    if width as u64 * height as u64 > MAX_PIXELS {
        return Err(DecodeError::TooLarge { width, height });
    }
    Ok(Size::new(width, height))
}

/// Packs straight-alpha RGBA bytes into premultiplied words.
#[cfg(feature = "png")]
fn from_rgba(data: &[u8], size: Size) -> Result<Picture, DecodeError> {
    let mut pixels: Vec<u32> = data
        .chunks_exact(4)
        .map(|px| u32::from_be_bytes([px[3], px[0], px[1], px[2]]))
        .collect();
    premultiply(&mut pixels);
    Picture::checked(pixels, size)
}

/// Packs opaque RGB bytes into words. Nothing to premultiply.
#[cfg(any(feature = "png", feature = "jpeg"))]
fn from_rgb(data: &[u8], size: Size) -> Result<Picture, DecodeError> {
    let pixels = data
        .chunks_exact(3)
        .map(|px| u32::from_be_bytes([0xFF, px[0], px[1], px[2]]))
        .collect();
    Picture::checked(pixels, size)
}

/// Decodes a PNG. Palette, greyscale and 16-bit files are expanded to 8-bit
/// colour by the decoder; an APNG decodes to its first frame.
#[cfg(feature = "png")]
pub fn decode_png(bytes: &[u8]) -> Result<Picture, DecodeError> {
    let malformed = |e: png::DecodingError| DecodeError::Malformed(e.to_string());

    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(malformed)?;
    let info = reader.info();
    let size = checked_size(info.width, info.height)?;

    let buffer_size = reader.output_buffer_size().ok_or(DecodeError::TooLarge {
        width: size.width,
        height: size.height,
    })?;
    let mut buf = vec![0u8; buffer_size];
    let out = reader.next_frame(&mut buf).map_err(malformed)?;
    let data = &buf[..out.buffer_size()];

    // `info` describes the canvas; `out` describes the frame that was actually
    // decoded, and for an APNG whose first frame is smaller than the canvas the
    // two differ. The pixels in hand are the frame's, so that is what this
    // picture is — composing a sub-frame onto the canvas is what animation
    // support will have to do, and guessing at it here would produce an image
    // whose buffer does not match its own size.
    let size = checked_size(out.width, out.height)?;

    Ok(match out.color_type {
        png::ColorType::Rgba => from_rgba(data, size)?,
        png::ColorType::Rgb => from_rgb(data, size)?,
        png::ColorType::Grayscale => {
            let pixels = data
                .iter()
                .map(|&g| u32::from_be_bytes([0xFF, g, g, g]))
                .collect();
            Picture::checked(pixels, size)?
        }
        png::ColorType::GrayscaleAlpha => {
            let mut pixels: Vec<u32> = data
                .chunks_exact(2)
                .map(|px| u32::from_be_bytes([px[1], px[0], px[0], px[0]]))
                .collect();
            premultiply(&mut pixels);
            Picture::checked(pixels, size)?
        }
        // EXPAND turns palette files into one of the arms above.
        png::ColorType::Indexed => {
            return Err(DecodeError::Malformed(
                "the decoder returned indexed pixels it promised to expand".into(),
            ));
        }
    })
}

/// Decodes a JPEG. Greyscale and CMYK files come out as the colour they show.
#[cfg(feature = "jpeg")]
pub fn decode_jpeg(bytes: &[u8]) -> Result<Picture, DecodeError> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|e| DecodeError::Malformed(e.to_string()))?;
    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| DecodeError::Malformed("no dimensions in the JPEG header".into()))?;
    let size = checked_size(width as u32, height as u32)?;

    let data = decoder
        .decode()
        .map_err(|e| DecodeError::Malformed(e.to_string()))?;
    from_rgb(&data, size)
}

/// Decodes a GIF to its **first frame**, composed at the file's full logical
/// size — a frame smaller than the screen lands at its offset on transparent
/// pixels, exactly as a viewer would show it.
#[cfg(feature = "gif")]
pub fn decode_gif(bytes: &[u8]) -> Result<Picture, DecodeError> {
    let malformed = |e: gif::DecodingError| DecodeError::Malformed(e.to_string());

    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(bytes).map_err(malformed)?;
    let size = checked_size(decoder.width() as u32, decoder.height() as u32)?;

    let frame = decoder
        .read_next_frame()
        .map_err(malformed)?
        .ok_or_else(|| DecodeError::Malformed("a GIF with no frames".into()))?;

    let mut pixels = vec![0u32; (size.width * size.height) as usize];
    let (left, top) = (frame.left as u32, frame.top as u32);
    for y in 0..frame.height as u32 {
        for x in 0..frame.width as u32 {
            let (dx, dy) = (left + x, top + y);
            if dx >= size.width || dy >= size.height {
                continue;
            }
            let i = ((y * frame.width as u32 + x) * 4) as usize;
            // `get`, not an index: the buffer's length is the gif crate's promise
            // about a file this crate did not write, and a truncated frame should
            // leave transparent pixels rather than panic a panel.
            let Some(px) = frame.buffer.get(i..i + 4) else {
                continue;
            };
            pixels[(dy * size.width + dx) as usize] =
                u32::from_be_bytes([px[3], px[0], px[1], px[2]]);
        }
    }
    premultiply(&mut pixels);
    Picture::checked(pixels, size)
}

/// Decodes an uncompressed 24- or 32-bit BMP — which is virtually every BMP
/// actually in circulation. Bottom-up and top-down rows both handled.
///
/// The 32-bit format's fourth byte is officially "reserved", and files written
/// as `BGRX` fill it with zero — an image that trusted it would be entirely
/// invisible. So the alpha channel is honoured only when some pixel actually
/// uses it, which is the same heuristic every viewer applies.
pub fn decode_bmp(bytes: &[u8]) -> Result<Picture, DecodeError> {
    fn u16at(bytes: &[u8], at: usize) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(field::<2>(bytes, at)?))
    }
    fn u32at(bytes: &[u8], at: usize) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(field::<4>(bytes, at)?))
    }
    fn field<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], DecodeError> {
        bytes
            .get(at..at + N)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| DecodeError::Malformed("truncated BMP header".into()))
    }

    if !bytes.starts_with(b"BM") {
        return Err(DecodeError::Malformed("not a BMP".into()));
    }
    let data_offset = u32at(bytes, 10)? as usize;
    if u32at(bytes, 14)? < 40 {
        return Err(DecodeError::Unsupported("BMP with a BITMAPCOREHEADER"));
    }
    let raw_width = u32at(bytes, 18)? as i32;
    let raw_height = u32at(bytes, 22)? as i32;
    let bpp = u16at(bytes, 28)?;
    let compression = u32at(bytes, 30)?;

    if compression != 0 {
        return Err(DecodeError::Unsupported("compressed BMP"));
    }
    if bpp != 24 && bpp != 32 {
        return Err(DecodeError::Unsupported("BMP that is not 24- or 32-bit"));
    }
    if raw_width <= 0 || raw_height == 0 || raw_height == i32::MIN {
        return Err(DecodeError::Malformed("BMP dimensions out of range".into()));
    }
    // Negative height is the header's way of saying rows run top-down.
    let top_down = raw_height < 0;
    let size = checked_size(raw_width as u32, raw_height.unsigned_abs())?;

    let bytes_per_px = bpp as usize / 8;
    // Rows are padded to four-byte boundaries.
    let stride = (size.width as usize * bytes_per_px).next_multiple_of(4);
    let data = bytes
        .get(data_offset..data_offset + stride * size.height as usize)
        .ok_or_else(|| DecodeError::Malformed("truncated BMP pixel data".into()))?;

    let mut pixels = Vec::with_capacity((size.width * size.height) as usize);
    let mut alpha_seen = false;
    for y in 0..size.height as usize {
        let row = if top_down {
            y
        } else {
            size.height as usize - 1 - y
        };
        let row = &data[row * stride..];
        for x in 0..size.width as usize {
            let px = &row[x * bytes_per_px..];
            let a = if bpp == 32 { px[3] } else { 0xFF };
            alpha_seen |= bpp == 32 && a != 0;
            pixels.push(u32::from_be_bytes([a, px[2], px[1], px[0]]));
        }
    }
    if bpp == 32 {
        if alpha_seen {
            premultiply(&mut pixels);
        } else {
            // Every alpha byte was zero: a BGRX file, not a transparent image.
            for px in &mut pixels {
                *px |= 0xFF00_0000;
            }
        }
    }
    Picture::checked(pixels, size)
}

/// Compiles the examples in this crate's README, so they cannot drift from the
/// API they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
