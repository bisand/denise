//! A panel binary with no font parser in it.
//!
//! ```text
//! cargo run -p baked -- baked.ppm
//! ```
//!
//! `build.rs` rasterised a face at 14, 20 and 32 px into `FACE`, a `static`
//! of plain tables, and this program draws from those. It depends on
//! `denise-text` with no features: it could not read a font file if it were
//! handed one, and that is the point — a kiosk that ships one face, at the
//! sizes it was designed at, has nothing to parse at boot and nothing in its
//! binary that parses. The same text through the `truetype` tier would draw
//! byte for byte the same, at about 270 KB more of binary.
//!
//! A size that was not baked — 16 here — snaps to the nearest that was, and
//! the program says so, because a panel that silently draws 14 px where its
//! layout was measured at 16 is the kind of thing this toolkit refuses to do.

use std::io::Write as _;

use denise::{BufferAge, Frame, PixelFormat, Point, Rect, Role, Size, Theme, theme};
use denise_render::Canvas;
use denise_text::{BakedSource, TextEngine, TextStyle};

include!(concat!(env!("OUT_DIR"), "/FACE.rs"));

const SIZE: Size = Size::new(720, 300);
const SAMPLE: &str = "Kjærlighet på Øy — 21.5 °C";

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "baked.ppm".to_owned());

    let mut engine = TextEngine::new();
    let face = engine.add_font(Box::new(BakedSource::new(&FACE)));
    let tables: usize = FACE.sizes.iter().map(|size| size.glyphs.len()).sum();
    eprintln!(
        "{}: {} sizes, {} glyph entries, {} bytes of coverage",
        FACE.name,
        FACE.sizes.len(),
        tables,
        FACE.coverage.len()
    );

    let theme: Theme = theme::DARK;
    let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
    {
        let mut frame = Frame::new(
            &mut pixels,
            SIZE,
            SIZE.width,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .expect("frame");
        let mut raster = Canvas::new(&mut frame);
        let mut canvas = raster.pen();
        canvas.clear(theme.color(Role::Base100));

        let mut y = 20;
        for size_px in [14, 16, 20, 32] {
            let style = TextStyle {
                font: face,
                size_px,
            };
            let snapped = engine.snap_size(style);
            let label = if snapped == size_px {
                format!("{size_px} px")
            } else {
                format!("{size_px} px → {snapped}")
            };
            engine.draw(
                &mut canvas,
                TextStyle {
                    font: face,
                    size_px: 14,
                },
                Point::new(16, y),
                &label,
                theme.color(if snapped == size_px {
                    Role::BaseContent
                } else {
                    Role::Warning
                }),
            );
            engine.draw(
                &mut canvas,
                style,
                Point::new(140, y),
                SAMPLE,
                theme.color(Role::BaseContent),
            );
            y += engine.line_height(style) + 10;
        }
        canvas.stroke_rect(
            Rect::new(8, 8, SIZE.width as i32 - 16, y - 8),
            1,
            theme.color(Role::Base300),
        );
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
    write!(out, "P6\n{} {}\n255\n", SIZE.width, SIZE.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path}");
    Ok(())
}
