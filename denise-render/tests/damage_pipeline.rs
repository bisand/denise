//! End-to-end proof that incremental repaint produces the same pixels as a full
//! repaint, against a swapchain that behaves like real hardware.
//!
//! The bug this exists to catch: with N buffers, the one you are handed is N-1
//! frames stale, so repainting only *this* frame's damage leaves the frame before
//! last's pixels on screen. It looks fine at a glance and flickers at the refresh
//! rate. It is also invisible on a development backend with a persistent shadow
//! buffer, which is precisely why it survives to the target hardware.
//!
//! The swapchain here uses a padded stride too, so an implementation that assumes
//! `stride == width` fails these tests rather than the Pi.

use denise::{
    BufferAge, Color, DamageTracker, Frame, MAX_DAMAGE_RECTS, PixelFormat, Rect, Size, Surface,
    SurfaceError,
};
use denise_render::Canvas;

/// Extra words per row, so nothing may assume rows are contiguous.
const STRIDE_PADDING: u32 = 7;

/// An in-memory N-buffered swapchain that reports honest buffer ages.
struct Swapchain {
    buffers: Vec<Vec<u32>>,
    /// Frame index at which each buffer was last presented; `None` if never.
    last_presented: Vec<Option<u64>>,
    size: Size,
    stride: u32,
    frame: u64,
    current: usize,
    acquired: bool,
}

impl Swapchain {
    fn new(size: Size, count: usize) -> Self {
        let stride = size.width + STRIDE_PADDING;
        let len = stride as usize * size.height as usize;
        Self {
            // Fill with a colour that appears nowhere in the scene, so any pixel
            // never written shows up as a mismatch rather than passing by luck.
            buffers: vec![vec![0x00DE_AD00; len]; count],
            last_presented: vec![None; count],
            size,
            stride,
            frame: 0,
            current: 0,
            acquired: false,
        }
    }

    /// The visible pixels of the most recently presented buffer, padding removed.
    fn presented(&self) -> Vec<u32> {
        let idx = (self.current + self.buffers.len() - 1) % self.buffers.len();
        self.buffers[idx]
            .chunks(self.stride as usize)
            .take(self.size.height as usize)
            .flat_map(|row| &row[..self.size.width as usize])
            .copied()
            .collect()
    }
}

impl Surface for Swapchain {
    fn size(&self) -> Size {
        self.size
    }

    fn scale_factor(&self) -> f32 {
        1.0
    }

    fn format(&self) -> PixelFormat {
        PixelFormat::Xrgb8888
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        assert!(!self.acquired, "acquire without an intervening present");
        self.acquired = true;

        // We are rendering what will become frame `self.frame + 1`. A buffer last
        // presented as frame `then` therefore holds contents that many frames back.
        // Getting this off by one makes single buffering report age 0, which reads
        // as "undefined" and quietly turns the whole test into a full repaint.
        let age = match self.last_presented[self.current] {
            Some(then) => BufferAge::Frames((self.frame + 1 - then) as u32),
            None => BufferAge::Undefined,
        };

        Frame::new(
            &mut self.buffers[self.current],
            self.size,
            self.stride,
            PixelFormat::Xrgb8888,
            age,
        )
    }

    fn present(&mut self, _damage: &[Rect]) -> Result<(), SurfaceError> {
        assert!(self.acquired, "present without an acquire");
        self.acquired = false;
        self.frame += 1;
        self.last_presented[self.current] = Some(self.frame);
        self.current = (self.current + 1) % self.buffers.len();
        Ok(())
    }
}

/// A deterministic scene: a box bouncing over a background, plus a blinking dot
/// that appears and disappears rather than moving, to exercise disjoint damage.
struct Scene {
    bounds: Size,
    boxx: Rect,
    velocity: (i32, i32),
    dot: Option<Rect>,
    tick: u32,
}

const BACKGROUND: Color = Color::from_rgb888(0x101018);
const BOX_COLOR: Color = Color::from_rgb888(0xF5A9B8);
const BORDER_COLOR: Color = Color::rgba(255, 255, 255, 96);
const DOT_COLOR: Color = Color::from_rgb888(0x89B4FA);

impl Scene {
    fn new(bounds: Size) -> Self {
        Self {
            bounds,
            boxx: Rect::new(3, 5, 37, 23),
            velocity: (3, 2),
            dot: None,
            tick: 0,
        }
    }

    fn step(&mut self, damage: &mut DamageTracker) {
        self.tick += 1;
        let w = self.bounds.width as i32;
        let h = self.bounds.height as i32;

        let (mut dx, mut dy) = self.velocity;
        if self.boxx.x + dx < 0 || self.boxx.x + dx + self.boxx.width > w {
            dx = -dx;
        }
        if self.boxx.y + dy < 0 || self.boxx.y + dy + self.boxx.height > h {
            dy = -dy;
        }
        self.velocity = (dx, dy);

        damage.add(self.boxx);
        self.boxx = self.boxx.translate(dx, dy);
        damage.add(self.boxx);

        // Blink a dot in the far corner every seventh frame, well away from the
        // box, so the damage list genuinely holds two disjoint regions.
        if self.tick.is_multiple_of(7) {
            let next = match self.dot {
                Some(old) => {
                    damage.add(old);
                    None
                }
                None => Some(Rect::new(w - 9, h - 9, 6, 6)),
            };
            self.dot = next;
            if let Some(new) = next {
                damage.add(new);
            }
        }
    }

    /// Paints the scene, clipped to `region`. This is the only draw path; a full
    /// repaint is just this with `region` set to the whole surface.
    ///
    /// The box is a rounded rectangle with a translucent border, so the comparison
    /// against a full repaint covers anti-aliased coverage and alpha blending, not
    /// just solid fills. Those are where an incremental repaint goes subtly wrong:
    /// a partially covered pixel composited once looks nothing like one composited
    /// twice, and only a pixel-exact reference catches it.
    fn paint(&self, frame: &mut Frame<'_>, region: &[Rect]) {
        let mut canvas = Canvas::new(frame);
        for clip in region {
            let mut c = canvas.with_clip(*clip);
            c.clear(BACKGROUND);
            c.fill_rounded_rect(self.boxx, 6, BOX_COLOR);
            c.stroke_rounded_rect(self.boxx, 6, 2, BORDER_COLOR);
            if let Some(dot) = self.dot {
                c.fill_rect(dot, DOT_COLOR);
            }
        }
    }
}

/// Renders `scene` with a full repaint into a tightly packed buffer.
fn reference(scene: &Scene, size: Size) -> Vec<u32> {
    let mut buf = vec![0u32; size.area() as usize];
    let mut frame = Frame::new(
        &mut buf,
        size,
        size.width,
        PixelFormat::Xrgb8888,
        BufferAge::Undefined,
    )
    .expect("reference geometry fits");
    scene.paint(&mut frame, &[Rect::from_size(size)]);
    drop(frame);
    buf
}

/// Drives `frames` frames through the damage pipeline and asserts that every
/// presented buffer matches a full repaint of the same scene, pixel for pixel.
fn run_pipeline(size: Size, buffer_count: usize, frames: u32) {
    let mut swapchain = Swapchain::new(size, buffer_count);
    let mut tracker = DamageTracker::new(size);
    let mut scene = Scene::new(size);

    for n in 0..frames {
        scene.step(&mut tracker);

        let mut frame = swapchain.acquire().expect("acquire");

        let mut resolved = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let src = tracker.resolve(frame.age());
            resolved[..src.len()].copy_from_slice(src);
            src.len()
        };
        let region = &resolved[..count];

        scene.paint(&mut frame, region);
        drop(frame);

        swapchain.present(region).expect("present");
        tracker.end_frame();

        let actual = swapchain.presented();
        let expected = reference(&scene, size);
        assert_eq!(
            actual.len(),
            expected.len(),
            "buffer geometry diverged at frame {n}"
        );
        if let Some(i) = actual.iter().zip(&expected).position(|(a, b)| a != b) {
            let (x, y) = (i % size.width as usize, i / size.width as usize);
            panic!(
                "frame {n} ({buffer_count} buffers): stale pixel at ({x}, {y}): \
                 presented {:#010x}, expected {:#010x}",
                actual[i], expected[i]
            );
        }
    }
}

#[test]
fn single_buffered_is_correct() {
    run_pipeline(Size::new(160, 100), 1, 200);
}

#[test]
fn double_buffered_is_correct() {
    run_pipeline(Size::new(160, 100), 2, 200);
}

#[test]
fn triple_buffered_is_correct() {
    run_pipeline(Size::new(160, 100), 3, 200);
}

#[test]
fn swapchain_deeper_than_tracked_history_is_correct() {
    // More buffers than the tracker keeps history for. It must fall back to full
    // repaints rather than producing wrong pixels.
    run_pipeline(
        Size::new(160, 100),
        denise::damage::MAX_TRACKED_FRAMES + 2,
        200,
    );
}

#[test]
fn incremental_repaint_actually_saves_work() {
    // A correctness harness that silently full-repaints every frame would pass all
    // of the above. Pin down that it does not.
    let size = Size::new(160, 100);
    let mut tracker = DamageTracker::new(size);
    let mut scene = Scene::new(size);
    let mut swapchain = Swapchain::new(size, 2);

    let mut damaged = 0u64;
    let frames: u32 = 200;

    for _ in 0..frames {
        scene.step(&mut tracker);
        let mut frame = swapchain.acquire().expect("acquire");
        let mut resolved = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let src = tracker.resolve(frame.age());
            resolved[..src.len()].copy_from_slice(src);
            src.len()
        };
        let region = &resolved[..count];
        damaged += region.iter().map(Rect::area).sum::<u64>();
        scene.paint(&mut frame, region);
        drop(frame);
        swapchain.present(region).expect("present");
        tracker.end_frame();
    }

    let full = size.area() * u64::from(frames);
    let ratio = damaged as f64 / full as f64;
    assert!(
        ratio < 0.35,
        "expected incremental repaint to touch well under a third of the surface, got {:.1}%",
        ratio * 100.0
    );
}
