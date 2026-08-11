//! M0 proof of the abstraction.
//!
//! A rectangle bounces around a dark background and a small square tracks the
//! pointer. Neither is interesting; what is interesting is that a frame in which
//! nothing moved costs nothing, and a frame in which the rectangle moved repaints
//! roughly two rectangles' worth of pixels rather than a megapixel.
//!
//! Stats go to stderr once a second. On an idle window the damage percentage
//! should read `0.0%`; hold a key down to see it climb.
//!
//! ```text
//! cargo run -p hello-rect
//! ```

use std::time::{Duration, Instant};

use denise::{
    Color, DamageTracker, ElementState, Frame, InputEvent, KeyCode, Point, Rect, Size, paint,
};
use denise_winit::{DeniseApp, WindowConfig, run};

const BACKGROUND: Color = Color::from_rgb888(0x1E1E2E);
const BOX_COLOR: Color = Color::from_rgb888(0xF5A9B8);
const CURSOR_COLOR: Color = Color::from_rgb888(0x89B4FA);
const BOX_SIZE: i32 = 120;
const CURSOR_SIZE: i32 = 24;
const SPEED: i32 = 4;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = WindowConfig {
        title: "Denise — hello-rect".into(),
        size: Size::new(800, 480),
        ..WindowConfig::default()
    };
    run(config, HelloRect::new())?;
    Ok(())
}

struct HelloRect {
    surface: Size,
    boxx: Rect,
    velocity: (i32, i32),
    cursor: Option<Rect>,
    paused: bool,
    exit: bool,
    stats: Stats,
}

impl HelloRect {
    fn new() -> Self {
        Self {
            surface: Size::ZERO,
            boxx: Rect::new(40, 40, BOX_SIZE, BOX_SIZE),
            velocity: (SPEED, SPEED),
            cursor: None,
            paused: false,
            exit: false,
            stats: Stats::new(),
        }
    }

    fn advance(&mut self, damage: &mut DamageTracker) {
        if self.paused || self.surface.is_empty() {
            return;
        }

        let bounds = Rect::from_size(self.surface);
        let (mut dx, mut dy) = self.velocity;

        let next_x = self.boxx.x + dx;
        if next_x < 0 || next_x + self.boxx.width > bounds.width {
            dx = -dx;
        }
        let next_y = self.boxx.y + dy;
        if next_y < 0 || next_y + self.boxx.height > bounds.height {
            dy = -dy;
        }
        self.velocity = (dx, dy);

        let old = self.boxx;
        self.boxx = self.boxx.translate(dx, dy);

        // Both the vacated and the newly covered region are dirty. The tracker
        // merges them into one rectangle when they overlap, which at four pixels a
        // frame they always do.
        damage.add(old);
        damage.add(self.boxx);
    }

    fn move_cursor(&mut self, position: Point, damage: &mut DamageTracker) {
        let next = Rect::new(
            position.x - CURSOR_SIZE / 2,
            position.y - CURSOR_SIZE / 2,
            CURSOR_SIZE,
            CURSOR_SIZE,
        );
        if let Some(old) = self.cursor.replace(next) {
            damage.add(old);
        }
        damage.add(next);
    }
}

impl DeniseApp for HelloRect {
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        for event in events {
            match event {
                InputEvent::CloseRequested => self.exit = true,

                InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    ..
                } => match code {
                    KeyCode::Escape | KeyCode::Q => self.exit = true,
                    KeyCode::Space => self.paused = !self.paused,
                    _ => {}
                },

                InputEvent::SurfaceResized { size, .. } => {
                    self.surface = *size;
                    // The tracker has already invalidated its history; clamp the box
                    // back inside the new bounds so it cannot get stranded offscreen.
                    let bounds = Rect::from_size(*size);
                    self.boxx.x = self.boxx.x.min(bounds.width - self.boxx.width).max(0);
                    self.boxx.y = self.boxx.y.min(bounds.height - self.boxx.height).max(0);
                }

                InputEvent::PointerMoved { position } => self.move_cursor(*position, damage),

                InputEvent::PointerLeft => {
                    if let Some(old) = self.cursor.take() {
                        damage.add(old);
                    }
                }

                _ => {}
            }
        }

        self.advance(damage);
    }

    fn render(&mut self, frame: &mut Frame<'_>, damage: &[Rect]) {
        // Repaint the damaged region only. Clearing the whole frame here would
        // still look correct — which is exactly why damage bugs survive until they
        // reach hardware that cannot afford them.
        paint::fill_rects(frame, damage, BACKGROUND);

        for region in damage {
            if let Some(part) = self.boxx.intersect(region) {
                paint::fill_rect(frame, part, BOX_COLOR);
            }
            if let Some(cursor) = self.cursor
                && let Some(part) = cursor.intersect(region)
            {
                paint::fill_rect(frame, part, CURSOR_COLOR);
            }
        }

        self.stats.record(damage, frame.size());
    }

    fn exit_requested(&self) -> bool {
        self.exit
    }
}

/// Frame and damage accounting, printed once a second.
struct Stats {
    last_report: Instant,
    frames: u32,
    damaged_pixels: u64,
    surface_pixels: u64,
}

impl Stats {
    fn new() -> Self {
        Self {
            last_report: Instant::now(),
            frames: 0,
            damaged_pixels: 0,
            surface_pixels: 0,
        }
    }

    fn record(&mut self, damage: &[Rect], size: Size) {
        self.frames += 1;
        self.damaged_pixels += damage.iter().map(Rect::area).sum::<u64>();
        self.surface_pixels += size.area();

        let elapsed = self.last_report.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }

        let fps = f64::from(self.frames) / elapsed.as_secs_f64();
        let ratio = if self.surface_pixels == 0 {
            0.0
        } else {
            self.damaged_pixels as f64 / self.surface_pixels as f64 * 100.0
        };
        eprintln!(
            "{fps:5.1} fps   {ratio:5.1}% of the surface repainted   ({}×{})",
            size.width, size.height
        );

        self.last_report = Instant::now();
        self.frames = 0;
        self.damaged_pixels = 0;
        self.surface_pixels = 0;
    }
}
