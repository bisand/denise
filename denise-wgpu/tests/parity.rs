//! The GPU painter against the software rasteriser, within a tolerance.
//!
//! The two are not byte-identical and are not meant to be: one anti-aliases
//! analytically per scanline in integers, the other by signed distance in
//! floats. What must hold is that a person could not tell them apart — so each
//! primitive is drawn both ways and the frames are compared with a bound on the
//! mean difference and a bound on how many pixels differ by a lot. The exact
//! cases — solid rectangles, the clip, the clear — are compared for equality,
//! because for those there is no excuse.
//!
//! With no adapter (a CI runner without a software Vulkan) every test skips.
//! Skipping is a pass here, and the reason is printed so it is never silent.

use denise::{BufferAge, Color, Frame, Paint, Pen, PixelFormat, Point, Rect, Size};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};
use denise_wgpu::{Gpu, wgpu};

const SIZE: Size = Size::new(200, 120);

/// A target this test owns, so an incremental frame has somewhere to persist.
fn target(gpu: &Gpu) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("parity target"),
        size: wgpu::Extent3d {
            width: SIZE.width,
            height: SIZE.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Draws `scene` through both painters and returns (software, gpu) frames.
fn both(scene: impl Fn(&mut Pen<'_>)) -> Option<(Vec<u32>, Vec<u32>)> {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping: {err}");
            return None;
        }
    };

    let mut software = vec![0u32; (SIZE.width * SIZE.height) as usize];
    {
        let mut frame = Frame::new(
            &mut software,
            SIZE,
            SIZE.width,
            PixelFormat::Argb8888,
            BufferAge::Undefined,
        )
        .expect("frame");
        let mut canvas = Canvas::new(&mut frame);
        let mut pen = canvas.pen();
        pen.clear(Color::BLACK);
        scene(&mut pen);
    }
    // The software frame's high byte is whatever the fill left; the GPU's is a
    // rendered alpha. Compare colour, which is what the eye sees.
    for w in &mut software {
        *w |= 0xFF00_0000;
    }

    let mut painter = gpu.painter(SIZE);
    {
        let mut pen = Pen::new(&mut painter);
        pen.clear(Color::BLACK);
        scene(&mut pen);
    }
    let mut hardware = painter.finish_to_pixels().expect("readback");
    for w in &mut hardware {
        *w |= 0xFF00_0000;
    }
    Some((software, hardware))
}

/// Per-channel absolute difference, largest channel, for one pixel.
fn diff(a: u32, b: u32) -> u32 {
    (0..3)
        .map(|i| {
            let shift = i * 8;
            ((a >> shift) & 0xFF).abs_diff((b >> shift) & 0xFF)
        })
        .max()
        .unwrap_or(0)
}

struct Stats {
    mean: f64,
    /// Fraction of pixels whose largest channel differs by more than 64.
    far: f64,
    max: u32,
}

fn stats(a: &[u32], b: &[u32]) -> Stats {
    assert_eq!(a.len(), b.len());
    let mut sum = 0u64;
    let mut far = 0usize;
    let mut max = 0u32;
    for (&x, &y) in a.iter().zip(b) {
        let d = diff(x, y);
        sum += u64::from(d);
        if d > 64 {
            far += 1;
        }
        max = max.max(d);
    }
    Stats {
        mean: sum as f64 / a.len() as f64,
        far: far as f64 / a.len() as f64,
        max,
    }
}

/// Asserts the two frames are close, printing the numbers either way so a
/// threshold that needs tuning is tuned on evidence.
fn assert_close(name: &str, sw: &[u32], hw: &[u32], mean: f64, far: f64) {
    let s = stats(sw, hw);
    eprintln!(
        "{name}: mean {:.3}, far {:.4}%, max {}",
        s.mean,
        s.far * 100.0,
        s.max
    );
    assert!(
        s.mean <= mean,
        "{name}: mean difference {:.3} exceeds {mean}",
        s.mean
    );
    assert!(
        s.far <= far,
        "{name}: {:.3}% of pixels differ by more than 64, allowed {:.3}%",
        s.far * 100.0,
        far * 100.0
    );
}

fn assert_identical(name: &str, sw: &[u32], hw: &[u32]) {
    let s = stats(sw, hw);
    assert_eq!(
        s.max, 0,
        "{name}: frames differ, max channel difference {}",
        s.max
    );
}

// ------------------------------------------------------------------ exact

#[test]
fn solid_rectangles_are_identical() {
    let Some((sw, hw)) = both(|p| {
        p.fill_rect(Rect::new(10, 10, 60, 40), Color::from_rgb888(0x89B4FA));
        p.fill_rect(Rect::new(-5, 90, 300, 20), Color::from_rgb888(0xA6E3A1));
        p.fill_rect(Rect::new(150, -8, 30, 30), Color::WHITE);
    }) else {
        return;
    };
    assert_identical("solid rectangles", &sw, &hw);
}

#[test]
fn the_clip_is_respected_exactly() {
    let Some((sw, hw)) = both(|p| {
        let mut c = p.with_clip(Rect::new(20, 20, 50, 30));
        c.fill_rect(Rect::new(0, 0, 200, 120), Color::from_rgb888(0xF38BA8));
        {
            let mut inner = c.with_clip(Rect::new(40, 10, 100, 100));
            inner.fill_rect(Rect::new(0, 0, 200, 120), Color::WHITE);
        }
        // Back to the outer clip after the inner one drops.
        c.fill_rect(Rect::new(0, 40, 200, 4), Color::from_rgb888(0x94E2D5));
    }) else {
        return;
    };
    assert_identical("clip", &sw, &hw);
}

#[test]
fn translucent_fills_composite_the_same_way() {
    let Some((sw, hw)) = both(|p| {
        p.fill_rect(Rect::new(0, 0, 200, 120), Color::from_rgb888(0x1E1E2E));
        p.fill_rect(Rect::new(20, 20, 100, 60), Color::rgba(255, 255, 255, 96));
        p.fill_rect(Rect::new(60, 40, 100, 60), Color::rgba(137, 180, 250, 160));
    }) else {
        return;
    };
    // Rounding differs by at most one step per channel between integer and
    // float blending.
    let s = stats(&sw, &hw);
    eprintln!("translucent: max {}", s.max);
    assert!(
        s.max <= 2,
        "translucent fills differ by {} per channel",
        s.max
    );
}

// ------------------------------------------------------------- anti-aliased

#[test]
fn rounded_rectangles_are_close() {
    let Some((sw, hw)) = both(|p| {
        p.fill_rounded_rect(Rect::new(10, 10, 80, 50), 12, Color::from_rgb888(0x89B4FA));
        p.stroke_rounded_rect(
            Rect::new(100, 10, 90, 50),
            8,
            3,
            Color::from_rgb888(0xF9E2AF),
        );
        p.fill_rounded_rect(Rect::new(10, 70, 40, 40), 20, Color::WHITE);
        p.stroke_rounded_rect(
            Rect::new(60, 70, 130, 40),
            0,
            2,
            Color::from_rgb888(0xA6E3A1),
        );
    }) else {
        return;
    };
    assert_close("rounded rectangles", &sw, &hw, 1.5, 0.006);
}

#[test]
fn circles_and_arcs_are_close() {
    let Some((sw, hw)) = both(|p| {
        p.fill_circle(Point::new(40, 40), 30, Color::from_rgb888(0xF38BA8));
        p.stroke_circle(Point::new(110, 40), 30, 6, Color::from_rgb888(0x94E2D5));
        p.stroke_arc(
            Point::new(160, 80),
            30,
            8,
            0,
            70 * denise::TURN / 100,
            Color::WHITE,
        );
        p.stroke_arc(
            Point::new(60, 90),
            20,
            4,
            denise::TURN / 4,
            -denise::TURN / 2,
            Color::from_rgb888(0xF9E2AF),
        );
    }) else {
        return;
    };
    assert_close("circles and arcs", &sw, &hw, 2.0, 0.012);
}

#[test]
fn lines_are_close() {
    let Some((sw, hw)) = both(|p| {
        p.draw_line(Point::new(10, 10), Point::new(190, 110), Color::WHITE);
        p.draw_line(
            Point::new(10, 110),
            Point::new(190, 10),
            Color::from_rgb888(0x89B4FA),
        );
        p.draw_line(
            Point::new(100, 5),
            Point::new(100, 115),
            Color::from_rgb888(0xA6E3A1),
        );
        p.draw_line(
            Point::new(5, 60),
            Point::new(195, 60),
            Color::from_rgb888(0xF38BA8),
        );
    }) else {
        return;
    };
    assert_close("lines", &sw, &hw, 1.5, 0.02);
}

#[test]
fn stars_and_icons_are_close() {
    let Some((sw, hw)) = both(|p| {
        p.fill_star(
            Point::new(50, 60),
            40,
            16,
            5,
            0,
            Color::from_rgb888(0xF9E2AF),
        );
        p.fill_star(
            Point::new(150, 60),
            40,
            20,
            8,
            denise::TURN / 16,
            Color::WHITE,
        );
    }) else {
        return;
    };
    // A polygon is the one shape the two paths agree on almost exactly: both
    // fill it by the even-odd rule and both measure the same distance to the
    // same outline, so the tolerance here is tighter than any other shape's.
    assert_close("stars", &sw, &hw, 0.6, 0.001);
}

#[test]
fn text_is_close() {
    let Some((sw, hw)) = both(|p| {
        let mut text = TextEngine::new();
        let style = TextStyle::built_in(16);
        text.draw(p, style, Point::new(8, 8), "Kjærlighet på Øy", Color::WHITE);
        text.draw(
            p,
            style,
            Point::new(8, 40),
            "0123456789",
            Color::from_rgb888(0x89B4FA),
        );
        let big = TextStyle::built_in(28);
        text.draw(
            p,
            big,
            Point::new(8, 70),
            "Denise",
            Color::from_rgb888(0xF5C2E7),
        );
    }) else {
        return;
    };
    // A mask is sampled nearest at 1:1, so the coverage arrives exactly and
    // only the final multiply rounds differently.
    assert_close("text", &sw, &hw, 0.5, 0.001);
}

#[test]
fn a_widget_tree_is_close() {
    use denise::{Role, theme};
    use denise_ui::Ui;
    use denise_ui::widgets::{Button, Label, Panel, Progress, RadialProgress, Rating, Toggle};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Msg {
        Go,
        Flip,
        Rate,
    }
    fn flip(_: bool) -> Msg {
        Msg::Flip
    }
    fn rate(_: f32) -> Msg {
        Msg::Rate
    }

    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };
    let size = Size::new(360, 240);

    let build = || {
        let mut ui: Ui<Msg> = Ui::new(size, theme::DARK);
        let root = ui.root();
        let card = ui
            .add(root, Panel::default(), Rect::new(10, 10, 340, 220))
            .expect("card");
        ui.add(
            card,
            Label::new("Painted twice").with_size(20),
            Rect::new(16, 12, 300, 26),
        );
        ui.add(
            card,
            Button::new("Go", Msg::Go).with_role(Role::Primary),
            Rect::new(16, 48, 90, 32),
        );
        ui.add(
            card,
            Toggle::new("Toggle", flip).with_checked(true),
            Rect::new(120, 48, 150, 32),
        );
        ui.add(card, Progress::new(0.6), Rect::new(16, 96, 200, 12));
        ui.add(
            card,
            RadialProgress::new(0.7).with_label("70%"),
            Rect::new(240, 90, 80, 80),
        );
        ui.add(card, Rating::new(3.5, rate), Rect::new(16, 124, 200, 28));
        ui
    };

    let mut software = vec![0u32; (size.width * size.height) as usize];
    {
        let mut ui = build();
        let mut frame = Frame::new(
            &mut software,
            size,
            size.width,
            PixelFormat::Argb8888,
            BufferAge::Undefined,
        )
        .expect("frame");
        ui.paint(&mut frame);
    }
    for w in &mut software {
        *w |= 0xFF00_0000;
    }

    let mut ui = build();
    let mut painter = gpu.painter(size);
    ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
    let mut hardware = painter.finish_to_pixels().expect("readback");
    for w in &mut hardware {
        *w |= 0xFF00_0000;
    }

    assert_close("widget tree", &software, &hardware, 2.0, 0.015);
}

#[test]
fn paint_is_premultiplied_on_the_way_in() {
    // Not a parity test: a guard that translucent paint reaches the shader
    // premultiplied, since the blend state assumes it.
    let p = Paint::new(Color::rgba(200, 100, 0, 128));
    assert!(p.premultiplied() & 0x00FF_0000 < 0x0065_0000);
}

// ------------------------------------------------------------ the atlas page

/// The whole point of `blit_glyph`: a text engine's page is uploaded when it
/// changes and never otherwise, so a frame of familiar text uploads nothing.
#[test]
fn an_atlas_page_is_uploaded_once_per_version() {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };
    let mut text = TextEngine::new();
    let style = TextStyle::built_in(16);

    let frame = |text: &mut TextEngine, s: &str| {
        let mut painter = gpu.painter(SIZE);
        {
            let mut pen = Pen::new(&mut painter);
            text.draw(&mut pen, style, Point::new(4, 4), s, Color::WHITE);
        }
        painter.finish_to_pixels().expect("readback");
    };

    frame(&mut text, "denise");
    assert_eq!(gpu.page_uploads(), 1, "the first frame uploads the page");

    frame(&mut text, "denise");
    frame(&mut text, "sindee");
    assert_eq!(
        gpu.page_uploads(),
        1,
        "glyphs already on the page cost no upload, in any order"
    );

    frame(&mut text, "denise + x");
    assert_eq!(
        gpu.page_uploads(),
        2,
        "a glyph not seen before repacks the page once"
    );

    frame(&mut text, "x + denise");
    assert_eq!(gpu.page_uploads(), 2, "and then it, too, is free");
}

// ------------------------------------------------------------------ images

/// The same for pictures as for glyphs: an `Image` widget's pixels go up once,
/// and only replacing them costs another upload. A clone is a new image.
#[test]
fn an_image_is_uploaded_once_per_version() {
    use denise::{Role, theme};
    use denise_ui::Ui;
    use denise_ui::widgets::{Fit, Image};

    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };
    let picture = |seed: u32| -> (Vec<u32>, Size) {
        let size = Size::new(16, 12);
        let px = (0..size.area())
            .map(|i| 0xFF00_0000 | (i as u32 * 7919 + seed))
            .collect();
        (px, size)
    };

    let mut ui: Ui<()> = Ui::new(SIZE, theme::DARK);
    let root = ui.root();
    let (px, size) = picture(1);
    let node = ui
        .add(
            root,
            Image::new(px, size).with_fit(Fit::Center),
            Rect::new(10, 10, 60, 40),
        )
        .expect("image");
    let _ = Role::Primary;

    let frame = |ui: &mut Ui<()>| {
        let mut painter = gpu.painter(SIZE);
        ui.paint_with(&mut Pen::new(&mut painter), BufferAge::Undefined);
        painter.finish_to_pixels().expect("readback");
    };

    frame(&mut ui);
    assert_eq!(
        gpu.image_uploads(),
        1,
        "the first frame uploads the picture"
    );
    frame(&mut ui);
    frame(&mut ui);
    assert_eq!(
        gpu.image_uploads(),
        1,
        "a picture already uploaded costs a quad"
    );

    let (px, size) = picture(2);
    ui.widget_mut::<Image>(node)
        .expect("still there")
        .set_pixels(px, size);
    frame(&mut ui);
    assert_eq!(
        gpu.image_uploads(),
        2,
        "replacing the pixels uploads once more"
    );
    frame(&mut ui);
    assert_eq!(gpu.image_uploads(), 2);
}

// ------------------------------------------------------------------ damage

/// The claim `finish_onto` makes: painting a scene in two passes — the whole of
/// it once, then only a damaged strip of a changed scene — lands the same
/// pixels as painting the changed scene whole. If that is not true, an
/// incremental frame shows a seam and the whole idea is unusable.
#[test]
fn an_incremental_frame_matches_a_full_one() {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };

    // A scene, and the same scene with one square moved. Only that square's
    // before-and-after is damaged.
    let scene = |pen: &mut Pen<'_>, moved: bool| {
        pen.clear(Color::from_rgb888(0x1E1E2E));
        pen.fill_rounded_rect(Rect::new(10, 10, 80, 40), 8, Color::from_rgb888(0x89B4FA));
        pen.fill_circle(Point::new(150, 80), 24, Color::from_rgb888(0xA6E3A1));
        // Over the damage, so the incremental pass redraws it clipped: a
        // polygon reads its edges out of a buffer the frame builds, and this
        // is what says a damaged frame builds the same one.
        pen.fill_star(
            Point::new(118, 74),
            22,
            9,
            5,
            0,
            Color::from_rgb888(0xF9E2AF),
        );
        let y = if moved { 70 } else { 60 };
        pen.fill_rect(Rect::new(100, y, 30, 20), Color::from_rgb888(0xF38BA8));
    };
    // The rectangle's old and new positions, with a pixel of margin.
    let damage = [Rect::new(99, 59, 32, 32)];

    // Incremental: the unmoved scene in full, then only the damage of the moved
    // one — which is what a window does every frame after the first.
    let (texture, view) = target(&gpu);
    {
        let mut painter = gpu.painter(SIZE);
        scene(&mut Pen::new(&mut painter), false);
        painter.finish(&view);
    }
    {
        let mut painter = gpu.painter(SIZE);
        {
            let mut pen = Pen::new(&mut painter);
            // Exactly what `Ui` does: clip to the damage, repaint the scene.
            let mut clipped = pen.with_clip(damage[0]);
            scene(&mut clipped, true);
        }
        painter.finish_onto(&view, &damage);
    }
    let incremental = gpu.read_texture(&texture).expect("readback");

    // And the moved scene painted whole, which is the answer it must match.
    let mut painter = gpu.painter(SIZE);
    scene(&mut Pen::new(&mut painter), true);
    let full = painter.finish_to_pixels().expect("readback");

    let s = stats(&full, &incremental);
    eprintln!("incremental vs full: mean {:.4}, max {}", s.mean, s.max);
    assert_eq!(
        s.max, 0,
        "an incremental frame differs from a full one by {} per channel",
        s.max
    );
}

/// Nothing damaged is nothing drawn: the target keeps exactly what it had.
#[test]
fn an_empty_damage_draws_nothing() {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(err) => {
            eprintln!("skipping: {err}");
            return;
        }
    };
    let (texture, view) = target(&gpu);
    {
        let mut painter = gpu.painter(SIZE);
        Pen::new(&mut painter).fill_rect(Rect::new(0, 0, 200, 120), Color::from_rgb888(0x94E2D5));
        painter.finish(&view);
    }
    let before = gpu.read_texture(&texture).expect("readback");

    let mut painter = gpu.painter(SIZE);
    Pen::new(&mut painter).clear(Color::from_rgb888(0xF38BA8));
    painter.finish_onto(&view, &[]);
    let after = gpu.read_texture(&texture).expect("readback");

    assert_eq!(
        stats(&before, &after).max,
        0,
        "an empty damage changed pixels"
    );
}
