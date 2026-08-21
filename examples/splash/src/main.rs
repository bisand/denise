//! The screen between the kernel handing over and the panel painting.
//!
//! ```text
//! /usr/local/bin/denise-splash                  # until the `denise` service is up
//! /usr/local/bin/denise-splash --watch kiosk --until 90
//! /usr/local/bin/denise-splash --title "Denise Raspberry Pi Demo" --say service
//! /usr/local/bin/denise-splash --after /dev/dri/card0     # wait for KMS first
//! ```
//!
//! `--say service` names whatever OpenRC started last, which is the line you want
//! the day a boot hangs: it stops on the thing it is stuck on. The default is
//! `quips`, for a panel people stand in front of, where "Starting dbus" means
//! nothing to anybody.
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
    use std::io::Read;
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

    /// What the line under the title says while the bar fills.
    ///
    /// Naming the service is the useful one the day a boot hangs, because the
    /// line stops on whatever it is stuck on. The other one is for a panel people
    /// stand in front of, where "Starting dbus" means nothing to anybody and the
    /// twelve seconds may as well be entertaining.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Say {
        Quips,
        Service,
    }

    /// Read in order, not at random, and indexed by the bar rather than a clock —
    /// so they march with the progress, never repeat, and the last one lands as
    /// the bar fills. A timer would let a stalled boot look busy, which is a lie
    /// this screen is in no position to tell.
    ///
    /// Half of these are things that actually went wrong bringing this board up.
    /// The first line and the last line, either side of a few from [`MIDDLE`].
    ///
    /// Fixed on purpose. The opener is what a cold screen says first and the
    /// closer has to arrive with the last of the bar — shuffling those away would
    /// trade the one thing this sequence does well for variety nobody asked the
    /// splash to have.
    const OPENER: &str = "Asking the firmware for a framebuffer";
    const CLOSER: &str = "Almost certainly nearly ready";

    /// How many of [`MIDDLE`] appear in one boot.
    ///
    /// Eight, plus the two fixed ones, over roughly twelve seconds: about a
    /// second and a bit each, which is long enough to read and short enough that
    /// the line is clearly keeping up with the bar.
    const MIDDLE_SHOWN: usize = 8;

    /// Drawn from at random each boot, so the panel does not recite the same
    /// twelve lines to the same person every morning.
    ///
    /// Half of these are things that actually went wrong bringing this board up.
    const MIDDLE: &[&str] = &[
        "Counting bitplanes",
        "Negotiating with HDMI",
        "Convincing the GPU it has a monitor",
        "Waking the mouse, which was asleep",
        "Rounding the corners",
        "Blaming the power supply",
        "Teaching pixels to agree on a colour",
        "Persuading 900 MHz of ARM to hurry",
        "Looking for a font that is not eight by eight",
        "Politely evicting the console",
        "Reticulating scanlines",
        "Checking whether the cable is in",
        "Explaining overscan to a television",
        "Deciding which framebuffer is in charge",
        "Waiting for something called a vblank",
        "Dividing by the refresh rate",
    ];

    /// This boot's lines: the opener, a shuffled few, the closer.
    fn quips() -> Vec<&'static str> {
        let mut middle: Vec<&'static str> = MIDDLE.to_vec();
        // Fisher-Yates with eight bytes of urandom behind an xorshift. A boot
        // splash does not need a crate for this, and at the point it runs there
        // is not much else to ask: the clock has not been set, so seeding from
        // the time would give the same order every morning.
        // Eight bytes, by `read_exact`. Not `fs::read`, which reads to end of
        // file: /dev/urandom has no end, so that allocates until the kernel
        // intervenes, and the splash dies before it draws anything.
        let mut seed = [0u8; 8];
        let mut state = std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut seed))
            .ok()
            .map(|()| u64::from_le_bytes(seed))
            .filter(|seed| *seed != 0)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        for i in (1..middle.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            middle.swap(i, (state % (i as u64 + 1)) as usize);
        }
        middle.truncate(MIDDLE_SHOWN);

        let mut lines = Vec::with_capacity(MIDDLE_SHOWN + 2);
        lines.push(OPENER);
        lines.extend(middle);
        lines.push(CLOSER);
        lines
    }

    /// Seconds since boot, for putting these messages beside `dmesg`.
    ///
    /// The wall clock is worse than useless this early: `swclock` has not run, so
    /// the first frames are stamped 1 January.
    fn uptime() -> f32 {
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse().ok())
            .unwrap_or(0.0)
    }

    pub fn run() -> ExitCode {
        let mut watch = String::from("denise");
        let mut until = 60u64;
        let mut linger = 4u64;
        let mut say = Say::Quips;
        let mut clock = false;
        let mut after: Option<String> = None;
        let mut title = String::from("Denise");
        let mut font: Option<String> = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--watch" => watch = args.next().unwrap_or(watch),
                "--until" => until = args.next().and_then(|s| s.parse().ok()).unwrap_or(until),
                "--linger" => linger = args.next().and_then(|s| s.parse().ok()).unwrap_or(linger),
                "--title" => title = args.next().unwrap_or(title),
                "--clock" => clock = true,
                "--after" => after = args.next(),
                "--say" => {
                    say = match args.next().as_deref() {
                        Some("service") => Say::Service,
                        Some("quips") | None => Say::Quips,
                        Some(other) => {
                            eprintln!("--say takes quips or service, not {other}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
                "--font" => font = args.next(),
                other => {
                    eprintln!("unknown argument {other}");
                    return ExitCode::FAILURE;
                }
            }
        }

        eprintln!("splash: up at {:.1}s", uptime());
        let face = system_font::load(font.as_deref());
        let boot = Boot::survey();
        let done = PathBuf::from(STARTED).join(&watch);
        let deadline = Instant::now() + Duration::from_secs(until);

        // Nothing is drawn until this exists. On a Pi the firmware puts up a
        // framebuffer within two seconds and vc4 replaces it around seven, and
        // the replacement reprograms the HDMI pipeline — the sink drops its lock
        // and takes a second or two to find it again. Painting before that means
        // a glimpse of splash, a blank, and then the splash again, which reads as
        // a fault. Waiting means it appears once and stays.
        //
        // A machine with no KMS coming never sees the path appear, so this gives
        // up and paints on whatever there is.
        if let Some(path) = after.as_deref() {
            let path = Path::new(path);
            let give_up = Instant::now() + Duration::from_secs(until);
            while !path.exists() && Instant::now() < give_up {
                std::thread::sleep(TICK);
            }
            eprintln!(
                "splash: waited for {} until {:.1}s ({})",
                path.display(),
                uptime(),
                if path.exists() { "there" } else { "gave up" }
            );
        }

        let lines = quips();
        let mut screen: Option<Screen> = None;
        let mut complained = false;
        loop {
            // Rebuilt rather than resized: a framebuffer being replaced is a new
            // device with a new format, not the same one with different bounds.
            if screen.as_ref().is_none_or(|s| s.stale()) {
                if let Some(old) = screen.as_ref() {
                    eprintln!(
                        "splash: framebuffer changed from {}x{} at {}bpp, at {:.1}s",
                        old.geometry.0.width,
                        old.geometry.0.height,
                        old.geometry.1,
                        uptime()
                    );
                }
                let began = screen.as_ref().and_then(|old| old.began);
                screen = Screen::open(
                    face.as_ref().map(|(name, _)| name.as_str()),
                    &title,
                    lines.clone(),
                    began,
                    clock,
                );
                match screen.as_ref() {
                    Some(new) => eprintln!(
                        "splash: drawing on {}x{} at {}bpp, at {:.1}s",
                        new.geometry.0.width,
                        new.geometry.0.height,
                        new.geometry.1,
                        uptime()
                    ),
                    None => {
                        if !complained {
                            eprintln!("splash: no framebuffer to open, at {:.1}s", uptime());
                            complained = true;
                        }
                    }
                }
            }

            if let Some(screen) = screen.as_mut() {
                screen.update(&boot, say);
                if let Err(e) = screen.present() {
                    eprintln!("splash: {e}; giving up the screen");
                    return ExitCode::SUCCESS;
                }
            }

            // The panel is up: it owns the screen now, and anything painted here
            // would be either invisible or a fight.
            if done.exists() {
                eprintln!("splash: {watch} started at {:.1}s", uptime());
                // Not "exit now". The service being *marked* started is the
                // moment its supervisor was launched, not the moment its first
                // frame reaches the display — the panel still has to start, open
                // DRM, find a font and lay a tree out. Leaving before then hands
                // the viewer a black screen for exactly that long.
                //
                // Staying is free: once the panel has DRM master these writes go
                // into the fbdev emulation's own buffer and never reach scanout.
                let stop = Instant::now() + Duration::from_secs(linger);
                while Instant::now() < stop {
                    if let Some(screen) = screen.as_mut() {
                        screen.update(&boot, say);
                        let _ = screen.present();
                    }
                    std::thread::sleep(TICK);
                }
                eprintln!("splash: done at {:.1}s", uptime());
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
        /// Seconds since boot, drawn in the corner. Off by default; on when
        /// somebody is pointing a camera at the screen, because a frame that says
        /// what time it is turns a film into a measurement.
        clock: Option<NodeId>,
        /// This boot's lines. Chosen once and carried across a framebuffer
        /// change, so the sequence does not restart when vc4 takes over.
        quips: Vec<&'static str>,
        /// How far along the boot was when the first line was shown. Carried
        /// across a framebuffer change for the same reason as the lines.
        began: Option<f32>,
        surface: FbdevSurface,
        ui: Ui<Msg>,
        started: Instant,
        status: NodeId,
        bar: Option<NodeId>,
        /// The geometry this was built for, to notice when it is no longer true.
        geometry: (Size, u32),
    }

    impl Screen {
        fn open(
            font: Option<&str>,
            title: &str,
            quips: Vec<&'static str>,
            began: Option<f32>,
            clock: bool,
        ) -> Option<Self> {
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
                Label::new(title.to_string())
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

            let clock_node = clock.then(|| {
                ui.add(
                    root,
                    Label::new("0.0 s")
                        .with_style(body)
                        .with_role(Role::Warning)
                        .with_align(Align::Center, Align::Center),
                    Rect::new(w - 220, h - 60, 180, 26),
                )
                .expect("clock")
            });

            Some(Self {
                clock: clock_node,
                quips,
                began,
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

        fn update(&mut self, boot: &Boot, say: Say) {
            let (fraction, name) = boot.progress();
            let text = match say {
                Say::Service => match name {
                    Some(name) => format!("Starting {name}"),
                    None => "Starting up".to_string(),
                },
                // Indexed by the bar, so the last line arrives with the last of
                // the fill — but measured from wherever the bar *was* when this
                // screen first painted, not from zero. Waiting for KMS costs the
                // first fifth of a boot, and indexing from zero spends that fifth
                // on lines nobody is there to read: the opener was never once
                // seen on the test board.
                Say::Quips => {
                    let now = fraction.unwrap_or(0.0);
                    let from = *self.began.get_or_insert(now);
                    let span = (1.0 - from).max(f32::EPSILON);
                    let at = ((now - from) / span).clamp(0.0, 1.0) * self.quips.len() as f32;
                    self.quips[(at as usize).min(self.quips.len() - 1)].to_string()
                }
            };
            if let Some(label) = self.ui.widget_mut::<Label>(self.status) {
                label.update(&text);
            }
            if let Some(node) = self.clock {
                let now = format!("{:.1} s", uptime());
                if let Some(label) = self.ui.widget_mut::<Label>(node) {
                    label.update(&now);
                }
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
