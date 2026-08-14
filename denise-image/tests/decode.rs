//! Round trips and hand-built files, through the public API only.

use denise::Size;
use denise_image::{DecodeError, decode, decode_bmp};

/// Opaque red and blue as premultiplied words — which for alpha 255 are just
/// the colours.
const RED: u32 = 0xFFC8_2828;
const BLUE: u32 = 0xFF28_28C8;

// — BMP, hand-built byte by byte —

/// Builds an uncompressed BMP: `rows` of `(r, g, b, a)`, top row first.
fn bmp(rows: &[Vec<[u8; 4]>], bpp: u16, top_down: bool) -> Vec<u8> {
    let height = rows.len() as i32;
    let width = rows[0].len() as i32;
    let bytes_per_px = bpp as usize / 8;
    let stride = (width as usize * bytes_per_px).next_multiple_of(4);
    let data_size = stride * rows.len();

    let mut out = Vec::new();
    out.extend(b"BM");
    out.extend((54 + data_size as u32).to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(54u32.to_le_bytes()); // pixel data offset
    out.extend(40u32.to_le_bytes()); // BITMAPINFOHEADER
    out.extend(width.to_le_bytes());
    out.extend((if top_down { -height } else { height }).to_le_bytes());
    out.extend(1u16.to_le_bytes()); // planes
    out.extend(bpp.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // BI_RGB
    out.extend([0u8; 20]); // sizes and palette counts nobody checks
    assert_eq!(out.len(), 54);

    let mut ordered: Vec<&Vec<[u8; 4]>> = rows.iter().collect();
    if !top_down {
        ordered.reverse();
    }
    for row in ordered {
        let start = out.len();
        for &[r, g, b, a] in row {
            out.extend(if bpp == 32 {
                vec![b, g, r, a]
            } else {
                vec![b, g, r]
            });
        }
        out.resize(start + stride, 0);
    }
    out
}

#[test]
fn a_24_bit_bmp_decodes_bottom_up_into_top_down_rows() {
    let file = bmp(
        &[
            vec![[0xC8, 0x28, 0x28, 0]; 3],
            vec![[0x28, 0x28, 0xC8, 0]; 3],
        ],
        24,
        false,
    );
    let picture = decode(&file).expect("decode");
    assert_eq!(picture.size(), Size::new(3, 2));
    assert_eq!(picture.pixels()[0], RED, "top row must come out first");
    assert_eq!(picture.pixels()[3], BLUE);
}

#[test]
fn a_top_down_bmp_reads_the_same_picture() {
    let rows = vec![
        vec![[0xC8, 0x28, 0x28, 0]; 3],
        vec![[0x28, 0x28, 0xC8, 0]; 3],
    ];
    let up = decode_bmp(&bmp(&rows, 24, false)).expect("bottom-up");
    let down = decode_bmp(&bmp(&rows, 24, true)).expect("top-down");
    assert_eq!(up, down);
}

#[test]
fn a_32_bit_bmp_with_real_alpha_comes_out_premultiplied() {
    let file = bmp(&[vec![[200, 100, 50, 128]]], 32, false);
    let picture = decode_bmp(&file).expect("decode");
    // 200*128/255 = 100.4 -> 100, 100*128/255 -> 50, 50*128/255 -> 25.
    assert_eq!(picture.pixels()[0], 0x8064_3219);
}

#[test]
fn a_32_bit_bgrx_bmp_is_opaque_not_invisible() {
    // Files written as BGRX leave the reserved byte at zero. Trusting it
    // would make the whole image vanish.
    let file = bmp(&[vec![[0xC8, 0x28, 0x28, 0]; 2]], 32, false);
    let picture = decode_bmp(&file).expect("decode");
    assert_eq!(picture.pixels()[0], RED);
}

#[test]
fn bmp_row_padding_is_stepped_over() {
    // A 1px-wide 24-bit BMP has 3 data bytes and 1 padding byte per row; a
    // decoder that forgets walks off diagonally.
    let file = bmp(
        &[vec![[0xC8, 0x28, 0x28, 0]], vec![[0x28, 0x28, 0xC8, 0]]],
        24,
        false,
    );
    let picture = decode_bmp(&file).expect("decode");
    assert_eq!(picture.pixels(), &[RED, BLUE]);
}

#[test]
fn truncated_and_hostile_bmps_error_instead_of_panicking() {
    let good = bmp(&vec![vec![[1, 2, 3, 0]; 4]; 4], 24, false);
    for cut in [0, 2, 20, 53, good.len() - 1] {
        assert!(matches!(
            decode_bmp(&good[..cut]),
            Err(DecodeError::Malformed(_))
        ));
    }

    // A header that claims giant dimensions must be refused before the
    // allocation, not after it.
    let mut liar = good.clone();
    liar[18..22].copy_from_slice(&500_000u32.to_le_bytes());
    liar[22..26].copy_from_slice(&500_000u32.to_le_bytes());
    assert!(matches!(
        decode_bmp(&liar),
        Err(DecodeError::TooLarge {
            width: 500_000,
            height: 500_000
        })
    ));

    let mut compressed = good.clone();
    compressed[30] = 1; // BI_RLE8
    assert!(matches!(
        decode_bmp(&compressed),
        Err(DecodeError::Unsupported(_))
    ));
}

// — PNG, round-tripped through the png crate's own encoder —

#[cfg(feature = "png")]
fn encode_png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    writer.write_image_data(data).expect("data");
    drop(writer);
    out
}

#[cfg(feature = "png")]
#[test]
fn an_opaque_png_round_trips_pixel_exact() {
    let file = encode_png(
        2,
        1,
        png::ColorType::Rgb,
        &[0xC8, 0x28, 0x28, 0x28, 0x28, 0xC8],
    );
    let picture = decode(&file).expect("decode");
    assert_eq!(picture.size(), Size::new(2, 1));
    assert_eq!(picture.pixels(), &[RED, BLUE]);
}

#[cfg(feature = "png")]
#[test]
fn png_transparency_comes_out_premultiplied() {
    let file = encode_png(1, 1, png::ColorType::Rgba, &[200, 100, 50, 128]);
    let picture = decode(&file).expect("decode");
    assert_eq!(picture.pixels()[0], 0x8064_3219);
}

#[cfg(feature = "png")]
#[test]
fn greyscale_pngs_come_out_as_the_grey_they_are() {
    let file = encode_png(2, 1, png::ColorType::Grayscale, &[0x00, 0xAB]);
    let picture = decode(&file).expect("decode");
    assert_eq!(picture.pixels(), &[0xFF00_0000, 0xFFAB_ABAB]);
}

#[cfg(feature = "png")]
#[test]
fn a_truncated_png_errors_instead_of_panicking() {
    let file = encode_png(4, 4, png::ColorType::Rgb, &[7u8; 48]);
    for cut in [8, 20, file.len() / 2] {
        assert!(matches!(
            decode(&file[..cut]),
            Err(DecodeError::Malformed(_))
        ));
    }
}

// — GIF, round-tripped through the gif crate's own encoder —

#[cfg(feature = "gif")]
#[test]
fn a_gif_round_trips_through_its_palette() {
    let mut out = Vec::new();
    {
        let palette = &[0xC8, 0x28, 0x28, 0x28, 0x28, 0xC8];
        let mut encoder = gif::Encoder::new(&mut out, 2, 1, palette).expect("encoder");
        let frame = gif::Frame {
            width: 2,
            height: 1,
            buffer: std::borrow::Cow::Borrowed(&[0, 1]),
            ..Default::default()
        };
        encoder.write_frame(&frame).expect("frame");
    }
    let picture = decode(&out).expect("decode");
    assert_eq!(picture.size(), Size::new(2, 1));
    assert_eq!(picture.pixels(), &[RED, BLUE]);
}

#[cfg(feature = "gif")]
#[test]
fn a_gif_frame_smaller_than_the_screen_lands_at_its_offset() {
    let mut out = Vec::new();
    {
        let palette = &[0xC8, 0x28, 0x28];
        let mut encoder = gif::Encoder::new(&mut out, 4, 4, palette).expect("encoder");
        let frame = gif::Frame {
            left: 2,
            top: 3,
            width: 1,
            height: 1,
            buffer: std::borrow::Cow::Borrowed(&[0u8]),
            ..Default::default()
        };
        encoder.write_frame(&frame).expect("frame");
    }
    let picture = decode(&out).expect("decode");
    assert_eq!(picture.size(), Size::new(4, 4));
    assert_eq!(picture.pixels()[(3 * 4 + 2) as usize], RED);
    assert_eq!(picture.pixels()[0], 0, "off-frame pixels stay transparent");
}

// — JPEG, from a committed fixture (nothing in the tree encodes JPEG) —

#[cfg(feature = "jpeg")]
#[test]
fn the_jpeg_fixture_decodes_to_its_two_halves() {
    let bytes = include_bytes!("fixtures/two-halves.jpg");
    let picture = decode(bytes).expect("decode");
    assert_eq!(picture.size(), Size::new(32, 24));

    // JPEG is lossy: assert the halves are the right colours within a
    // tolerance, sampled away from the seam where ringing lives.
    let near = |px: u32, want: u32| {
        px.to_be_bytes()
            .iter()
            .zip(want.to_be_bytes())
            .all(|(&a, b)| a.abs_diff(b) <= 16)
    };
    let at = |x: u32, y: u32| picture.pixels()[(y * 32 + x) as usize];
    assert!(near(at(4, 12), RED), "left half was {:#010X}", at(4, 12));
    assert!(
        near(at(28, 12), BLUE),
        "right half was {:#010X}",
        at(28, 12)
    );
}

// — the front door —

#[test]
fn unrecognised_bytes_say_so() {
    assert_eq!(
        decode(b"not an image at all"),
        Err(DecodeError::Unrecognised)
    );
    assert_eq!(decode(&[]), Err(DecodeError::Unrecognised));
}

// — the invariant every decoder now goes through —

/// Whatever a decoder produces, the buffer matches the size on the tin.
///
/// `Picture::pixels` promises `width` words per row, and `PixelView::new` in
/// `denise-render` checks that before it will draw: a picture whose buffer is
/// short does not crash, it silently renders nothing from a decode that said
/// `Ok`. This walks every format the crate can build here and asserts the
/// arithmetic, so a decoder that starts disagreeing with its own header is
/// caught at the boundary rather than on a panel.
#[test]
fn every_decoded_picture_has_exactly_the_pixels_its_size_claims() {
    // `width` red columns, `height` rows deep — the shape is what matters here,
    // not the colour.
    let block = |width: usize, height: usize| -> Vec<Vec<[u8; 4]>> {
        vec![vec![[0xC8, 0x28, 0x28, 0xFF]; width]; height]
    };

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("bmp 24-bit", bmp(&block(4, 3), 24, false)),
        ("bmp 32-bit", bmp(&block(4, 3), 32, false)),
        ("bmp top-down", bmp(&block(5, 2), 24, true)),
        // A width whose rows need the four-byte padding, which is where a
        // stride/width confusion would show up.
        ("bmp odd width", bmp(&block(3, 3), 24, false)),
        (
            "jpeg fixture",
            include_bytes!("fixtures/two-halves.jpg").to_vec(),
        ),
    ];

    for (name, bytes) in cases {
        let picture = decode(&bytes).unwrap_or_else(|e| panic!("{name} did not decode: {e}"));
        let size = picture.size();
        assert_eq!(
            picture.pixels().len(),
            size.width as usize * size.height as usize,
            "{name} produced a buffer that does not match its {}x{}",
            size.width,
            size.height,
        );
    }
}

/// A BMP whose header promises more rows than the file carries is malformed,
/// and says so rather than decoding to a short buffer.
#[test]
fn a_bmp_that_outruns_its_own_data_is_malformed() {
    let rows = vec![vec![[0xC8, 0x28, 0x28, 0xFF]; 4]; 3];
    let mut bytes = bmp(&rows, 24, false);
    bytes.truncate(bytes.len() - 12);
    assert!(matches!(decode_bmp(&bytes), Err(DecodeError::Malformed(_))));
}
