//! M3 end to end: a widget tree, a modal dialog and a cursor sprite, on bare
//! Linux with no desktop environment.
//!
//! ```text
//! /tmp/panel [seconds] [vsync]
//! ```
//!
//! Tab and Enter drive the whole thing without a pointer, which is what a panel
//! with only a membrane keypad needs. With a pointer attached, click as usual.
//! `F2` cycles the theme, `Escape` closes the dialog or quits.
//!
//! # Two things this demonstrates that are easy to get wrong
//!
//! **The application never touches damage.** There is no dirty flag anywhere
//! below, no comparison of what changed, no call to a damage tracker. The tree
//! invalidates on every route into widget state, so the whole class of "I forgot
//! to mark that dirty" bug is absent by construction rather than by care.
//!
//! **Input is read after the display wait, not before.** `acquire` blocks until
//! the previous flip retires under vsync. Reading input before it draws a pointer
//! position already up to a refresh period stale — 6.2 ms at p50 on a Pi 3, and
//! plainly visible as drag.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the panel demo needs Linux, DRM and evdev");
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}

#[cfg(target_os = "linux")]
mod app {
    use std::os::fd::BorrowedFd;
    use std::time::{Duration, Instant};

    use denise::{
        ElementState, InputEvent, InputSource, KeyCode, Radius, Rect, Role, Size, Surface, Theme,
    };
    use denise_drm::{DrmSurface, PresentMode, SurfaceConfig};
    use denise_evdev::{InputBackend, layout};
    use denise_fbdev::FbdevSurface;
    use denise_ui::widgets::{Align, Button, Label, Panel, TextInput};
    use denise_ui::{NodeId, Ui};
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    /// What the widgets can ask the application to do.
    ///
    /// Messages, not callbacks: no widget holds a reference to another widget, and
    /// everything that changes state happens in one place where the whole
    /// application is in scope.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Msg {
        Save,
        Reset,
        ConfirmSave,
        Dismiss,
    }

    /// Everything the application owns. The tree owns the rest.
    struct App {
        ui: Ui<Msg>,
        name: NodeId,
        pin: NodeId,
        status: NodeId,
        save: NodeId,
        /// Root of the confirmation scene while it is open.
        dialog: Option<NodeId>,
        theme: usize,
        saves: u32,
    }

    impl App {
        fn new(size: Size) -> Self {
            let mut ui: Ui<Msg> = Ui::new(size, Theme::BUILT_IN[1]);
            let root = ui.root();

            // Everything is laid out relative to its parent, in pixels. There is no
            // layout engine yet and a fixed-resolution panel does not need one; the
            // card is centred here by arithmetic, which is honest about that.
            let card_w = (size.width as i32 * 3 / 5).clamp(360, 720);
            let card_h = (size.height as i32 * 3 / 5).clamp(280, 520);
            let card = ui
                .add(
                    root,
                    Panel::default(),
                    Rect::new(
                        (size.width as i32 - card_w) / 2,
                        (size.height as i32 - card_h) / 2,
                        card_w,
                        card_h,
                    ),
                )
                .expect("card");

            let pad = 28;
            let row = 46;
            let inner = card_w - pad * 2;
            let mut y = pad;

            ui.add(
                card,
                Label::new("Operator sign-in").with_size(24),
                Rect::new(pad, y, inner, 30),
            )
            .expect("title");
            y += row + 8;

            ui.add(card, Label::new("Navn"), Rect::new(pad, y, inner, 24))
                .expect("name label");
            y += 26;
            let name = ui
                .add(
                    card,
                    TextInput::<Msg>::new()
                        .with_placeholder("Ola Nordmann")
                        .with_submit(Msg::Save),
                    Rect::new(pad, y, inner, 40),
                )
                .expect("name");
            y += row + 8;

            ui.add(card, Label::new("PIN"), Rect::new(pad, y, inner, 24))
                .expect("pin label");
            y += 26;
            let pin = ui
                .add(
                    card,
                    TextInput::<Msg>::new()
                        .with_password(true)
                        .with_max_chars(8)
                        .with_submit(Msg::Save),
                    Rect::new(pad, y, 200, 40),
                )
                .expect("pin");
            y += row + 20;

            let button_w = (inner - 16) / 2;
            let save = ui
                .add(
                    card,
                    Button::new("Lagre", Msg::Save),
                    Rect::new(pad, y, button_w, 46),
                )
                .expect("save");
            ui.add(
                card,
                Button::new("Nullstill", Msg::Reset).with_role(Role::Neutral),
                Rect::new(pad + button_w + 16, y, button_w, 46),
            )
            .expect("reset");
            y += 46 + 18;

            let status = ui
                .add(
                    card,
                    Label::new("F3 switches keyboard layout")
                        .with_role(Role::Base300)
                        .with_align(Align::Center, Align::Center),
                    Rect::new(pad, y, inner, 24),
                )
                .expect("status");

            Self {
                ui,
                name,
                pin,
                status,
                save,
                dialog: None,
                theme: 1,
                saves: 0,
            }
        }

        fn field_text(&self, id: NodeId) -> String {
            self.ui
                .widget::<TextInput<Msg>>(id)
                .map(|f| f.text().to_owned())
                .unwrap_or_default()
        }

        /// Names the active keyboard layout in the status line.
        fn show_layout(&mut self, name: &str) {
            let text = format!("keyboard: {name}  —  F3 switches it");
            self.set_status(&text);
        }

        fn set_status(&mut self, text: &str) {
            if let Some(label) = self.ui.widget_mut::<Label>(self.status) {
                label.set_text(text);
            }
        }

        /// Opens the confirmation as a *scene*, not as a widget inside the page.
        ///
        /// That is what makes it modal: nothing underneath is hittable, focusable
        /// or reachable by Tab, and the backdrop dims what is behind it. The dim is
        /// painted per damage region, so the dialog's own caret blinking does not
        /// drag a full-screen alpha fill along with it — which would cost 63% of a
        /// frame budget on a Pi 3.
        fn open_dialog(&mut self) {
            if self.dialog.is_some() {
                return;
            }
            let size = self.ui.size();
            let root = self.ui.push_scene(110);
            let w = 420.min(size.width as i32 - 40);
            let h = 190;
            let dialog = self
                .ui
                .add(
                    root,
                    Panel::default().with_border(Role::Primary, 2),
                    Rect::new(
                        (size.width as i32 - w) / 2,
                        (size.height as i32 - h) / 2,
                        w,
                        h,
                    ),
                )
                .expect("dialog");

            let name = self.field_text(self.name);
            let who = if name.is_empty() { "uten navn" } else { &name };
            self.ui
                .add(
                    dialog,
                    Label::new("Lagre endringer?")
                        .with_size(24)
                        .with_align(Align::Center, Align::Center),
                    Rect::new(20, 20, w - 40, 30),
                )
                .expect("dialog title");
            self.ui
                .add(
                    dialog,
                    Label::new(who)
                        .with_role(Role::Base300)
                        .with_align(Align::Center, Align::Center),
                    Rect::new(20, 58, w - 40, 24),
                )
                .expect("dialog body");

            let button_w = (w - 60) / 2;
            self.ui
                .add(
                    dialog,
                    Button::new("Ja", Msg::ConfirmSave).with_radius(Radius::Field),
                    Rect::new(20, h - 66, button_w, 46),
                )
                .expect("yes");
            self.ui
                .add(
                    dialog,
                    Button::new("Avbryt", Msg::Dismiss).with_role(Role::Neutral),
                    Rect::new(40 + button_w, h - 66, button_w, 46),
                )
                .expect("no");

            self.dialog = Some(root);
        }

        fn close_dialog(&mut self) -> bool {
            if self.dialog.take().is_none() {
                return false;
            }
            self.ui.pop_scene();
            true
        }

        fn cycle_theme(&mut self) {
            self.theme = (self.theme + 1) % Theme::BUILT_IN.len();
            let theme = Theme::BUILT_IN[self.theme];
            self.ui.set_theme(theme);
        }

        /// Drains one batch of messages. Returns `true` if the application should
        /// keep running.
        fn dispatch(&mut self) {
            let messages: Vec<Msg> = self.ui.drain_messages().collect();
            for message in messages {
                match message {
                    Msg::Save => self.open_dialog(),
                    Msg::Reset => {
                        if let Some(field) = self.ui.widget_mut::<TextInput<Msg>>(self.name) {
                            field.clear();
                        }
                        if let Some(field) = self.ui.widget_mut::<TextInput<Msg>>(self.pin) {
                            field.clear();
                        }
                        self.set_status("Nullstilt");
                    }
                    Msg::ConfirmSave => {
                        self.saves += 1;
                        let name = self.field_text(self.name);
                        let pin = self.field_text(self.pin);
                        let text = if name.is_empty() {
                            "Lagret uten navn".to_owned()
                        } else {
                            format!("Lagret {name} ({} siffer PIN)", pin.chars().count())
                        };
                        self.close_dialog();
                        self.set_status(&text);
                        self.ui.focus(Some(self.save));
                    }
                    Msg::Dismiss => {
                        self.close_dialog();
                        self.set_status("Avbrutt");
                    }
                }
            }
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let seconds: u64 = std::env::args()
            .nth(1)
            .and_then(|a| a.parse().ok())
            .unwrap_or(120)
            .clamp(1, 3600);
        let requested = match std::env::args().nth(2).as_deref() {
            Some("vsync") | Some("tearfree") => PresentMode::Vsync,
            _ => PresentMode::Immediate,
        };

        let mut surface: Box<dyn Surface> = match DrmSurface::open(SurfaceConfig {
            present_mode: requested,
            ..SurfaceConfig::default()
        }) {
            Ok(drm) => {
                eprintln!(
                    "display DRM/KMS {} — {} buffers, {}",
                    drm.mode_name(),
                    drm.buffer_count(),
                    match drm.present_mode() {
                        PresentMode::Vsync => "vsync: tear-free, paced by vblank",
                        PresentMode::Immediate => "immediate: async flips, no vblank wait",
                    }
                );
                Box::new(drm)
            }
            Err(drm_error) => match FbdevSurface::open_first() {
                Ok(fb) => {
                    eprintln!("display fbdev {} ({})", fb.info(), fb.path().display());
                    eprintln!("        no DRM: {drm_error}");
                    Box::new(fb)
                }
                Err(fb_error) => {
                    return Err(format!("no display — DRM: {drm_error}; fbdev: {fb_error}").into());
                }
            },
        };

        let size = surface.size();
        let mut input = InputBackend::open_all(size)?;
        // Whatever this system is configured for, with `DENISE_KEYMAP` as an
        // override and F3 to switch at runtime.
        let (keymap, source) = input.set_layout_from_system();
        for device in input.devices() {
            eprintln!("input   {}: {}", device.capabilities(), device.name());
        }
        eprintln!("keymap  {} (from {source})", keymap.name);
        eprintln!("\nTab / Enter to drive it, F2 theme, F3 keyboard layout, Escape quits\n");

        let mut app = App::new(size);
        // Something must hold focus for a keyboard-only panel to be usable at all.
        app.ui.focus(Some(app.name));
        // Prime the clock before the first `poll`. `next_wake_ms` is only set by
        // `tick`, so a loop that polls first blocks with no timeout at all and the
        // caret never blinks — which is exactly what happened the first time this
        // ran on hardware.
        app.ui.tick(0);

        // Built once: the device set does not change, and `poll` updates each
        // entry's revents in place.
        let raw_fds = input.raw_fds();
        let borrowed: Vec<BorrowedFd<'_>> = raw_fds
            .iter()
            // SAFETY: every descriptor belongs to `input`, which outlives this loop
            // and holds each device open until the process exits.
            .map(|&fd| unsafe { BorrowedFd::borrow_raw(fd) })
            .collect();
        let mut poll_fds: Vec<PollFd<'_>> = borrowed
            .iter()
            .map(|fd| PollFd::new(fd, PollFlags::IN))
            .collect();

        let started = Instant::now();
        let deadline = started + Duration::from_secs(seconds);
        let mut events = Vec::new();
        let mut frames = 0u64;
        let mut wakeups = 0u64;
        let mut painted = 0u64;
        let mut input_events = 0u64;
        let mut timer_wakes = 0u64;

        // Shown on screen and switchable at runtime, because a layout set only by
        // an environment variable is a layout somebody forgets to set, and the
        // symptom — a Norwegian key typing a semicolon — reads as a broken
        // keyboard rather than a wrong setting.
        let mut layout_index = layout::BUILT_IN
            .iter()
            .position(|l| core::ptr::eq(*l, keymap))
            .unwrap_or(0);
        app.show_layout(keymap.name);

        while Instant::now() < deadline {
            let now = || started.elapsed().as_millis() as u64;

            // Block until input arrives, the caret owes a blink, or the run ends.
            // With nothing focused and nothing animating there is no timeout at
            // all and the process uses no CPU whatsoever.
            let timeout = poll_timeout(app.ui.next_wake_ms(), now(), deadline);
            poll(&mut poll_fds, timeout.as_ref())?;
            wakeups += 1;

            events.clear();
            input.poll(&mut events);
            if events.is_empty() {
                timer_wakes += 1;
            }
            input_events += events.len() as u64;
            app.ui.handle(&events);
            app.ui.tick(now());
            if !handle_shortcuts(&mut app, &events, &mut input, &mut layout_index) {
                break;
            }
            app.dispatch();

            if !app.ui.needs_paint() {
                continue;
            }

            // `acquire` may block until vblank. Read input again on the far side of
            // that wait so what gets drawn is as fresh as it can be.
            let mut frame = surface.acquire()?;
            events.clear();
            input.poll(&mut events);
            app.ui.handle(&events);
            app.ui.tick(now());
            app.dispatch();

            input_events += events.len() as u64;
            app.ui.paint(&mut frame);
            painted += app.ui.damage().iter().map(Rect::area).sum::<u64>();
            drop(frame);
            surface.present(app.ui.damage())?;
            app.ui.presented();
            frames += 1;
        }

        let elapsed = started.elapsed().as_secs_f64();
        let surface_area = Rect::from_size(size).area().max(1);
        eprintln!(
            "\n{frames} frames over {elapsed:.1}s, {wakeups} wake-ups, {} saves",
            app.saves
        );
        eprintln!(
            "repainted {:.2}% of the surface per drawn frame — the tree decided all of it",
            painted as f64 / (surface_area * frames.max(1)) as f64 * 100.0
        );
        eprintln!("{input_events} input events, {timer_wakes} wake-ups with nothing to read");
        Ok(())
    }

    /// How long `poll` may block: until the next caret blink, or until the run
    /// ends, whichever comes first. `None` blocks indefinitely.
    fn poll_timeout(next_wake: Option<u64>, now_ms: u64, deadline: Instant) -> Option<Timespec> {
        let until_deadline = deadline.saturating_duration_since(Instant::now());
        let wait = match next_wake {
            Some(at) => Duration::from_millis(at.saturating_sub(now_ms)).min(until_deadline),
            None => until_deadline,
        };
        Some(Timespec {
            tv_sec: wait.as_secs() as i64,
            tv_nsec: wait.subsec_nanos() as i64,
        })
    }

    /// Keys the application claims before the tree sees them. Returns `false` to
    /// quit.
    fn handle_shortcuts(
        app: &mut App,
        events: &[InputEvent],
        input: &mut InputBackend,
        layout_index: &mut usize,
    ) -> bool {
        for event in events {
            let InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            } = event
            else {
                continue;
            };
            match code {
                KeyCode::Escape => {
                    if !app.close_dialog() {
                        return false;
                    }
                }
                KeyCode::F2 => app.cycle_theme(),
                KeyCode::F3 => {
                    *layout_index = (*layout_index + 1) % layout::BUILT_IN.len();
                    let next = layout::BUILT_IN[*layout_index];
                    input.set_layout(next);
                    app.show_layout(next.name);
                }
                _ => {}
            }
        }
        true
    }
}
