//! The smallest useful Denise application: type a name, press a button.
//!
//! ```text
//! cargo run -p hello
//! ```
//!
//! Everything a Denise application needs is here and nothing else is. Read it top
//! to bottom; it is about eighty lines, and roughly half of those are comments.
//!
//! # The three pieces
//!
//! 1. **A message type.** Widgets do not run callbacks. A button holds a value of
//!    your own type and emits it when pressed, so every state change in the
//!    application happens in one place you wrote, not in a closure somewhere else.
//! 2. **A tree.** [`Ui::add`] returns a [`NodeId`], which is how you reach a widget
//!    again later — to read what was typed, or to change what a label says.
//! 3. **An event loop.** `denise-winit` provides one for development. On a kiosk
//!    you would swap it for `denise-drm`, and on Windows or macOS the control
//!    embeds in a window the host already owns. The code between here and there
//!    does not change.
//!
//! # What you will not find
//!
//! No dirty flags, no `invalidate()` calls, no repaint bookkeeping. Type into the
//! field and the toolkit repaints the field, not the window. That is the one thing
//! this library is really about, and the way you use it is by not doing anything.

#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
use std::time::Duration;
use std::time::Instant;

#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
use denise::{DamageTracker, ElementState, Frame, InputEvent, KeyCode};
use denise::{Rect, Role, Size, theme};
use denise_ui::widgets::{Button, Label, Panel, TextInput};
use denise_ui::{NodeId, Ui};
#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
use denise_winit::{DeniseApp, WindowConfig, run_with};

const WINDOW: Size = Size::new(460, 260);

/// What the widgets send back.
///
/// One variant here; a real application has one per thing that can happen. It is
/// an ordinary enum, so the compiler tells you when you forget to handle one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Message {
    Greet,
}

#[cfg(all(feature = "kiosk", target_os = "linux"))]
mod kiosk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `--snapshot out.ppm` draws one frame and exits. It needs no display, which
    // makes it the way to review a layout over SSH, to diff a theme change, and
    // to produce the images in the README. The rest of this file is the part
    // worth reading.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--snapshot") {
        let path = args.next().unwrap_or_else(|| "hello.ppm".into());
        // `--snapshot out.ppm 2` renders the same layout at a 2x scale factor —
        // the whole of Denise's DPI story made executable. Fractions work too.
        let scale: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1.0);
        return snapshot(&path, scale).map_err(Into::into);
    }

    // Which display this talks to is decided here, at compile time. `kiosk` drives
    // a Linux framebuffer directly and never links winit; the default opens a
    // window. The forty lines that differ live in `kiosk.rs`, so the file you are
    // reading stays about the tree and not about the machine.
    #[cfg(all(feature = "kiosk", target_os = "linux"))]
    return kiosk::run();

    #[cfg(not(all(feature = "kiosk", target_os = "linux")))]
    {
        run_with(
            WindowConfig {
                title: "Denise — hello".into(),
                // Logical: the same apparent window on a Pi and on a Retina Mac.
                size: WINDOW,
                ..WindowConfig::default()
            },
            // The surface and the display's scale factor, handed over at the one
            // moment they are both known and nothing has been built yet. The same
            // two numbers the `--snapshot` path takes from the command line.
            Hello::new,
        )?;
        Ok(())
    }
}

struct Hello {
    ui: Ui<Message>,
    /// The field to read the name out of.
    name: NodeId,
    /// The label to write the greeting into.
    greeting: NodeId,
    started: Instant,
    /// Set by Escape, read by the window backend. The kiosk loop returns out of
    /// itself instead, which is why this only exists on the other side.
    #[cfg(not(all(feature = "kiosk", target_os = "linux")))]
    exit: bool,
}

impl Hello {
    fn new(size: Size, scale: f32) -> Self {
        // The whole of the DPI story is these three lines and the habit that
        // follows: the layout is designed in logical units, and *the
        // application* multiplies — once, here — because it is the one that
        // computes every rectangle anyway. The theme scales its metrics, every
        // rectangle goes through `Rect::scaled` (which scales *edges*, so
        // panels designed to touch still touch at fractional scales), and text
        // sizes are the application's numbers like any other.
        let s = |r: Rect| r.scaled(scale);
        let px = |v: f32| (v * scale + 0.5) as u16;

        // A theme, not a palette. Widgets ask for roles — `Role::Primary`, and the
        // background and text colours the theme derives to stay legible against it
        // — so swapping `theme::DARK` for `theme::LIGHT` below is the whole of
        // supporting both.
        let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale));
        let root = ui.root();

        // A card to sit everything on, centred. Children are positioned relative
        // to their parent, so this rectangle is the only one in the file that
        // knows how big the screen is — which is what lets the same tree look
        // right in a 460x260 window and on a 1920x1080 display.
        let card_size = s(Rect::new(0, 0, 428, 228));
        let card = ui
            .add(
                root,
                Panel::default(),
                Rect::new(
                    (size.width as i32 - card_size.width) / 2,
                    (size.height as i32 - card_size.height) / 2,
                    card_size.width,
                    card_size.height,
                ),
            )
            .expect("card");

        ui.add(
            card,
            Label::new("Hello, Denise").with_size(px(22.0)),
            s(Rect::new(20, 18, 388, 28)),
        );
        ui.add(
            card,
            Label::new("What is your name?").with_size(px(16.0)),
            s(Rect::new(20, 58, 388, 20)),
        );

        // `with_submit` makes Enter emit the same message as the button, which is
        // what anyone typing into a field expects and what a keypad-only panel
        // needs.
        let name = ui
            .add(
                card,
                TextInput::<Message>::new()
                    .with_placeholder("your name")
                    .with_submit(Message::Greet)
                    .with_size(px(16.0)),
                s(Rect::new(20, 82, 388, 34)),
            )
            .expect("field");

        ui.add(
            card,
            Button::new("Greet", Message::Greet)
                .with_role(Role::Primary)
                .with_size(px(16.0)),
            s(Rect::new(20, 128, 110, 34)),
        );

        let greeting = ui
            .add(
                card,
                Label::new("").with_size(px(16.0)),
                s(Rect::new(20, 176, 388, 24)),
            )
            .expect("greeting");

        // The field starts focused, so the first keystroke lands somewhere useful
        // and Tab moves on from there.
        ui.focus(Some(name));

        Self {
            ui,
            name,
            greeting,
            started: Instant::now(),
            #[cfg(not(all(feature = "kiosk", target_os = "linux")))]
            exit: false,
        }
    }

    /// The only piece of application logic: read the field, write the label.
    fn greet(&mut self) {
        let name = self
            .ui
            .widget::<TextInput<Message>>(self.name)
            .map(|field| field.text().trim().to_string())
            .unwrap_or_default();

        let greeting = if name.is_empty() {
            "Hello, whoever you are.".to_string()
        } else {
            format!("Hello, {name}.")
        };

        // `widget_mut` marks the node for repaint on the way in, so the label
        // changing is all it takes. Nothing else on screen is touched.
        if let Some(label) = self.ui.widget_mut::<Label>(self.greeting) {
            label.set_text(greeting);
        }
    }
}

#[cfg(not(all(feature = "kiosk", target_os = "linux")))]
impl DeniseApp for Hello {
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        // Escape quits, as it does on the kiosk. Nothing here opens a popup or a
        // drawer, so the key is unambiguously this application's; the gallery,
        // which does open both, asks the tree first.
        for event in events {
            if let InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } = event
            {
                self.exit = true;
            }
        }

        self.ui.handle(events);
        // Drives the caret blink. A tree with nothing animating asks to be woken
        // never, which is why an idle panel costs nothing.
        self.ui.tick(self.started.elapsed().as_millis() as u64);

        // Collected before acting: draining borrows the tree, and handling a
        // message needs it back.
        let messages: Vec<Message> = self.ui.drain_messages().collect();
        for message in messages {
            match message {
                Message::Greet => self.greet(),
            }
        }

        // What changed, not the window. A caret blinking in the field marks one
        // small rectangle, and one small rectangle is what gets copied.
        if self.ui.needs_paint() {
            let pending = self.ui.pending_damage();
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
        // The tree keeps its own damage and repaints only the widgets that
        // changed, whatever it is asked to cover. What `update` marks above is
        // what gets *copied to the screen* afterwards — the same rectangles, so
        // the window and a kiosk display do the same amount of work.
        self.ui.paint(frame);
        self.ui.presented();
    }

    fn exit_requested(&self) -> bool {
        self.exit
    }

    /// The caret is the only thing here with a clock, and it wants half a
    /// second. Saying so is the difference between sixty frames a second and
    /// two.
    fn next_frame_in(&self) -> Option<Duration> {
        let now = self.started.elapsed().as_millis() as u64;
        self.ui
            .next_wake_ms()
            .map(|wake| Duration::from_millis(wake.saturating_sub(now)))
    }
}

/// Draws one frame into a file, with no window and no event loop.
///
/// A `Frame` is just a borrowed pixel buffer with a size and a format, so
/// anything that can lend one can be drawn into — a window, a display's scanout
/// buffer, or a `Vec` like this one.
fn snapshot(path: &str, scale: f32) -> std::io::Result<()> {
    use std::io::Write as _;

    // The surface grows with the scale — a 2x display has twice the pixels —
    // and the layout inside grows to match through the one multiply in
    // `Hello::new`.
    let window = Size::new(
        (WINDOW.width as f32 * scale + 0.5) as u32,
        (WINDOW.height as f32 * scale + 0.5) as u32,
    );
    let mut hello = Hello::new(window, scale);
    hello.greet();

    let mut pixels = vec![0u32; (window.width * window.height) as usize];
    {
        let mut frame = denise::Frame::new(
            &mut pixels,
            window,
            window.width,
            denise::PixelFormat::Xrgb8888,
            denise::BufferAge::Undefined,
        )
        .expect("frame");
        hello.ui.paint(&mut frame);
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", window.width, window.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path} at {}x{}", window.width, window.height);
    Ok(())
}
