//! The same application, on a Linux machine with no desktop.
//!
//! Line for line what `examples/hello/src/kiosk.rs` is, which is the point: the
//! form changed where the tree comes from and changed nothing about the machine.
//!
//! Kept out of `main.rs` on purpose. That file is meant to be read in one sitting
//! and to be about a tree; this one is about a machine, and none of it changes
//! what is drawn. `bare-linux` supplies the display, the input and the console
//! guard, and what remains is the loop.
//!
//! ```text
//! cargo build -p designed --no-default-features --features kiosk \
//!     --release --target aarch64-unknown-linux-musl
//! ```

use std::time::Duration;

use bare_linux::{Display, PresentMode, Waits, capture, mute_console, open_input, poll_timeout};
use denise::{ElementState, InputEvent, InputSource, KeyCode, Surface};

use crate::{Designed, Message};

const SHOT_PATH: &str = "/tmp/denise-designed.ppm";

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut surface = Display::open(PresentMode::Vsync)?;
    let size = surface.size();
    let (mut input, _keymap) = open_input(size)?;
    // Held for the whole run: dropping it puts the console back as it was.
    let _console = mute_console();

    // Built at the display's size. The form says 460x260 and the panel is
    // whatever it is, so `Designed::new` centres the one in the other — the same
    // thing `hello` does, and the reason a form file carries a design size
    // rather than a promise about the display. Scale factor 1, because a kiosk
    // panel is designed at its display's native resolution.
    let mut app = Designed::new(size, 1.0);
    eprintln!("\ntype a name and press Enter, F12 screenshots, Escape quits\n");

    // Refreshed by `wait` whenever a device is opened or closed, which is
    // why this is a `Waits` and not a list built once.
    let mut waits = Waits::new(&input);

    // The first frame, before anything is allowed to block. `poll` waits on input
    // and on whatever the tree wants waking for, and a loop that waits before it
    // draws puts a mode on the display and then shows black.
    present(&mut surface, &mut app, false)?;

    let deadline = std::time::Instant::now() + Duration::from_secs(60 * 60 * 24);
    let mut events = Vec::new();
    let mut shoot = false;

    loop {
        let now = app.started.elapsed().as_millis() as u64;
        let timeout = poll_timeout(app.ui.next_wake_ms(), now, deadline);
        waits.wait(&mut input, timeout.as_ref())?;

        events.clear();
        input.poll(&mut events);
        for event in &events {
            if let InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            } = event
            {
                match code {
                    KeyCode::Escape => return Ok(()),
                    KeyCode::F12 => shoot = true,
                    _ => {}
                }
            }
        }

        app.ui.handle(&events);
        app.ui.tick(now);
        // The same three lines as the window backend, because this is the same
        // application: drain what the tree emitted, act on it, draw if anything
        // changed.
        let messages: Vec<Message> = app.ui.drain_messages().collect();
        for message in messages {
            match message {
                Message::Greet => app.greet(),
            }
        }

        if app.ui.needs_paint() || shoot {
            present(&mut surface, &mut app, core::mem::take(&mut shoot))?;
        }
    }
}

/// Paints and presents one frame, capturing it first if one was asked for.
///
/// The capture happens between painting and presenting, so what lands in the file
/// is what the display is about to show rather than a re-render of it.
fn present(
    surface: &mut Display,
    app: &mut Designed,
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
