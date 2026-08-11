//! Shared fixtures for the benchmarks.
//!
//! The point of these benches is not to find out how fast a memory fill is. It is
//! to keep honest the one claim the whole project rests on: that a frame in which a
//! little changed costs a little. So the fixtures are a plausible UI scene at a
//! plausible panel size, drawn twice — once whole, once through damage — and the
//! interesting number is the ratio.

use denise::{Color, PixelFormat, Rect, Size};
use denise_render::Canvas;

/// A 1080p panel, the size at which damage tracking stops being optional.
pub const PANEL: Size = Size::new(1920, 1080);

/// A 7" Raspberry Pi touchscreen, the size most of these actually ship at.
pub const SMALL_PANEL: Size = Size::new(800, 480);

const BACKGROUND: Color = Color::from_rgb888(0x1E1E2E);
const PANEL_FILL: Color = Color::from_rgb888(0x313244);
const ACCENT: Color = Color::from_rgb888(0x89B4FA);
const SCRIM: Color = Color::rgba(0, 0, 0, 128);

/// An owned pixel buffer sized like a real scanout buffer.
pub struct Target {
    pixels: Vec<u32>,
    size: Size,
    stride: u32,
}

impl Target {
    /// A tightly packed buffer.
    pub fn new(size: Size) -> Self {
        Self::with_stride(size, size.width)
    }

    /// A buffer with a pitch-aligned stride, as DRM hands out.
    pub fn with_stride(size: Size, stride: u32) -> Self {
        assert!(stride >= size.width);
        Self {
            pixels: vec![0; (stride * size.height) as usize],
            size,
            stride,
        }
    }

    /// Borrows the buffer for drawing.
    pub fn canvas(&mut self) -> Canvas<'_> {
        Canvas::from_pixels(
            &mut self.pixels,
            self.size,
            self.stride,
            PixelFormat::Xrgb8888,
        )
        .expect("bench geometry fits")
    }

    /// Borrows the buffer for reading.
    pub fn view(&self) -> denise_render::PixelView<'_> {
        denise_render::PixelView::new(&self.pixels, self.size, self.stride)
            .expect("bench geometry fits")
    }

    /// Extent of the buffer.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Backing slice, padding included.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }
}

/// The widgets making up the test scene, in paint order.
pub fn scene(size: Size) -> Vec<Widget> {
    let w = size.width as i32;
    let h = size.height as i32;
    let mut widgets = Vec::new();

    // A sidebar and a row of cards: the shape of every dashboard ever shipped.
    widgets.push(Widget::Panel {
        rect: Rect::new(0, 0, w / 5, h),
        radius: 0,
    });

    let card_w = (w - w / 5 - 80) / 3;
    for i in 0..3 {
        widgets.push(Widget::Panel {
            rect: Rect::new(w / 5 + 20 + i * (card_w + 20), 20, card_w, h / 3),
            radius: 12,
        });
    }

    // Separators in the sidebar.
    for i in 0..8 {
        let y = 60 + i * 48;
        widgets.push(Widget::Rule {
            y,
            x0: 16,
            x1: w / 5 - 16,
        });
    }

    // A diagonal, so the anti-aliased line path is represented.
    widgets.push(Widget::Line {
        from: (w / 5 + 40, h / 2 + 40),
        to: (w - 40, h - 60),
    });

    widgets
}

/// One drawable in the test scene.
pub enum Widget {
    /// A card or sidebar.
    Panel {
        /// Bounds.
        rect: Rect,
        /// Corner radius; zero for a plain rectangle.
        radius: i32,
    },
    /// A one-pixel horizontal separator.
    Rule {
        /// Row.
        y: i32,
        /// Left end.
        x0: i32,
        /// Right end, exclusive.
        x1: i32,
    },
    /// An arbitrary-angle line.
    Line {
        /// Start point.
        from: (i32, i32),
        /// End point.
        to: (i32, i32),
    },
}

impl Widget {
    /// Bounds this widget touches, for damage purposes.
    pub fn bounds(&self) -> Rect {
        match self {
            Widget::Panel { rect, .. } => *rect,
            Widget::Rule { y, x0, x1 } => Rect::from_edges(*x0, *y, *x1, *y + 1),
            Widget::Line { from, to } => Rect::from_edges(
                from.0.min(to.0),
                from.1.min(to.1),
                from.0.max(to.0) + 2,
                from.1.max(to.1) + 2,
            ),
        }
    }
}

/// Paints the scene into whatever the canvas's clip currently admits.
///
/// Callers restrict the clip to a damage region; the scene code itself is unaware
/// of damage, which is exactly how a real widget tree behaves.
pub fn paint_scene(canvas: &mut Canvas<'_>, widgets: &[Widget]) {
    canvas.clear(BACKGROUND);
    for widget in widgets {
        match widget {
            Widget::Panel { rect, radius } => {
                canvas.fill_rounded_rect(*rect, *radius, PANEL_FILL);
                canvas.stroke_rounded_rect(*rect, *radius, 1, SCRIM);
            }
            Widget::Rule { y, x0, x1 } => {
                canvas.fill_rect(Rect::from_edges(*x0, *y, *x1, *y + 1), SCRIM);
            }
            Widget::Line { from, to } => {
                canvas.draw_line(
                    denise::Point::new(from.0, from.1),
                    denise::Point::new(to.0, to.1),
                    ACCENT,
                );
            }
        }
    }
}

/// A damage pattern standing in for "a couple of small things changed".
///
/// Two caret-sized rectangles and a button-sized one: about 0.4% of a 1080p panel.
pub fn typical_damage(size: Size) -> Vec<Rect> {
    let w = size.width as i32;
    let h = size.height as i32;
    vec![
        Rect::new(w / 5 + 40, 60, 2, 24),
        Rect::new(w / 5 + 40, h / 2, 160, 40),
        Rect::new(w - 220, h - 80, 180, 44),
    ]
}
