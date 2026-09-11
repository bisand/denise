//! The gallery and the record editor, drawing into a browser canvas.
//!
//! Neither application is rewritten for the web. Their `app.rs` files are
//! compiled in from `examples/` by path, byte for byte, and this file is a
//! third backend beside the two each `main.rs` already has: it takes input
//! events from the page, runs the same `handle`, `tick` and `paint` loop the
//! window backend runs, and hands back the damage rectangles so the page copies
//! exactly what changed onto the canvas — which is the thing the toolkit is
//! about, now visible in a browser's paint flashing.
//!
//! It targets `wasm32-wasip1` rather than `wasm32-unknown-unknown` because the
//! applications use `std::time::Instant` and read `/etc/localtime`. A WASI
//! target lets them do that unchanged; the page supplies a clock, random bytes
//! and no filesystem, so the clock shows UTC and the editor's Save reports that
//! it cannot write — both of which are true of a browser tab.
//!
//! Everything crosses the boundary as numbers. There is no `wasm-bindgen`: the
//! page passes key codes through a small shared buffer and reads pixels and
//! rectangles straight out of linear memory.

use std::cell::RefCell;

use denise::{
    BufferAge, ElementState, Frame, InputEvent, PixelFormat, Point, PointerButton, Rect, Size,
};
use denise_text::{GlyphSource, TrueTypeSource};
use denise_ui::{Motion, TextStyle};

mod keymap;

// The gallery's `app.rs` names `crate::clock`, and the editor's names
// `crate::table`, so those two live at the root where they expect to be.
#[allow(dead_code)]
#[path = "../../../examples/gallery/src/clock.rs"]
mod clock;
#[allow(dead_code)]
#[path = "../../../examples/gallery/src/app.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "../../../examples/table-editor/src/table.rs"]
mod table;
#[allow(dead_code)]
#[path = "../../../examples/table-editor/src/app.rs"]
mod editor;

/// The most rectangles one frame reports. More than that is reported as their
/// bounding box, the same degradation the damage tracker itself makes.
const MAX_RECTS: usize = 64;

enum Demo {
    Gallery(gallery::App),
    Editor(editor::App),
}

impl Demo {
    fn ui(&mut self) -> &mut dyn UiLoop {
        match self {
            Demo::Gallery(app) => app,
            Demo::Editor(app) => app,
        }
    }
}

/// The loop both applications' window backends run, named once.
trait UiLoop {
    fn step(&mut self, events: &[InputEvent]);
    fn needs_paint(&self) -> bool;
    fn pending(&self) -> &[Rect];
    fn paint(&mut self, frame: &mut Frame<'_>);
    fn next_wake_in(&self) -> Option<u64>;
}

impl UiLoop for gallery::App {
    fn step(&mut self, events: &[InputEvent]) {
        self.keyboard_input(events);
        self.ui.handle(events);
        let now = self.elapsed_ms();
        self.ui.tick(now);
        self.handle(now);
    }
    fn needs_paint(&self) -> bool {
        self.ui.needs_paint()
    }
    fn pending(&self) -> &[Rect] {
        self.ui.pending_damage()
    }
    fn paint(&mut self, frame: &mut Frame<'_>) {
        self.ui.paint(frame);
        self.ui.presented();
    }
    fn next_wake_in(&self) -> Option<u64> {
        let now = self.elapsed_ms();
        self.ui.next_wake_ms().map(|wake| wake.saturating_sub(now))
    }
}

impl UiLoop for editor::App {
    fn step(&mut self, events: &[InputEvent]) {
        // The keys the editor's window backend claims before the tree sees
        // them, minus Escape-quits: a tab has nothing to quit to.
        for event in events {
            if let InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            } = event
            {
                match code {
                    denise::KeyCode::ArrowUp if !self.is_confirming() => self.move_selection(-1),
                    denise::KeyCode::ArrowDown if !self.is_confirming() => self.move_selection(1),
                    denise::KeyCode::F2 => self.on_message(editor::Message::NextTheme),
                    denise::KeyCode::Escape if self.keyboard_open() => self.dismiss_keyboard(),
                    _ => {}
                }
            }
        }
        self.keyboard_input(events);
        self.ui.handle(events);
        let now = self.elapsed_ms();
        self.ui.tick(now);
        self.handle(now);
    }
    fn needs_paint(&self) -> bool {
        self.ui.needs_paint()
    }
    fn pending(&self) -> &[Rect] {
        self.ui.pending_damage()
    }
    fn paint(&mut self, frame: &mut Frame<'_>) {
        self.ui.paint(frame);
        self.ui.presented();
    }
    fn next_wake_in(&self) -> Option<u64> {
        let now = self.elapsed_ms();
        self.ui.next_wake_ms().map(|wake| wake.saturating_sub(now))
    }
}

struct Host {
    demo: Demo,
    size: Size,
    /// What the tree paints into: one persistent buffer, so every frame after
    /// the first has an age of one.
    pixels: Vec<u32>,
    /// The same pixels in the byte order `ImageData` wants, refreshed only
    /// inside the rectangles that changed.
    rgba: Vec<u8>,
    first: bool,
    /// Which built-in theme the editor is on. It starts on the first and its
    /// only message is "next", so the host counts.
    editor_theme: usize,
    events: Vec<InputEvent>,
    /// `x, y, w, h` per rectangle painted by the last frame.
    rects: [i32; MAX_RECTS * 4],
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    /// Where the page writes a `KeyboardEvent.code` before calling [`denise_key`].
    static KEY: RefCell<[u8; 32]> = const { RefCell::new([0; 32]) };
}

fn font() -> Option<(String, Box<dyn GlyphSource>)> {
    TrueTypeSource::from_bytes("Noto Sans", ttf_noto_sans::REGULAR)
        .ok()
        .map(|source| ("Noto Sans".to_string(), Box::new(source) as Box<dyn GlyphSource>))
}

fn with_host<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|host| host.borrow_mut().as_mut().map(f))
}

fn push(event: InputEvent) {
    with_host(|host| host.events.push(event));
}

/// Builds `demo` — 0 the gallery, 1 the record editor — for a surface of
/// `width` × `height` physical pixels at `scale` physical per logical.
///
/// `light` starts it in the light theme, to match the page around it, and
/// `reduced` is `prefers-reduced-motion`: `Motion::None`, which lands every
/// transition at once and leaves the tree asking for no wake at all.
#[unsafe(no_mangle)]
pub extern "C" fn denise_start(
    demo: u32,
    width: u32,
    height: u32,
    scale: f32,
    light: u32,
    reduced: u32,
) -> u32 {
    let size = Size::new(width.max(1), height.max(1));
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let motion = if reduced != 0 {
        Motion::None
    } else {
        Motion::default()
    };
    let demo = match demo {
        1 => {
            let px = |v: u16| ((v as f32) * scale + 0.5) as u16;
            let mut app = editor::App::new(
                size,
                scale,
                "people.csv".to_string(),
                table::Table::parse(table::SAMPLE),
                TextStyle::built_in(px(16)),
                TextStyle::built_in(px(24)),
            );
            if let Some((_, source)) = font() {
                let id = app.ui.add_font(source);
                app.set_font(id);
            }
            app.ui.show_cursor(false);
            app.ui.set_motion(motion);
            if light == 0 {
                app.on_message(editor::Message::NextTheme);
            }
            Demo::Editor(app)
        }
        _ => {
            let mut app = gallery::App::new(size, scale, font(), motion);
            app.ui.show_cursor(false);
            if light != 0 {
                app.on_message(gallery::Message::UseTheme(0));
            }
            Demo::Gallery(app)
        }
    };
    let pixels = (size.width as usize) * (size.height as usize);
    HOST.with(|host| {
        *host.borrow_mut() = Some(Host {
            demo,
            size,
            pixels: vec![0; pixels],
            rgba: vec![255; pixels * 4],
            first: true,
            editor_theme: if light == 0 { 1 } else { 0 },
            events: Vec::new(),
            rects: [0; MAX_RECTS * 4],
        });
    });
    1
}

/// Runs one pass of the loop and paints if anything is dirty.
///
/// Returns how many rectangles were painted, each readable at
/// [`denise_rects`] as four `i32`s; zero means nothing changed and the page
/// has nothing to copy.
#[unsafe(no_mangle)]
pub extern "C" fn denise_frame() -> u32 {
    with_host(|host| {
        let events = std::mem::take(&mut host.events);
        let ui = host.demo.ui();
        ui.step(&events);
        if !ui.needs_paint() {
            return 0;
        }

        let full = Rect::from_size(host.size);
        let mut rects: Vec<Rect> = if host.first || ui.pending().is_empty() {
            vec![full]
        } else {
            ui.pending().to_vec()
        };
        if rects.len() > MAX_RECTS {
            let union = rects.iter().skip(1).fold(rects[0], |a, b| a.union(b));
            rects = vec![union];
        }

        let age = if host.first {
            BufferAge::Undefined
        } else {
            BufferAge::Frames(1)
        };
        host.first = false;
        {
            let Ok(mut frame) = Frame::new(
                &mut host.pixels,
                host.size,
                host.size.width,
                PixelFormat::Xrgb8888,
                age,
            ) else {
                return 0;
            };
            ui.paint(&mut frame);
        }

        let width = host.size.width as usize;
        let mut count = 0;
        for rect in &rects {
            let Some(rect) = rect.intersect(&full) else {
                continue;
            };
            let (x0, y0) = (rect.x as usize, rect.y as usize);
            let (x1, y1) = (x0 + rect.width as usize, y0 + rect.height as usize);
            for y in y0..y1 {
                let row = &host.pixels[y * width + x0..y * width + x1];
                let out = &mut host.rgba[(y * width + x0) * 4..(y * width + x1) * 4];
                for (word, px) in row.iter().zip(out.chunks_exact_mut(4)) {
                    px[0] = (word >> 16) as u8;
                    px[1] = (word >> 8) as u8;
                    px[2] = *word as u8;
                    px[3] = 255;
                }
            }
            host.rects[count * 4..count * 4 + 4]
                .copy_from_slice(&[rect.x, rect.y, rect.width, rect.height]);
            count += 1;
        }
        count as u32
    })
    .unwrap_or(0)
}

/// Milliseconds until the tree wants another pass, or `-1` for "not until
/// something happens".
#[unsafe(no_mangle)]
pub extern "C" fn denise_next_wake() -> i32 {
    with_host(|host| {
        host.demo
            .ui()
            .next_wake_in()
            .map_or(-1, |ms| ms.min(i32::MAX as u64) as i32)
    })
    .unwrap_or(-1)
}

/// The RGBA pixels, `width * height * 4` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn denise_rgba() -> *const u8 {
    with_host(|host| host.rgba.as_ptr()).unwrap_or(std::ptr::null())
}

/// The rectangles the last [`denise_frame`] painted.
#[unsafe(no_mangle)]
pub extern "C" fn denise_rects() -> *const i32 {
    with_host(|host| host.rects.as_ptr()).unwrap_or(std::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn denise_pointer_move(x: i32, y: i32) {
    push(InputEvent::PointerMoved {
        position: Point::new(x, y),
    });
}

/// `button` is `MouseEvent.button`: 0 left, 1 middle, 2 right.
#[unsafe(no_mangle)]
pub extern "C" fn denise_pointer_button(button: u32, down: u32, x: i32, y: i32, mods: u32) {
    push(InputEvent::PointerButton {
        button: match button {
            0 => PointerButton::Left,
            1 => PointerButton::Middle,
            2 => PointerButton::Right,
            other => PointerButton::Other(other as u16),
        },
        state: if down != 0 {
            ElementState::Down
        } else {
            ElementState::Up
        },
        position: Point::new(x, y),
        modifiers: keymap::modifiers(mods),
    });
}

/// Deltas are content offsets, as `InputEvent::PointerScroll` wants, which is
/// the sign `WheelEvent.deltaX` and `deltaY` already have.
#[unsafe(no_mangle)]
pub extern "C" fn denise_scroll(dx: f32, dy: f32, x: i32, y: i32) {
    push(InputEvent::PointerScroll {
        delta_x: dx,
        delta_y: dy,
        position: Point::new(x, y),
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn denise_pointer_left() {
    push(InputEvent::PointerLeft);
}

/// `phase` is 0 down, 1 moved, 2 up, 3 cancelled.
#[unsafe(no_mangle)]
pub extern "C" fn denise_touch(phase: u32, id: u32, x: i32, y: i32) {
    let id = u64::from(id);
    let position = Point::new(x, y);
    push(match phase {
        0 => InputEvent::TouchDown { id, position },
        1 => InputEvent::TouchMoved { id, position },
        _ => InputEvent::TouchUp {
            id,
            position,
            cancelled: phase == 3,
        },
    });
}

/// Where the page writes a key's `code`, up to 32 bytes of ASCII.
#[unsafe(no_mangle)]
pub extern "C" fn denise_key_buffer() -> *mut u8 {
    KEY.with(|key| key.borrow_mut().as_mut_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn denise_key(len: u32, down: u32, repeat: u32, mods: u32) {
    let code = KEY.with(|key| {
        let key = key.borrow();
        let len = (len as usize).min(key.len());
        keymap::key_code(std::str::from_utf8(&key[..len]).unwrap_or(""))
    });
    push(InputEvent::Key {
        code,
        state: if down != 0 {
            ElementState::Down
        } else {
            ElementState::Up
        },
        repeat: repeat != 0,
        modifiers: keymap::modifiers(mods),
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn denise_text(ch: u32) {
    if let Some(ch) = char::from_u32(ch) {
        push(InputEvent::Text { ch });
    }
}

/// Switches to a built-in theme: 0 light, 1 dark, 2 high contrast.
#[unsafe(no_mangle)]
pub extern "C" fn denise_theme(index: u32) {
    with_host(|host| match &mut host.demo {
        Demo::Gallery(app) => app.on_message(gallery::Message::UseTheme(index as usize % 3)),
        Demo::Editor(app) => {
            // The editor only cycles, so step it until it lands.
            while host.editor_theme != index as usize % 3 {
                app.on_message(editor::Message::NextTheme);
                host.editor_theme = (host.editor_theme + 1) % 3;
            }
        }
    });
}
