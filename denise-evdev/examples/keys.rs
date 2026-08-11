//! Shows what every key press produces: position, then character.
//!
//! The diagnostic for "my keyboard types the wrong thing". Each key reports the
//! [`KeyCode`](denise::KeyCode) — the *position*, named after the US layout — and
//! any text the layout composed from it is printed underneath, indented. A wrong
//! layout and a wrong key are then immediately distinguishable: the position is a
//! fact about the hardware, the text is a fact about the layout.
//!
//! Read-only and display-free: it touches no DRM device and sets no mode, so it is
//! safe to run over SSH while looking at the console.
//!
//! ```text
//! cargo run -p denise-evdev --example keys -- [seconds] [layout]
//! /tmp/keys 30 no
//! ```
//!
//! A dead key prints a position and no text at all until the next key resolves it.
//! That is correct, and it is also exactly what a broken keyboard looks like if
//! you are not expecting it.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("this needs Linux and evdev");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    use denise::{ElementState, InputEvent, InputSource, KeyCode, Modifiers, Size};
    use denise_evdev::{InputBackend, layout};

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(30)
        .clamp(1, 600);

    let mut input = InputBackend::open_all(Size::new(1280, 800))?;
    let (chosen, source) = match std::env::args().nth(2) {
        Some(name) => match layout::by_name(layout::normalise_name(&name)) {
            Some(layout) => {
                input.set_layout(layout);
                (layout, layout::LayoutSource::Denise)
            }
            None => {
                eprintln!("no layout called {name:?}; falling back to the system's");
                input.set_layout_from_system()
            }
        },
        None => input.set_layout_from_system(),
    };

    for device in input.devices() {
        eprintln!("input   {}: {}", device.capabilities(), device.name());
    }
    eprintln!(
        "keymap  {} (from {source})   (available: {})",
        chosen.name,
        layout::BUILT_IN
            .iter()
            .map(|l| l.name)
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("\npress keys — Escape quits\n");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut events = Vec::new();
    let mut keys = 0u32;
    let mut characters = 0u32;
    let mut quit = false;

    while !quit && Instant::now() < deadline {
        events.clear();
        input.poll(&mut events);

        for event in &events {
            match event {
                InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    modifiers,
                    repeat,
                } => {
                    keys += 1;
                    let mut held = String::new();
                    for (bit, name) in [
                        (Modifiers::SHIFT, "shift"),
                        (Modifiers::CTRL, "ctrl"),
                        (Modifiers::ALT, "alt"),
                        (Modifiers::SUPER, "super"),
                    ] {
                        if modifiers.contains(bit) {
                            held.push(' ');
                            held.push_str(name);
                        }
                    }
                    let repeat = if *repeat { " (repeat)" } else { "" };
                    eprintln!("key   {code:?}{held}{repeat}");
                    if *code == KeyCode::Escape {
                        quit = true;
                    }
                }
                InputEvent::Text { ch } => {
                    characters += 1;
                    eprintln!("  --> text {ch:?}  U+{:04X}", *ch as u32);
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }

    eprintln!("\n{keys} key presses produced {characters} characters");
    if keys > 0 && characters == 0 {
        eprintln!("nothing typed: either the keys pressed are not text keys, or");
        eprintln!("the layout above is not the one on the keyboard");
    }
    Ok(())
}
