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

    use denise::{InputEvent, Radius, Rect, Role, Size, Surface, Theme};
    use denise_ui::widgets::{Align, Button, Label, Panel, TextInput};
    use denise_ui::{NodeId, Ui};
    use denise_win32::{ControlDelegate, DeniseControl, DibSurface};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG,
        PostQuitMessage, RegisterClassExW, SW_SHOW, SWP_NOZORDER, SetTimer, SetWindowPos,
        ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_SIZE, WM_TIMER, WNDCLASSEXW,
        WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    const WIDTH: i32 = 520;
    const HEIGHT: i32 = 360;
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
        name: NodeId,
        started: Instant,
        saves: u32,
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
                    Rect::new(24, 190, 380, 24),
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

        // SAFETY: the class was just registered and every argument is valid.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("Denise.Example.Host"),
                w!("Denise — embedded control"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                WIDTH,
                HEIGHT,
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
