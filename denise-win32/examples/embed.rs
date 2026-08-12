//! A Denise panel as a child control in an ordinary Win32 window.
//!
//! ```text
//! cargo run -p denise-win32 --example embed
//! ```
//!
//! The window, the message loop and the timer belong to the host. Denise owns one
//! child `HWND` inside it and nothing else — which is the shape a real host uses:
//! the control goes in a dialog next to the buttons it already has.
//!
//! # It is a diagnostic, not just a demo
//!
//! Three lines under the panel report what actually arrived: the last key
//! position with its modifiers, the last committed character with its codepoint,
//! and the last pointer event with the damage the frame produced. That is the
//! same trick `denise-evdev`'s `keys` example plays on Linux, and it is what
//! turned "æøå does not work" into a fixed AltGr bug in M4 — a panel that merely
//! *looks* right tells you nothing about which layer is lying.
//!
//! What to try, in rough order of how likely it is to be broken:
//!
//! 1. **Tab.** It should move focus between the field and the buttons. If the
//!    control is in a dialog and Tab never arrives, `WM_GETDLGCODE` is the
//!    suspect.
//! 2. **AltGr.** On a Norwegian layout `AltGr+2` is `@`. The key line should say
//!    `AltRight` and the text line `U+0040`. If it says `AltLeft`, the extended
//!    bit is being lost.
//! 3. **`æ ø å`.** Key positions `Quote`, `Semicolon`, `BracketLeft` on a
//!    Norwegian keyboard; the characters come separately, from `WM_CHAR`.
//! 4. **Press a button and drag off it before releasing.** It must un-press. If
//!    it stays lit, `SetCapture` is not working.
//! 5. **Resize the window.** The damage line should show small rectangles once
//!    it settles, not the whole client area every frame.
//! 6. **Move it to a display with a different DPI**, if there is one.
//!
//! Compile-checked for `x86_64-pc-windows-msvc` and not yet run; see the crate
//! documentation for what that does and does not cover.

#[cfg(not(windows))]
fn main() {
    eprintln!("this example needs Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}

#[cfg(windows)]
mod app {
    use std::time::Instant;

    use denise::{ElementState, InputEvent, Modifiers, Radius, Rect, Role, Size, Surface, Theme};
    use denise_ui::widgets::{Align, Button, Label, Panel, TextInput};
    use denise_ui::{NodeId, Ui};
    use denise_win32::{ControlDelegate, DeniseControl, DibSurface};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRect, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW,
        GetMessageW, MSG, PostQuitMessage, RegisterClassExW, SW_SHOW, SWP_NOZORDER, SetTimer,
        SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_SIZE, WM_TIMER,
        WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    const WIDTH: i32 = 520;
    const HEIGHT: i32 = 400;
    /// The heartbeat, in milliseconds. Fixed rather than rescheduled from
    /// `next_wake_ms`: an update with nothing to do is one empty event list and no
    /// invalidation, which is cheaper than the code to be precise about a caret.
    const TICK_MS: u32 = 20;
    const TIMER_ID: usize = 1;

    /// Where the host keeps the control between messages. A real application
    /// would put it in its own document or dialog structure; a single-window
    /// example has nowhere better.
    static mut CONTROL: Option<DeniseControl> = None;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Msg {
        Save,
        Reset,
    }

    struct Demo {
        ui: Ui<Msg>,
        status: NodeId,
        keys: NodeId,
        text: NodeId,
        pointer: NodeId,
        name: NodeId,
        started: Instant,
        saves: u32,
        /// Rectangles the last painted frame damaged, so the line can say whether
        /// the incremental path is doing anything.
        last_damage: usize,
    }

    impl Demo {
        fn new(size: Size) -> Self {
            let mut ui: Ui<Msg> = Ui::new(size, Theme::DARK);
            // Windows draws a pointer already. Said once, it sticks.
            ui.show_cursor(false);

            let root = ui.root();
            let card = ui
                .add(
                    root,
                    Panel::default().with_radius(Radius::Box),
                    Rect::new(28, 28, size.width as i32 - 56, size.height as i32 - 56),
                )
                .expect("card");

            ui.add(
                card,
                Label::new("Operatørpanel").with_size(22),
                Rect::new(24, 20, 380, 30),
            )
            .expect("title");

            let name = ui
                .add(
                    card,
                    TextInput::<Msg>::new()
                        .with_placeholder("Ola Nordmann")
                        .with_submit(Msg::Save),
                    Rect::new(24, 70, 380, 40),
                )
                .expect("name");

            ui.add(
                card,
                Button::new("Lagre", Msg::Save),
                Rect::new(24, 130, 180, 44),
            )
            .expect("save");

            ui.add(
                card,
                Button::new("Nullstill", Msg::Reset).with_role(Role::Neutral),
                Rect::new(220, 130, 180, 44),
            )
            .expect("reset");

            let status = ui
                .add(
                    card,
                    Label::new("Tab, Enter, click.")
                        .with_role(Role::Base300)
                        .with_align(Align::Center, Align::Center),
                    Rect::new(24, 186, 380, 22),
                )
                .expect("status");

            // The diagnostic. Monospace-ish and small, because it is meant to be
            // read while something else has focus.
            let mut line = |y: i32, text: &str| {
                ui.add(
                    card,
                    Label::new(text).with_role(Role::Base300).with_size(13),
                    Rect::new(24, y, 380, 18),
                )
                .expect("diagnostic line")
            };
            let keys = line(214, "key   -");
            let text = line(234, "text  -");
            let pointer = line(254, "mouse -");

            ui.focus(Some(name));

            Self {
                ui,
                status,
                keys,
                text,
                pointer,
                name,
                started: Instant::now(),
                saves: 0,
                last_damage: 0,
            }
        }

        fn set(&mut self, node: NodeId, text: String) {
            if let Some(label) = self.ui.widget_mut::<Label>(node) {
                // `update` rather than `set_text`: an unchanged reading should not
                // cost a repaint, and holding a key down sends the same line many
                // times a second.
                label.update(&text);
            }
        }

        /// Records what arrived, so the panel reports the input rather than only
        /// reacting to it.
        fn describe(&mut self, events: &[InputEvent]) {
            for event in events {
                match event {
                    InputEvent::Key {
                        code,
                        state,
                        repeat,
                        modifiers,
                    } => {
                        let mut held = String::new();
                        for (bit, name) in [
                            (Modifiers::SHIFT, "shift"),
                            (Modifiers::CTRL, "ctrl"),
                            (Modifiers::ALT, "alt"),
                            (Modifiers::SUPER, "win"),
                        ] {
                            if modifiers.contains(bit) {
                                held.push(' ');
                                held.push_str(name);
                            }
                        }
                        let updown = if state == &ElementState::Down {
                            "v"
                        } else {
                            "^"
                        };
                        let rep = if *repeat { " (repeat)" } else { "" };
                        self.set(self.keys, format!("key   {updown} {code:?}{held}{rep}"));
                    }
                    InputEvent::Text { ch } => {
                        self.set(self.text, format!("text  {ch:?}  U+{:04X}", *ch as u32));
                    }
                    InputEvent::PointerMoved { position } => {
                        let damage = self.last_damage;
                        self.set(
                            self.pointer,
                            format!("mouse {},{}   damage {damage}", position.x, position.y),
                        );
                    }
                    InputEvent::PointerScroll { delta_y, .. } => {
                        self.set(self.pointer, format!("wheel {delta_y:+.0}"));
                    }
                    InputEvent::PointerLeft => {
                        self.set(self.pointer, "mouse left the control".to_owned());
                    }
                    _ => {}
                }
            }
        }
    }

    impl ControlDelegate for Demo {
        fn update(
            &mut self,
            surface: &mut DibSurface,
            events: &[InputEvent],
            damage: &mut Vec<Rect>,
        ) {
            if surface.size() != self.ui.size() {
                self.ui = Demo::new(surface.size()).ui;
                self.ui.invalidate_all();
            }

            self.describe(events);
            self.ui.handle(events);
            self.ui.tick(self.started.elapsed().as_millis() as u64);

            let messages: Vec<Msg> = self.ui.drain_messages().collect();
            for message in messages {
                let text = match message {
                    Msg::Save => {
                        self.saves += 1;
                        let who = self
                            .ui
                            .widget::<TextInput<Msg>>(self.name)
                            .map(|f| f.text().to_owned())
                            .unwrap_or_default();
                        format!(
                            "Lagret {} ({})",
                            self.saves,
                            if who.is_empty() { "ingen" } else { &who }
                        )
                    }
                    Msg::Reset => {
                        if let Some(field) = self.ui.widget_mut::<TextInput<Msg>>(self.name) {
                            field.clear();
                        }
                        "Nullstilt.".to_owned()
                    }
                };
                if let Some(label) = self.ui.widget_mut::<Label>(self.status) {
                    label.set_text(text);
                }
            }

            // An idle panel does nothing at all, and Windows is told nothing, so
            // nothing composites.
            if !self.ui.needs_paint() {
                return;
            }

            match surface.acquire() {
                Ok(mut frame) => {
                    self.ui.paint(&mut frame);
                    drop(frame);
                    damage.extend_from_slice(self.ui.damage());
                    self.last_damage = self.ui.damage().len();
                    self.ui.presented();
                }
                Err(e) => eprintln!("acquire: {e}"),
            }
        }

        fn next_wake_ms(&self) -> Option<u64> {
            self.ui.next_wake_ms()
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        // SAFETY: a null module name asks for this process's handle.
        let instance = unsafe { GetModuleHandleW(None) }?;

        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(host_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Denise.Example.Host"),
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised and `host_proc` has the required
        // signature.
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err("could not register the host class".into());
        }

        // `CreateWindowEx` takes the *window* size, not the client size, so
        // asking for 520x400 gets a client area smaller by the title bar and the
        // borders — and the panel inside would be quietly cut off at the bottom.
        // This is what the API is for.
        let mut wanted = RECT {
            left: 0,
            top: 0,
            right: WIDTH,
            bottom: HEIGHT,
        };
        // SAFETY: `wanted` is a live local and the style matches the one below.
        unsafe { AdjustWindowRect(&mut wanted, WS_OVERLAPPEDWINDOW, false) }?;

        // SAFETY: the class was just registered and every argument is valid.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("Denise.Example.Host"),
                w!("Denise — embedded control"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                wanted.right - wanted.left,
                wanted.bottom - wanted.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }?;

        let control = DeniseControl::new(
            window,
            Rect::new(0, 0, WIDTH, HEIGHT),
            1.0,
            Box::new(Demo::new(Size::new(WIDTH as u32, HEIGHT as u32))),
        )?;
        // SAFETY: single-threaded, and nothing reads this before it is written.
        unsafe { CONTROL = Some(control) };
        control.update();

        // SAFETY: `window` is live and the timer id is this window's own.
        unsafe { SetTimer(Some(window), TIMER_ID, TICK_MS, None) };
        // SAFETY: `window` is live.
        unsafe {
            let _ = ShowWindow(window, SW_SHOW);
        }

        let mut message = MSG::default();
        // SAFETY: the standard message loop. `GetMessageW` returns 0 on WM_QUIT
        // and -1 on error, both of which end it.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            // SAFETY: `message` was just filled by `GetMessageW`. `TranslateMessage`
            // is what turns WM_KEYDOWN into WM_CHAR, so a panel without it can be
            // navigated and not typed into.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    extern "system" fn host_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_SIZE => {
                let width = (lparam.0 & 0xFFFF) as i32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                // SAFETY: single-threaded; the control outlives the window.
                if let Some(control) = unsafe { CONTROL } {
                    // SAFETY: the control's window is live for as long as this one.
                    unsafe {
                        let _ =
                            SetWindowPos(control.hwnd(), None, 0, 0, width, height, SWP_NOZORDER);
                    }
                }
                LRESULT(0)
            }
            WM_TIMER => {
                // SAFETY: as above.
                if let Some(control) = unsafe { CONTROL } {
                    control.update();
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: ends the message loop above.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: the standard fallback, valid for any window and message.
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }
}
