//! The same game, on a Linux machine with no desktop.
//!
//! Kept out of `main.rs` for the reason `hello` keeps its own out: none of this
//! changes what is drawn. `bare-linux` supplies the display, the input and the
//! console guard, and what remains is the loop.
//!
//! ```text
//! cargo build -p solitaire --no-default-features --features kiosk \
//!     --release --target aarch64-unknown-linux-musl
//! ```
//!
//! Run it from a text console, as root or as an account in the `video` and
//! `input` groups. The bar's Quit, Ctrl+Q, Ctrl+C and Escape leave, asking
//! first if a game is under way; F12 writes what is on the display to a file.
//!
//! # The loop sleeps
//!
//! There is no frame timer here and no poll interval. [`Waits`] blocks until an
//! input device has something to say, or until the tree asks to be woken —
//! for the next sample of a card in the air, or for the turn of the next
//! second on the bar's clock. A dealt table that nobody is touching wakes for
//! nothing at all, which is what a card game left open on a kiosk overnight
//! should cost.

use std::time::Duration;

use bare_linux::{Display, PresentMode, Waits, capture, mute_console, open_input, poll_timeout};
use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};

use crate::app::{App, Font, Options};

const SHOT_PATH: &str = "/tmp/denise-solitaire.ppm";

pub fn run(font: Font, seed: u64, turn: usize) -> Result<(), Box<dyn std::error::Error>> {
    // Vsync, as the gallery found on a Pi 3: the vc4's unpaced flips tear, and
    // a card dragged across a tear is where anybody would see it.
    let mut surface = Display::open(PresentMode::Vsync)?;
    let size = surface.size();
    let (mut input, _keymap) = open_input(size)?;
    // Held for the whole run: dropping it puts the console back as it was.
    let _console = mute_console();

    // Built at the display's size and its scale, which a panel wired straight
    // to a framebuffer reports as 1. There is no window to close here, so the
    // bar gets a Quit.
    let options = Options {
        seed,
        turn,
        quit: true,
    };
    let mut app = App::new(size, surface.scale_factor(), font, options);
    eprintln!("\ndrag the cards, or the arrows and Enter; F12 screenshots, Escape quits\n");

    // Refreshed by `wait` whenever a device is opened or closed, which is why
    // this is a `Waits` and not a list built once.
    let mut waits = Waits::new(&input);

    // The first frame, before anything is allowed to block: a loop that waits
    // before it draws puts a mode on the display and then shows black.
    app.update(&[], app.elapsed_ms());
    present(&mut surface, &mut app, false)?;

    let deadline = std::time::Instant::now() + Duration::from_secs(60 * 60 * 24);
    let mut events = Vec::new();
    let mut shoot = false;

    loop {
        let timeout = poll_timeout(app.ui.next_wake_ms(), app.elapsed_ms(), deadline);
        waits.wait(&mut input, timeout.as_ref())?;

        events.clear();
        input.poll(&mut events);
        for event in &events {
            if let InputEvent::Key {
                code: KeyCode::F12,
                state: ElementState::Down,
                ..
            } = event
            {
                shoot = true;
            }
        }

        // Everything else — Escape and the chords among it — is the
        // application's to read, so that it is read the same way in a window.
        app.update(&events, app.elapsed_ms());
        if app.exit {
            return Ok(());
        }

        if app.ui.needs_paint() || shoot {
            present(&mut surface, &mut app, core::mem::take(&mut shoot))?;
        }
    }
}

/// Paints and presents one frame, capturing it first if one was asked for.
///
/// `acquire` hands back a buffer with an age, and `Ui::paint` widens this
/// frame's damage by what that buffer missed — so on a display flipping
/// between two buffers a carried card repaints four card-sized rectangles, not
/// two, and still not the table.
fn present(
    surface: &mut Display,
    app: &mut App,
    shoot: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut frame = surface.acquire()?;
    app.ui.paint(&mut frame);
    if shoot {
        match capture(&frame, SHOT_PATH) {
            Ok(()) => eprintln!("wrote {SHOT_PATH}"),
            Err(e) => eprintln!("could not write {SHOT_PATH}: {e}"),
        }
    }
    drop(frame);
    surface.present(app.ui.damage())?;
    app.ui.presented();
    Ok(())
}
