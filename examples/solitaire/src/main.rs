//! Klondike, to show what damage tracking is for.
//!
//! ```text
//! cargo run -p solitaire
//! cargo run -p solitaire -- --three                 # turn three cards, not one
//! cargo run -p solitaire -- --seed 7                # the same deals every time
//! cargo run -p solitaire -- --font /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
//! cargo run -p solitaire -- --snapshot table.ppm
//! cargo run -p solitaire -- --snapshot table.ppm 2  # at a 2x scale factor
//! cargo run -p solitaire --release -- --bench
//! cargo run -p solitaire --no-default-features --features kiosk    # the display
//! ```
//!
//! Drag the cards, or double-click one to send it to its foundation. The stock
//! turns when pressed. From the keyboard: the arrows move between piles, Enter
//! picks up and puts down, Up and Down on a picked-up pile take more cards or
//! fewer, Space turns the stock, `H` sends a card home, `U` or Backspace takes
//! a move back, `N` deals again, `T` changes between turning one card and
//! three, `B` changes the backs. Escape puts down what was picked up, and then
//! leaves — asking first, if there is a game to lose.
//!
//! # Why a card game is in a toolkit's examples
//!
//! Because it is the hardest ordinary thing to draw cheaply. A table is forty
//! pictures laid over one another; a move changes two small parts of it; and
//! while a card is being carried, something the size of a card moves on every
//! report of the mouse. Drawn the simple way — paint the table when anything
//! changes — that is a megapixel of pictures per frame, and on a Raspberry Pi 3
//! it is a game that thinks about every move.
//!
//! Denise repaints what changed. This example is that sentence made playable,
//! and `--bench` prints what it is worth. On a Raspberry Pi 3A+ at 1920×1080,
//! into a buffer two frames old as the display's are:
//!
//! ```text
//! the table painted whole     20.0 ms    100% of the surface
//! a card carried               3.3 ms    3.3%
//! a second on the clock        0.1 ms    0.4%
//! ```
//!
//! A frame at 60 Hz is 16.7 ms. The first line misses it and the second fits
//! in it five times, with the same game and the same rasteriser: the whole of
//! the difference is not painting what did not change. And nothing in this
//! example computes a damage rectangle to get it — see `table.rs` for where
//! the rectangles come from instead.
//!
//! # Where to read
//!
//! - `table.rs` — the three widgets the toolkit does not have, and **why a
//!   pile is a widget and the table is not**. Start there.
//! - `app.rs` — the tree, the messages, and the one function that decides what
//!   is repainted by comparing what each pile shows with what it should.
//! - `layout.rs` — where everything goes. No widget in it, so all of it is
//!   tested without a display.
//! - `game.rs`, `cards.rs` — the rules. Nothing of Denise in them.
//! - `faces.rs` — the pictures, decoded by `denise-image` and scaled once.
//! - `kiosk.rs` — the same application on a display with no desktop.
//!
//! This file holds the command line, the window, the snapshot and the bench.

mod app;
mod cards;
mod faces;
mod game;
mod layout;
mod table;

#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk;

use std::time::Instant;

use app::{App, Font, Options};
use denise::{BufferAge, Frame, InputEvent, PixelFormat, Point, Rect, Size};

/// The window, in logical pixels: room for seven cards a comfortable size.
const WINDOW: Size = Size::new(1100, 760);

const USAGE: &str = "\
usage: solitaire [--three] [--seed N] [--font PATH]
       solitaire --snapshot OUT.ppm [SCALE]
       solitaire --bench

  --three      turn three cards from the stock at a time, not one
  --seed N     deal from this seed: the same games in the same order
  --font PATH  write in this face and not the best one found
  --snapshot   draw one frame of a dealt table into a PPM and exit; no display
  --bench      print what a frame costs on this machine and exit; no display";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut font: Option<String> = None;
    let mut snapshot: Option<(String, f32)> = None;
    let mut bench = false;
    let mut turn = 1;
    // No two evenings alike, and nothing to keep secret: the clock will do.
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--three" => turn = 3,
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(seed),
            "--font" => font = args.next(),
            // `--snapshot out.ppm 2` draws the same table at a 2x scale
            // factor, as `hello` does: the surface grows, and the bar and the
            // text grow with it through the one multiply in `Layout::new`.
            "--snapshot" => {
                let path = args.next().unwrap_or_else(|| "solitaire.ppm".into());
                let scale = args.next_if(|s| s.parse::<f32>().is_ok());
                let scale = scale.and_then(|s| s.parse().ok()).unwrap_or(1.0);
                snapshot = Some((path, scale));
            }
            "--bench" => bench = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument {other}\n\n{USAGE}");
                return Ok(());
            }
        }
    }

    if bench {
        bench_run(system_font::load(font.as_deref()));
        return Ok(());
    }
    if let Some((path, scale)) = snapshot {
        let font = system_font::load(font.as_deref());
        return write_snapshot(&path, scale, font, turn).map_err(Into::into);
    }

    let font = system_font::load(font.as_deref());
    backend::run(font, seed, turn)
}

// Exactly one backend; the rule is table-editor's, written out there.
#[cfg(all(feature = "kiosk", target_os = "linux"))]
use kiosk as backend;

#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
use window as backend;

#[cfg(not(any(all(feature = "kiosk", target_os = "linux"), feature = "desktop")))]
compile_error!(
    "solitaire has no backend to draw with. Enable `desktop` for a window, or \
     `kiosk` for the display itself, which needs Linux."
);

/// A window, on any desktop.
#[cfg(all(feature = "desktop", not(all(feature = "kiosk", target_os = "linux"))))]
mod window {
    use std::time::Duration;

    use super::{App, Font, Options, WINDOW};
    use denise::{DamageTracker, Frame, InputEvent, Rect};
    use denise_winit::{DeniseApp, WindowConfig, run_with};

    pub fn run(font: Font, seed: u64, turn: usize) -> Result<(), Box<dyn std::error::Error>> {
        run_with(
            WindowConfig {
                title: "Denise — solitaire".into(),
                // Logical: the same apparent table on a Pi and on a Retina Mac.
                size: WINDOW,
                ..WindowConfig::default()
            },
            // The surface and its scale factor, at the first moment both are
            // known. A window has a close button, so the bar offers no Quit.
            move |surface, scale| {
                let options = Options {
                    seed,
                    turn,
                    quit: false,
                };
                let mut app = App::new(surface, scale, font, options);
                // The window system already draws a pointer; the tree must not
                // draw a second one over it.
                app.ui.show_cursor(false);
                Window { app }
            },
        )?;
        Ok(())
    }

    struct Window {
        app: App,
    }

    impl DeniseApp for Window {
        fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
            let now = self.app.elapsed_ms();
            self.app.update(events, now);

            // The tree's own rectangles, not `add_full`: the two a carried
            // card left and reached are what is copied to the window, the same
            // two that were painted. An empty list with `needs_paint` set
            // means the tree wants everything.
            if self.app.ui.needs_paint() {
                let pending = self.app.ui.pending_damage();
                if pending.is_empty() {
                    damage.add_full();
                } else {
                    for rect in pending {
                        damage.add(*rect);
                    }
                }
            }
        }

        fn render(&mut self, frame: &mut Frame<'_>, _damage: &[Rect]) {
            self.app.ui.paint(frame);
            self.app.ui.presented();
        }

        fn exit_requested(&self) -> bool {
            self.app.exit
        }

        /// A card in the air wants the tree's animation rate; the clock on the
        /// bar wants the turn of the next second; a table nobody is playing at
        /// wants nothing. All three are this one answer.
        fn next_frame_in(&self) -> Option<Duration> {
            let now = self.app.elapsed_ms();
            self.app
                .ui
                .next_wake_ms()
                .map(|wake| Duration::from_millis(wake.saturating_sub(now)))
        }
    }
}

/// A surface in memory, standing in for a display: what the snapshot is drawn
/// into, and what the bench paints frame after frame.
struct Memory {
    pixels: Vec<u32>,
    size: Size,
}

impl Memory {
    fn new(size: Size) -> Self {
        Self {
            pixels: vec![0; (size.width * size.height) as usize],
            size,
        }
    }

    /// Paints whatever the tree has pending and retires it, as a backend does.
    /// Returns how many pixels that was.
    fn paint(&mut self, app: &mut App, age: BufferAge) -> u64 {
        if !app.ui.needs_paint() {
            return 0;
        }
        let mut frame = Frame::new(
            &mut self.pixels,
            self.size,
            self.size.width,
            PixelFormat::Xrgb8888,
            age,
        )
        .expect("frame");
        app.ui.paint(&mut frame);
        drop(frame);
        // What `paint` resolved, which is what it covered: this frame's
        // damage, and with a buffer two frames old the last frame's too.
        let painted = app.ui.damage().iter().map(Rect::area).sum();
        app.ui.presented();
        painted
    }
}

/// Draws one frame of a dealt table into a file, with no window and no event
/// loop.
fn write_snapshot(path: &str, scale: f32, font: Font, turn: usize) -> std::io::Result<()> {
    use std::io::Write as _;

    let size = Size::new(
        (WINDOW.width as f32 * scale + 0.5) as u32,
        (WINDOW.height as f32 * scale + 0.5) as u32,
    );
    // Always the same deal, so that two snapshots differ only by what changed
    // in the code between them.
    let options = Options {
        seed: 1987,
        turn,
        quit: false,
    };
    let mut app = App::new(size, scale, font, options);
    app.update(&[], 0);
    let mut memory = Memory::new(size);
    memory.paint(&mut app, BufferAge::Undefined);

    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", size.width, size.height)?;
    for word in &memory.pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path} at {}x{}", size.width, size.height);
    Ok(())
}

/// One line of the bench: how long each frame took, and how much it painted.
#[derive(Default)]
struct Measured {
    ms: Vec<f64>,
    pixels: Vec<u64>,
}

impl Measured {
    fn print(mut self, what: &str, surface: u64) {
        self.ms.sort_by(f64::total_cmp);
        self.pixels.sort_unstable();
        let median = self.ms.get(self.ms.len() / 2).copied().unwrap_or(0.0);
        let worst = self.ms.last().copied().unwrap_or(0.0);
        let pixels = self.pixels.get(self.pixels.len() / 2).copied().unwrap_or(0);
        println!(
            "  {what:<32} {median:>8.2} ms   worst {worst:>8.2}   {:>5.1}% of the surface",
            pixels as f64 * 100.0 / surface.max(1) as f64
        );
    }
}

/// The game, a surface in memory to paint it on, and a clock to advance.
struct Bench {
    app: App,
    memory: Memory,
    now: u64,
}

impl Bench {
    /// One whole frame, sixteen milliseconds after the last: events in, the
    /// tree brought up to date, the damage painted. How long, and how much.
    ///
    /// The buffer is two frames old every time, as on a display that flips
    /// between two, so a frame repaints its own damage and the frame before's.
    fn frame(&mut self, events: &[InputEvent]) -> (f64, u64) {
        self.now += 16;
        let started = Instant::now();
        self.app.update(events, self.now);
        let painted = self.memory.paint(&mut self.app, BufferAge::Frames(2));
        (started.elapsed().as_secs_f64() * 1000.0, painted)
    }

    /// `frames` frames, each given the events `each` makes for it.
    fn measure(&mut self, frames: usize, mut each: impl FnMut(usize) -> InputEvent) -> Measured {
        let mut measured = Measured::default();
        for n in 0..frames {
            let (ms, pixels) = self.frame(&[each(n)]);
            measured.ms.push(ms);
            measured.pixels.push(pixels);
        }
        measured
    }
}

/// Times whole frames with no display attached.
///
/// ```text
/// solitaire --bench
/// ```
///
/// Nothing here waits for a vblank or reads an input device, so the numbers
/// are what the *application* costs — the game, the tree and the rasteriser —
/// at 1920×1080, painting into a buffer two frames old: the honest price on a
/// display that flips between two buffers, and twice what a window pays.
///
/// What to read off it: the second line is what painting the table costs, and
/// every line after it is what playing costs. The distance between them is the
/// toolkit.
fn bench_run(font: Font) {
    use denise::{ElementState, KeyCode, Modifiers, PointerButton};

    let size = Size::new(1920, 1080);
    let surface = size.area();
    let options = Options {
        seed: 1,
        turn: 1,
        quit: true,
    };
    let key = |code: KeyCode| InputEvent::Key {
        code,
        state: ElementState::Down,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    let button = |state: ElementState, position: Point| InputEvent::PointerButton {
        button: PointerButton::Left,
        state,
        position,
        modifiers: Modifiers::NONE,
    };
    let moved = |x: i32, y: i32| InputEvent::PointerMoved {
        position: Point::new(x, y),
    };

    // The first table: every picture on it decoded and scaled, and painted.
    let started = Instant::now();
    let mut bench = Bench {
        app: App::new(size, 1.0, font, options),
        memory: Memory::new(size),
        now: 0,
    };
    bench.app.update(&[], 0);
    bench.memory.paint(&mut bench.app, BufferAge::Undefined);
    let first = started.elapsed().as_secs_f64() * 1000.0;

    println!(
        "solitaire --bench, at {}x{}, in memory",
        size.width, size.height
    );
    println!(
        "  {:<32} {first:>8.2} ms",
        "the first table, pictures scaled"
    );

    // The table painted whole, the pictures already scaled: what every frame
    // would cost if a frame were the table.
    let mut whole = Measured::default();
    for _ in 0..12 {
        // Asked for from outside, which no event in a game does: a theme
        // change would.
        bench.app.ui.invalidate_all();
        let started = Instant::now();
        let painted = bench.memory.paint(&mut bench.app, BufferAge::Frames(2));
        whole.ms.push(started.elapsed().as_secs_f64() * 1000.0);
        whole.pixels.push(painted);
    }
    whole.print("the table painted whole", surface);

    // The last pile's top card, picked up and carried across the table.
    let grab = bench.app.top_card(6);
    bench.frame(&[button(ElementState::Down, grab)]);
    bench
        .measure(200, |n| moved(grab.x - n as i32 * 7, grab.y - n as i32 * 2))
        .print("a card carried", surface);
    bench.frame(&[button(ElementState::Up, Point::new(-50, -50))]);
    bench.frame(&[]);

    // The pointer alone, across the seven piles. The tree repaints a node the
    // pointer enters or leaves, so this is the price of hover on a widget that
    // draws nothing for it: most frames nothing but the sprite, and a pile
    // each time an edge is crossed, which is the worst case beside it.
    bench
        .measure(200, |n| moved(120 + n as i32 * 9, grab.y))
        .print("the pointer across the piles", surface);
    // And over the open table, where it crosses nothing: the pointer's sprite.
    let foot = size.height as i32 - 12;
    bench
        .measure(200, |n| moved(120 + n as i32 * 9, foot))
        .print("the pointer across open table", surface);

    // The keyboard's ring, walked along the piles and back.
    bench.frame(&[key(KeyCode::ArrowDown)]);
    bench
        .measure(60, |n| {
            key(if (n / 6) % 2 == 0 {
                KeyCode::ArrowRight
            } else {
                KeyCode::ArrowLeft
            })
        })
        .print("the keyboard's ring moved", surface);

    // The stock turned, card after card: the waste, and the count on the bar.
    // Dearer than its pixels, because every turn here shows a card for the
    // first time and its picture is decoded and scaled on the spot. The second
    // time round the stock it is the copy alone.
    bench
        .measure(24, |_| key(KeyCode::Space))
        .print("the stock turned", surface);

    // The clock: a second passing with nothing else happening.
    let mut ticks = Measured::default();
    for _ in 0..20 {
        bench.now += 1000 - 16;
        let (ms, pixels) = bench.frame(&[]);
        ticks.ms.push(ms);
        ticks.pixels.push(pixels);
    }
    ticks.print("a second on the clock", surface);
}
