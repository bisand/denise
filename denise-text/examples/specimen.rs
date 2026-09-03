//! Renders a font specimen to a PPM, so a font can be judged before a device is.
//!
//! ```text
//! cargo run -p denise-text --features truetype --example specimen -- specimen.ppm [font.ttf]
//! ```
//!
//! With no font path it renders the built-in bitmap font, which needs no feature
//! flag and no file. With one, it renders that face beside the built-in one at the
//! same sizes, which is the comparison that actually decides whether 145 KB is
//! worth spending on a given panel.
//!
//! No font ships with Denise. Type designers' licences differ, and quietly
//! embedding somebody's font in a toolkit is a decision for whoever ships the
//! device, not for the toolkit.

use std::io::Write as _;

use denise::{BufferAge, Frame, PixelFormat, Point, Rect, Role, Size, Theme, theme};
use denise_render::Canvas;
#[cfg(feature = "truetype")]
use denise_text::GlyphSource as _;
use denise_text::{TextEngine, TextStyle};

const SIZE: Size = Size::new(880, 800);
const SAMPLE: &str = "Kjærlighet på Øy";
const PANGRAM: &str = "Vår sære Zulu fra badeøya spilte jo whist og quickstep i min taxi.";
const SIZES: [u16; 5] = [8, 16, 24, 32, 48];

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "specimen.ppm".to_owned());
    let font_path = args.next();
    // A third argument replaces the sample, which is how you check whether a
    // given tier can actually draw the script a panel has to show.
    let sample = args.next();
    let sample = sample.as_deref().unwrap_or(SAMPLE);

    let mut engine = TextEngine::new();
    // `mut` only when a second face can be added, which is feature-dependent.
    #[allow(unused_mut)]
    let mut faces = vec![(TextStyle::built_in(16).font, "built-in 5x7".to_owned())];

    #[cfg(feature = "truetype")]
    if let Some(font_path) = &font_path {
        let data = std::fs::read(font_path)?;
        match denise_text::TrueTypeSource::from_bytes(font_path, &data) {
            Ok(source) => {
                let name = source.name().to_owned();
                let id = engine.add_font(Box::new(source));
                faces.push((id, name));
            }
            Err(error) => eprintln!("could not parse {font_path}: {error}"),
        }
    }
    #[cfg(feature = "shaping")]
    if let Some(font_path) = &font_path {
        let data = std::fs::read(font_path)?;
        match denise_text::ShapedSource::from_fonts("shaped", [data]) {
            Ok(source) => {
                let id = engine.add_font(Box::new(source));
                faces.push((id, "shaped (cosmic-text)".to_owned()));
            }
            Err(error) => eprintln!("could not build a shaper: {error}"),
        }
    }
    #[cfg(not(any(feature = "truetype", feature = "shaping")))]
    if font_path.is_some() {
        eprintln!("built without --features truetype; ignoring the font path");
    }

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

        let mut y = 16;
        for (font, name) in &faces {
            let heading = TextStyle {
                font: *font,
                size_px: 16,
            };
            engine.draw(
                &mut canvas,
                heading,
                Point::new(16, y),
                name,
                theme.color(Role::Accent),
            );
            y += engine.line_height(heading) + 6;

            for size in SIZES {
                let style = TextStyle {
                    font: *font,
                    size_px: size,
                };
                let snapped = engine.snap_size(style);
                let label = format!("{size}px");
                engine.draw(
                    &mut canvas,
                    TextStyle::built_in(8),
                    Point::new(16, y + 4),
                    &label,
                    theme.color(Role::Base300),
                );
                engine.draw(
                    &mut canvas,
                    style,
                    Point::new(64, y),
                    sample,
                    theme.color(Role::BaseContent),
                );
                if snapped != size {
                    engine.draw(
                        &mut canvas,
                        TextStyle::built_in(8),
                        Point::new(16, y + 14),
                        &format!("→{snapped}"),
                        theme.color(Role::Warning),
                    );
                }
                y += engine.line_height(style).max(12) + 4;
            }

            // A pangram at a readable size, to show spacing rather than shapes.
            let body = TextStyle {
                font: *font,
                size_px: 16,
            };
            let width = engine.measure_line(body, PANGRAM);
            engine.draw(
                &mut canvas,
                body,
                Point::new(16, y),
                PANGRAM,
                theme.color(Role::BaseContent),
            );
            y += engine.line_height(body) + 16;
            eprintln!("{name}: pangram is {width} px wide at 16 px");
        }

        // A frame around the last line, to show that measurement and ink agree.
        let stats = engine.stats();
        eprintln!(
            "{} glyphs cached, {} hits, {} misses, {} resets",
            engine.atlas().len(),
            stats.hits,
            stats.misses,
            stats.resets
        );
        canvas.stroke_rect(
            Rect::new(8, 8, SIZE.width as i32 - 16, y.min(SIZE.height as i32) - 8),
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
