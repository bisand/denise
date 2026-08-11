//! Proof of the abstraction, the rasteriser and the theme.
//!
//! A rounded rectangle bounces around and a translucent square tracks the pointer.
//! Neither is interesting; what is interesting is that a frame in which nothing
//! moved costs nothing, and a frame in which the rectangle moved repaints roughly
//! two rectangles' worth of pixels rather than a megapixel.
//!
//! Nothing here names a colour. Every colour comes from a semantic role, which is
//! why `T` can swap the whole palette at runtime without touching the drawing code.
//!
//! | Key | |
//! |---|---|
//! | `T` | next theme |
//! | `Space` | pause |
//! | `Esc`, `Q` | quit |
//!
//! Stats go to stderr once a second. On an idle window the damage percentage
//! should read `0.0%`; hold a key down to see it climb.
//!
//! ```text
//! cargo run -p hello-rect
//! ```

use std::time::{Duration, Instant};

use denise::theme::{Radius, Role, Theme};
use denise::{Color, DamageTracker, ElementState, Frame, InputEvent, KeyCode, Point, Rect, Size};
use denise_render::Canvas;
use denise_winit::{DeniseApp, WindowConfig, run};

const BOX_SIZE: i32 = 120;
const CURSOR_SIZE: i32 = 28;
const SPEED: i32 = 4;
/// Alpha applied to a content colour to get a border that reads as an edge rather
/// than an outline.
const BORDER_ALPHA: u8 = 72;

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
    theme: Theme,
    theme_index: usize,
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
            theme: Theme::BUILT_IN[0],
            theme_index: 0,
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
                    KeyCode::T => {
                        self.theme_index = (self.theme_index + 1) % Theme::BUILT_IN.len();
                        self.theme = Theme::BUILT_IN[self.theme_index];
                        // Every pixel on screen is now the wrong colour. This is
                        // the one case where a full repaint is the correct answer,
                        // and saying so is cheaper than tracking it.
                        damage.add_full();
                        eprintln!("theme: {}", self.theme.name);
                    }
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
        let size = frame.size();
        let mut canvas = Canvas::new(frame);

        // Repaint the damaged regions only. Clearing the whole frame here would
        // still look correct — which is exactly why damage bugs survive until they
        // reach hardware that cannot afford them.
        //
        // The scene code below is written as though it were painting the whole
        // window. The clip is what turns it into an incremental repaint, so there
        // is no second, damage-aware draw path to keep in step with this one.
        // Not one literal colour or radius below: the theme supplies every one, so
        // pressing T restyles the whole scene without this function changing.
        let theme = self.theme;
        let radius = theme.radius(Radius::Box);
        let (box_fill, box_content) = theme.pair(Role::Primary);
        let border = Color::rgba(box_content.r, box_content.g, box_content.b, BORDER_ALPHA);
        let cursor_fill = theme.color(Role::Accent).with_alpha(160);

        for region in damage {
            let mut c = canvas.with_clip(*region);
            c.clear(theme.color(Role::Base100));
            c.fill_rounded_rect(self.boxx, radius, box_fill);
            c.stroke_rounded_rect(self.boxx, radius, theme.metrics.border.max(2), border);
            if let Some(cursor) = self.cursor {
                c.fill_rounded_rect(cursor, CURSOR_SIZE / 2, cursor_fill);
            }
        }

        self.stats.record(damage, size);
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
