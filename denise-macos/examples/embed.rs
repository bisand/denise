//! A Denise panel inside an ordinary Cocoa window.
//!
//! ```text
//! cargo run -p denise-macos --example embed
//! ```
//!
//! The window, the run loop and the menu bar belong to AppKit. Denise owns one
//! `NSView` inside it and nothing else — which is the whole point of this backend
//! and the shape a real host would use: `DeniseView` goes into a split view, a
//! tab, or a window of its own, next to controls the host already has.
//!
//! Worth watching for while it runs:
//!
//! - The caret blinks without any input, because the timer calls `update` and the
//!   tree damages exactly the caret's rectangle. Nothing else repaints.
//! - Hover lights the buttons through a tracking area the view installs itself,
//!   so the host never had to set `acceptsMouseMovedEvents` on its window.
//! - Typing `ø` works: AppKit has already run the layout and any dead key by the
//!   time the character arrives, so Denise takes it as committed text.
//! - There is exactly one cursor, the system's. `show_cursor(false)` says so once
//!   and no amount of pointer motion argues.
//!
//! # Looking at it without a screen
//!
//! ```text
//! cargo run -p denise-macos --example embed -- snapshot embed.ppm
//! ```
//!
//! Builds the view, renders it through AppKit's own `cacheDisplayInRect:` — so
//! `drawRect:`, `isFlipped` and the blit all really run — and writes a PPM
//! instead of showing a window. Useful over SSH, and useful in review: the whole
//! draw path is exercised without a window server.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("this example needs macOS");
}

#[cfg(target_os = "macos")]
fn main() {
    app::run();
}

#[cfg(target_os = "macos")]
mod app {
    use std::time::Instant;

    use denise::{InputEvent, Radius, Rect, Role, Size, Surface, Theme};
    use denise_macos::{DeniseView, ViewDelegate, ViewSurface};
    use denise_ui::widgets::{Align, Button, Label, Panel, TextInput};
    use denise_ui::{NodeId, Ui};
    use objc2::rc::Retained;
    use objc2::{MainThreadOnly, sel};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, NSTimer};

    /// Points, not pixels. On a Retina display the surface behind this is twice
    /// as many pixels across, which is exactly the conversion this backend exists
    /// to get right.
    const WIDTH: f64 = 520.0;
    const HEIGHT: f64 = 360.0;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Msg {
        Save,
        Reset,
    }

    struct Demo {
        ui: Ui<Msg>,
        status: NodeId,
        name: NodeId,
        started: Instant,
        saves: u32,
    }

    impl Demo {
        fn new(size: Size) -> Self {
            let mut ui: Ui<Msg> = Ui::new(size, Theme::DARK);
            // The window system draws a pointer already. Said once, it sticks.
            ui.show_cursor(false);

            let root = ui.root();
            // The tree lays out in physical pixels, so a Retina view is twice as
            // many of them and everything — positions *and* type sizes — is
            // multiplied here. A backend that scaled only the geometry would draw
            // a correctly placed panel in half-size text.
            let scale = (size.height as f32 / HEIGHT as f32).max(1.0);
            let px = |points: f32| (points * scale) as i32;
            let pt = |points: f32| (points * scale) as u16;

            let card = ui
                .add(
                    root,
                    Panel::default().with_radius(Radius::Box),
                    Rect::new(px(28.0), px(28.0), px(464.0), px(304.0)),
                )
                .expect("card");

            ui.add(
                card,
                Label::new("Operatørpanel").with_size(pt(22.0)),
                Rect::new(px(24.0), px(20.0), px(416.0), px(30.0)),
            )
            .expect("title");

            ui.add(
                card,
                Label::new("Navn").with_size(pt(13.0)),
                Rect::new(px(24.0), px(70.0), px(416.0), px(22.0)),
            )
            .expect("name label");

            let name = ui
                .add(
                    card,
                    TextInput::<Msg>::new()
                        .with_placeholder("Ola Nordmann")
                        .with_size(pt(15.0))
                        .with_submit(Msg::Save),
                    Rect::new(px(24.0), px(96.0), px(416.0), px(40.0)),
                )
                .expect("name");

            ui.add(
                card,
                Button::new("Lagre", Msg::Save).with_size(pt(15.0)),
                Rect::new(px(24.0), px(160.0), px(200.0), px(44.0)),
            )
            .expect("save");

            ui.add(
                card,
                Button::new("Nullstill", Msg::Reset)
                    .with_role(Role::Neutral)
                    .with_size(pt(15.0)),
                Rect::new(px(240.0), px(160.0), px(200.0), px(44.0)),
            )
            .expect("reset");

            let status = ui
                .add(
                    card,
                    Label::new("Tab, Enter, click. Try æ ø å.")
                        .with_role(Role::Base300)
                        .with_size(pt(13.0))
                        .with_align(Align::Center, Align::Center),
                    Rect::new(px(24.0), px(220.0), px(416.0), px(24.0)),
                )
                .expect("status");

            ui.focus(Some(name));

            Self {
                ui,
                status,
                name,
                started: Instant::now(),
                saves: 0,
            }
        }

        fn set_status(&mut self, text: &str) {
            if let Some(label) = self.ui.widget_mut::<Label>(self.status) {
                label.set_text(text);
            }
        }

        fn now_ms(&self) -> u64 {
            self.started.elapsed().as_millis() as u64
        }
    }

    impl ViewDelegate for Demo {
        fn update(
            &mut self,
            surface: &mut ViewSurface,
            events: &[InputEvent],
            damage: &mut Vec<Rect>,
        ) {
            // The view may have been resized since the last update, and a tree
            // laid out for the old size would draw off the edge.
            if surface.size() != self.ui.size() {
                self.ui = Demo::new(surface.size()).ui;
                self.ui.invalidate_all();
            }

            self.ui.handle(events);
            self.ui.tick(self.now_ms());

            let messages: Vec<Msg> = self.ui.drain_messages().collect();
            for message in messages {
                match message {
                    Msg::Save => {
                        self.saves += 1;
                        let who = self
                            .ui
                            .widget::<TextInput<Msg>>(self.name)
                            .map(|f| f.text().to_owned())
                            .unwrap_or_default();
                        let who = if who.is_empty() { "ingen" } else { &who };
                        let text = format!("Lagret {} ({who})", self.saves);
                        self.set_status(&text);
                    }
                    Msg::Reset => {
                        if let Some(field) = self.ui.widget_mut::<TextInput<Msg>>(self.name) {
                            field.clear();
                        }
                        self.set_status("Nullstilt.");
                    }
                }
            }

            // The whole reason `needs_paint` exists: an idle panel does nothing at
            // all, and AppKit is told nothing, so nothing composites.
            if !self.ui.needs_paint() {
                return;
            }

            match surface.acquire() {
                Ok(mut frame) => {
                    self.ui.paint(&mut frame);
                    drop(frame);
                    damage.extend_from_slice(self.ui.damage());
                    self.ui.presented();
                }
                Err(e) => eprintln!("acquire: {e}"),
            }
        }

        fn next_wake_ms(&self) -> Option<u64> {
            self.ui.next_wake_ms()
        }
    }

    pub fn run() {
        let mut args = std::env::args().skip(1);
        if args.next().as_deref() == Some("snapshot") {
            let path = args.next().unwrap_or_else(|| "embed.ppm".to_owned());
            snapshot(&path);
            return;
        }

        let mtm = MainThreadMarker::new().expect("main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT));
        // 1.0 to start with. AppKit corrects it through
        // `viewDidChangeBackingProperties` as soon as the view has a window, which
        // is a Retina display's whole story in one callback.
        let view = DeniseView::new(
            mtm,
            frame,
            1.0,
            Box::new(Demo::new(Size::new(WIDTH as u32, HEIGHT as u32))),
        )
        .expect("view");

        let window = new_window(mtm, frame);
        window.setContentView(Some(&view));
        window.setTitle(&NSString::from_str("Denise — embedded NSView"));
        window.makeKeyAndOrderFront(None);
        window.makeFirstResponder(Some(&view));

        // Now the view has a window, so the backing scale is finally knowable.
        view.sync_surface_size();
        view.update();

        // A fixed heartbeat rather than a rescheduled one-shot: `next_wake_ms`
        // exists for a host that wants to be precise, and 20 ms of slack is
        // cheaper than the code to be precise about a blinking caret.
        // SAFETY: the view outlives the timer — the window retains it — and
        // `deniseTick:` is a method it defines.
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                0.02,
                &view,
                sel!(deniseTick:),
                None,
                true,
            );
        }

        eprintln!("running — close the window to quit");
        app.run();
    }

    /// Renders the view through AppKit and writes a PPM, with no window.
    ///
    /// `cacheDisplayInRect:toBitmapImageRep:` runs the real `drawRect:`, so this
    /// covers `isFlipped`, the CoreGraphics blit and the widget rendering — every
    /// part of the path except the window server compositing the result.
    fn snapshot(path: &str) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT));
        let size = Size::new(WIDTH as u32, HEIGHT as u32);
        let view = DeniseView::new(mtm, frame, 1.0, Box::new(Demo::new(size))).expect("view");
        view.update();

        let rep = view
            .bitmapImageRepForCachingDisplayInRect(frame)
            .expect("bitmap rep");
        view.cacheDisplayInRect_toBitmapImageRep(frame, &rep);

        let data = rep.bitmapData();
        assert!(!data.is_null(), "no bitmap data");
        let width = rep.pixelsWide() as usize;
        let height = rep.pixelsHigh() as usize;
        let stride = rep.bytesPerRow() as usize;
        let bytes_per_pixel = (rep.bitsPerPixel() / 8) as usize;
        // SAFETY: `rep` owns `stride * height` bytes and outlives this slice.
        let pixels = unsafe { std::slice::from_raw_parts(data, stride * height) };

        let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
        for y in 0..height {
            for x in 0..width {
                let at = y * stride + x * bytes_per_pixel;
                // The rep AppKit hands back is RGBA in byte order, unlike the
                // 0xAARRGGBB words Denise writes. Two conventions, one machine.
                out.extend_from_slice(&pixels[at..at + 3]);
            }
        }
        std::fs::write(path, out).expect("write ppm");
        eprintln!("wrote {path} ({width}x{height})");
    }

    fn new_window(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSWindow> {
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // SAFETY: the standard designated initialiser, with a style mask and
        // backing store type it accepts.
        unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        }
    }
}
