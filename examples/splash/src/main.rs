//! The screen between the kernel handing over and the panel painting.
//!
//! ```text
//! /usr/local/bin/denise-splash                  # until the `denise` service is up
//! /usr/local/bin/denise-splash --watch kiosk --until 90
//! ```
//!
//! A panel that boots to black for ten seconds looks broken, and the machine
//! cannot say otherwise because nothing owns the screen yet. This owns it, says
//! what is happening, and gets out of the way.
//!
//! # fbdev, never DRM
//!
//! Every other bare-Linux example here opens DRM if the machine has it and falls
//! back to fbdev if not. This one refuses the upgrade on purpose. DRM has exactly
//! one master, and the process holding it when the real panel starts is the
//! process the real panel cannot start behind — a splash that made the
//! application it is covering for fail to launch would be a poor kind of splash.
//! Writes to `/dev/fb0` are harmless once somebody else is scanning out.
//!
//! It costs tearing on a screen that changes twice a second. That is the right
//! trade.
//!
//! # The framebuffer moves underneath it
//!
//! On a Raspberry Pi there are two of them in one boot: the firmware's, as
//! `simplefb`, at about 1.3 seconds, and then vc4's at about 7, with a different
//! size *and* a different pixel format. The second registration blanks whatever
//! the first was showing. So the geometry is re-read every frame and everything —
//! surface, tree, layout — is rebuilt when it changes, which is cheap because
//! there is so little of it.
//!
//! # Progress is real, not a guess
//!
//! The bar is not a timer dressed up. OpenRC leaves a symlink in
//! `/run/openrc/started` for every service it has started, and the runlevel
//! directories say how many there will be, so the fraction is a genuine count of
//! work done over work to do. When there is no OpenRC to ask, there is no bar —
//! an honest spinner is better than a fabricated percentage.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("a boot splash wants a Linux framebuffer; there is nothing to draw on here");
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::run()
}

#[cfg(target_os = "linux")]
mod app {
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    use denise::{Radius, Rect, Role, Size, Surface, Theme};
    use denise_fbdev::FbdevSurface;
    use denise_ui::widgets::{Align, Label, Panel, Progress, Spinner};
    use denise_ui::{Motion, NodeId, TextStyle, Ui};

    /// Where OpenRC records what it has started, and where it is told what to.
    const STARTED: &str = "/run/openrc/started";
    const RUNLEVELS: &str = "/etc/runlevels";

    /// How often the screen is reconsidered.
    ///
    /// Fast enough that the bar moves rather than jumps, slow enough that a board
    /// still bringing up its drivers is not competing with a redraw. Nothing here
    /// waits on input, because there is none to wait on.
    const TICK: Duration = Duration::from_millis(150);

    /// No message type: nothing here can be pressed.
    type Msg = ();

    pub fn run() -> ExitCode {
        let mut watch = String::from("denise");
        let mut until = 60u64;
        let mut font: Option<String> = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--watch" => watch = args.next().unwrap_or(watch),
                "--until" => until = args.next().and_then(|s| s.parse().ok()).unwrap_or(until),
                "--font" => font = args.next(),
                other => {
                    eprintln!("unknown argument {other}");
                    return ExitCode::FAILURE;
                }
            }
        }

        let face = system_font::load(font.as_deref());
        let boot = Boot::survey();
        let done = PathBuf::from(STARTED).join(&watch);
        let deadline = Instant::now() + Duration::from_secs(until);

        let mut screen: Option<Screen> = None;
        loop {
            // Rebuilt rather than resized: a framebuffer being replaced is a new
            // device with a new format, not the same one with different bounds.
            if screen.as_ref().is_none_or(|s| s.stale()) {
                screen = Screen::open(face.as_ref().map(|(name, _)| name.as_str()));
            }

            if let Some(screen) = screen.as_mut() {
                screen.update(&boot);
                if let Err(e) = screen.present() {
                    eprintln!("splash: {e}; giving up the screen");
                    return ExitCode::SUCCESS;
                }
            }

            // The panel is up: it owns the screen now, and anything painted here
            // would be either invisible or a fight.
            if done.exists() {
                eprintln!("splash: {watch} has the screen");
                return ExitCode::SUCCESS;
            }
            if Instant::now() >= deadline {
                eprintln!("splash: {watch} did not start within {until}s");
                return ExitCode::SUCCESS;
            }
            std::thread::sleep(TICK);
        }
    }

    /// What OpenRC has started, over what it was asked to start.
    struct Boot {
        /// Services named across every runlevel, counted once each.
        total: usize,
    }

    impl Boot {
        fn survey() -> Self {
            let mut names: Vec<String> = Vec::new();
            if let Ok(levels) = std::fs::read_dir(RUNLEVELS) {
                for level in levels.flatten() {
                    let Ok(services) = std::fs::read_dir(level.path()) else {
                        continue;
                    };
                    for service in services.flatten() {
                        let name = service.file_name().to_string_lossy().into_owned();
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
            Self { total: names.len() }
        }

        /// How far along, and the name of the most recent service to start.
        ///
        /// `None` for the fraction where there is no OpenRC to count — a bar that
        /// invents its own position is worse than no bar.
        fn progress(&self) -> (Option<f32>, Option<String>) {
            let Ok(entries) = std::fs::read_dir(STARTED) else {
                return (None, None);
            };
            let mut count = 0usize;
            let mut newest: Option<(std::time::SystemTime, String)> = None;
            for entry in entries.flatten() {
                count += 1;
                let name = entry.file_name().to_string_lossy().into_owned();
                let at = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                if newest.as_ref().is_none_or(|(seen, _)| at > *seen) {
                    newest = Some((at, name));
                }
            }
            let fraction = (self.total > 0).then(|| (count as f32 / self.total as f32).min(1.0));
            (fraction, newest.map(|(_, name)| name))
        }
    }

    /// One framebuffer, and the tree measured for it.
    struct Screen {
        surface: FbdevSurface,
        ui: Ui<Msg>,
        started: Instant,
        status: NodeId,
        bar: Option<NodeId>,
        /// The geometry this was built for, to notice when it is no longer true.
        geometry: (Size, u32),
    }

    impl Screen {
        fn open(font: Option<&str>) -> Option<Self> {
            let surface = FbdevSurface::open_first().ok()?;
            let size = surface.size();
            let geometry = (size, surface.info().bits_per_pixel);

            let mut ui: Ui<Msg> = Ui::new(size, Theme::BUILT_IN[1]);
            // The spinner is the only thing here that moves on its own.
            ui.set_motion(Motion::Every(80));

            let (heading, body) = match font.and_then(|path| system_font::load(Some(path))) {
                Some((_, source)) => {
                    let id = ui.add_font(source);
                    (
                        TextStyle {
                            font: id,
                            size_px: 28,
                        },
                        TextStyle {
                            font: id,
                            size_px: 15,
                        },
                    )
                }
                None => (TextStyle::built_in(24), TextStyle::built_in(16)),
            };

            let root = ui.root();
            let w = size.width as i32;
            let h = size.height as i32;

            // The mark from the logo: three translucent bitplanes, offset. Three
            // panels rather than a bitmap, because a bitmap would have to be
            // shipped at both of the sizes this display is going to be.
            let unit = (w.min(h) / 9).clamp(48, 150);
            let plane_w = unit * 3 / 2;
            let step = unit / 3;
            // Laid out downwards from a top chosen so the whole block ends up
            // centred — the parts have to be measured before the first one can be
            // placed, which is the price of having no layout engine.
            let mark_h = unit + step * 2;
            let block_h = mark_h + 44 + 34 + 22 + 22 + 20 + 8;
            let mark_x = (w - plane_w - step * 2) / 2;
            let mark_y = (h - block_h) / 2;
            for (index, role) in [Role::Accent, Role::Secondary, Role::Primary]
                .into_iter()
                .enumerate()
            {
                let offset = index as i32 * step;
                ui.add(
                    root,
                    Panel::filled(role).with_radius(Radius::Box),
                    Rect::new(mark_x + offset, mark_y + offset, plane_w, unit),
                )
                .expect("plane");
            }

            let text_w = (w * 2 / 5).clamp(320, 560);
            let text_x = (w - text_w) / 2;
            let mut y = mark_y + mark_h + 44;

            ui.add(
                root,
                Label::new("Denise")
                    .with_style(heading)
                    .with_align(Align::Center, Align::Center),
                Rect::new(text_x, y, text_w, 34),
            )
            .expect("title");
            y += 34 + 22;

            let status = ui
                .add(
                    root,
                    Label::new("Starting up")
                        .with_style(body)
                        .with_role(Role::Neutral)
                        .with_align(Align::Center, Align::Center),
                    Rect::new(text_x, y, text_w, 22),
                )
                .expect("status");
            y += 22 + 20;

            // A bar when the count is real, a spinner when it is not. Both say
            // "something is happening"; only one of them claims to know how much.
            let bar = if Boot::survey().total > 0 {
                Some(
                    ui.add(root, Progress::new(0.0), Rect::new(text_x, y, text_w, 8))
                        .expect("bar"),
                )
            } else {
                ui.add(root, Spinner::new(), Rect::new(w / 2 - 14, y - 6, 28, 28))
                    .expect("spinner");
                None
            };

            Some(Self {
                surface,
                ui,
                started: Instant::now(),
                status,
                bar,
                geometry,
            })
        }

        /// Whether the framebuffer this was measured for is still the one there.
        fn stale(&self) -> bool {
            match FbInfo::read() {
                Some(now) => now != self.geometry,
                // Unreadable sysfs is not a reason to throw a working screen
                // away; it is a reason to keep drawing on the one we have.
                None => false,
            }
        }

        fn update(&mut self, boot: &Boot) {
            let (fraction, name) = boot.progress();
            if let Some(label) = self.ui.widget_mut::<Label>(self.status) {
                let text = match name {
                    Some(name) => format!("Starting {name}"),
                    None => "Starting up".to_string(),
                };
                label.update(&text);
            }
            if let (Some(bar), Some(fraction)) = (self.bar, fraction)
                && let Some(widget) = self.ui.widget_mut::<Progress>(bar)
            {
                widget.update(fraction);
            }
            self.ui.tick(self.started.elapsed().as_millis() as u64);
        }

        fn present(&mut self) -> Result<(), Box<dyn std::error::Error>> {
            if !self.ui.needs_paint() {
                return Ok(());
            }
            let mut frame = self.surface.acquire()?;
            self.ui.paint(&mut frame);
            drop(frame);
            self.surface.present(self.ui.damage())?;
            self.ui.presented();
            Ok(())
        }
    }

    /// The current framebuffer geometry, straight from sysfs.
    ///
    /// Read from `/sys` rather than from the open surface: the question is what
    /// the framebuffer *is* now, and an open surface can only answer what it was
    /// when it was opened.
    struct FbInfo;

    impl FbInfo {
        fn read() -> Option<(Size, u32)> {
            let base = Path::new("/sys/class/graphics/fb0");
            let size = std::fs::read_to_string(base.join("virtual_size")).ok()?;
            let (w, h) = size.trim().split_once(',')?;
            let bpp = std::fs::read_to_string(base.join("bits_per_pixel")).ok()?;
            Some((
                Size::new(w.parse().ok()?, h.parse().ok()?),
                bpp.trim().parse().ok()?,
            ))
        }
    }
}
