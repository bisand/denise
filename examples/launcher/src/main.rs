//! A menu of the other demos, on the display they are about to take over.
//!
//! ```text
//! /usr/local/bin/denise-launcher
//! /usr/local/bin/denise-launcher "Video=/usr/local/bin/denise-video-player /root/promo.h264"
//! ```
//!
//! It lists the `denise-*` executables sitting beside it and makes a button of
//! each. An argument of the form `Label=command args...` overrides the button of
//! that name and leaves the rest alone, which is how a demo that needs a file to
//! open gets one: `denise-video-player` scans like any other and then exits 2,
//! because with no argument all it can do is print its usage.
//!
//! # The whole trick is giving the display back
//!
//! A kiosk demo owns the screen: DRM master, every evdev device, the console's
//! keyboard. Two of them at once is not a thing, so a launcher cannot start one
//! and keep running the way a desktop launcher does. What happens here instead is
//! that the menu is torn down completely — surface dropped, input dropped —
//! before the child starts, and built again from nothing when the child exits.
//! The process survives; none of its hardware does.
//!
//! That is why [`Menu`] is built inside [`show`] rather than held in [`run`]: the
//! borrow checker enforcing that the tree cannot outlive the display it was
//! measured for is exactly the invariant we want.
//!
//! **The console guard is the exception.** It is taken once in [`run`] and held
//! across everything, because dropping it puts the terminal back into text mode
//! and the login prompt would flash between every menu and every demo.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the launcher starts kiosk demos, so it needs Linux, a display and evdev");
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::run()
}

#[cfg(target_os = "linux")]
mod app {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitCode};
    use std::time::{Duration, Instant};

    use bare_linux::{
        Display, PresentMode, Waits, capture, mute_console, open_input, poll_timeout,
    };
    use denise::{
        ElementState, InputEvent, InputSource, KeyCode, Rect, Role, Size, Surface, Theme,
    };
    use denise_text::{GlyphSource, TrueTypeSource};
    use denise_ui::widgets::{Align, Button, Label, Panel};
    use denise_ui::{NodeId, TextStyle, Ui};

    const SHOT_PATH: &str = "/tmp/denise-launcher.ppm";

    /// How long a freshly opened menu ignores the keyboard.
    ///
    /// The keystroke that quit a demo is still happening when the menu takes the
    /// devices back: the key is held, the kernel is repeating it, and the release
    /// has not come yet. Without this, one press of Escape quits the demo *and*
    /// the menu behind it — which under a supervisor started the demo again, so
    /// the menu appeared to be unreachable.
    ///
    /// Long enough to outlast a normal press and the first repeat, short enough
    /// that nobody deliberately pressing a key notices it was dropped.
    const SETTLE: Duration = Duration::from_millis(400);

    /// Card padding, button height and the gap between buttons, in pixels.
    const PAD: i32 = 28;
    const ROW: i32 = 52;
    const GAP: i32 = 10;

    /// Executables beside us that are not demos, or are not worth a button.
    ///
    /// The prefix this scans for is shared with the machinery that *runs* the
    /// demos, because it is one product installed into one directory. So the
    /// list is not incidental tidying: `denise-run` is the supervisor that
    /// started this process and would recurse, `denise-console` is a getty
    /// wrapper expecting a tty argument, and `denise-splash` would put the boot
    /// screen up and never take it down. Each of those is a button that looks
    /// like a demo and is a trap.
    ///
    /// The rest are merely not worth a button: `denise-demo` is the shell runner
    /// that may well have started this one, and the probes print a line and exit
    /// — a button that blanks the screen for a tenth of a second is worse than no
    /// button at all.
    const NOT_DEMOS: &[&str] = &[
        "denise-launcher",
        "denise-run",
        "denise-console",
        "denise-splash",
        "denise-demo",
        "denise-video-probe",
        "denise-video-rawprobe",
    ];

    /// One button's worth: what to show, and what to run.
    struct Demo {
        label: String,
        command: PathBuf,
        args: Vec<String>,
    }

    /// The line under the buttons: what happened, and whether it was fine.
    ///
    /// The role is the point. A demo that refuses to start looks exactly like one
    /// that ran and was quit — both are a menu reappearing — and the only thing
    /// on screen that can tell them apart is the colour of this line.
    struct Status {
        text: String,
        role: Role,
    }

    impl Status {
        fn good(text: String) -> Self {
            Self {
                text,
                role: Role::Neutral,
            }
        }

        fn bad(text: String) -> Self {
            Self {
                text,
                role: Role::Error,
            }
        }
    }

    /// What the buttons send back. Nothing else can happen here.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Msg {
        Launch(usize),
        /// Swap the menu for the "are you sure" screen.
        AskExit,
        /// Yes: give the console back and stop.
        ConfirmExit,
        /// No: back to the menu.
        Cancel,
    }

    /// How a screen ended.
    enum Choice {
        Launch(usize),
        /// Escape. Under a supervisor this means "restart me", which is a way of
        /// starting over rather than a way out.
        Quit,
        /// Give the screen back and stay gone. See [`EXIT_TO_CONSOLE`].
        Console,
    }

    /// The exit status that means "the operator asked for the console".
    ///
    /// Whatever starts this has to agree: a supervisor that restarts on any exit
    /// would put the panel straight back up, and the request would look like a
    /// flicker. Nothing in this crate can enforce that, which is why it is a
    /// documented number rather than a clever mechanism.
    const EXIT_TO_CONSOLE: u8 = 3;

    /// What to tell the operator about getting the panel back.
    ///
    /// Read from a file rather than taken as text, for two reasons. The words
    /// belong to whoever installed this — only they know whether it is
    /// `rc-service`, `systemctl` or a power cycle — and `/etc/conf.d` cannot pass
    /// a sentence as one argument anyway.
    struct Hint(Vec<String>);

    impl Hint {
        fn read(path: Option<&str>) -> Self {
            let Some(path) = path else {
                return Self(Vec::new());
            };
            match std::fs::read_to_string(path) {
                Ok(text) => Self(text.lines().map(str::to_string).collect()),
                Err(e) => {
                    eprintln!("{path}: {e}");
                    Self(Vec::new())
                }
            }
        }

        /// The lines to show, never empty — a confirmation that cannot say how to
        /// undo itself is worse than no confirmation.
        fn lines(&self) -> Vec<&str> {
            if self.0.is_empty() {
                vec!["Nobody left instructions for starting it again."]
            } else {
                self.0.iter().map(String::as_str).collect()
            }
        }
    }

    /// The face, kept as bytes rather than as a parsed source.
    ///
    /// The tree is rebuilt after every demo and a `GlyphSource` is *moved* into
    /// it, so a parsed one cannot be reused. Bytes can, and re-parsing costs less
    /// than the file read we would otherwise do on every return to the menu.
    struct Face {
        name: String,
        bytes: Vec<u8>,
    }

    impl Face {
        /// Finds a face the same way every other example does, then keeps the file.
        fn find(requested: Option<&str>) -> Option<Self> {
            // `load` parses as well as reads, and the parsed half is thrown away
            // here. That is one wasted parse at startup in exchange for never
            // duplicating the directory list this crate exists to centralise.
            let (name, _) = system_font::load(requested)?;
            match std::fs::read(&name) {
                Ok(bytes) => Some(Self { name, bytes }),
                Err(e) => {
                    eprintln!("{name}: {e}");
                    None
                }
            }
        }

        fn source(&self) -> Option<Box<dyn GlyphSource>> {
            match TrueTypeSource::from_bytes(&self.name, &self.bytes) {
                Ok(source) => Some(Box::new(source)),
                Err(why) => {
                    eprintln!("{}: {why}", self.name);
                    None
                }
            }
        }
    }

    pub fn run() -> ExitCode {
        let mut font: Option<String> = None;
        let mut start: Option<String> = None;
        let mut hint: Option<String> = None;
        let mut listed: Vec<Demo> = Vec::new();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--font" => font = args.next(),
                "--start" => start = args.next(),
                "--exit-hint" => hint = args.next(),
                // An argument with an `=` starts an entry; bare ones after it are
                // that entry's arguments. Quoting the whole thing works too, but
                // this is the form that survives `/etc/conf.d`, where the value is
                // one shell word list and no amount of quoting inside it groups
                // anything back together.
                entry if entry.contains('=') => match parse_entry(entry) {
                    Some(demo) => listed.push(demo),
                    None => {
                        eprintln!("expected Label=command args..., not {entry}");
                        return ExitCode::FAILURE;
                    }
                },
                extra => match listed.last_mut() {
                    Some(demo) => demo.args.push(extra.to_string()),
                    None => {
                        eprintln!("{extra} belongs to no demo; entries look like Label=command");
                        return ExitCode::FAILURE;
                    }
                },
            }
        }

        let mut demos = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(discover(exe.parent()?)))
            .unwrap_or_default();
        merge(&mut demos, listed);

        if demos.is_empty() {
            eprintln!("no demos to offer: no denise-* executables beside this one,");
            eprintln!("and no Label=command arguments given");
            return ExitCode::FAILURE;
        }

        let face = Face::find(font.as_deref());
        if face.is_none() {
            eprintln!("no TrueType font found; using the built-in 8x8 bitmap font");
        }

        // Once, for the whole run. See the header: dropping this between demos is
        // what makes the login prompt flash.
        let _console = mute_console();

        // `--start` runs one demo before the menu is ever drawn, which is what a
        // panel that boots into its application wants: the menu is then the thing
        // you fall back to when that application quits, rather than a screen
        // somebody has to get past every morning.
        let mut status = match start {
            Some(label) => match demos.iter().position(|d| same_label(&d.label, &label)) {
                Some(index) => launch(&demos[index]),
                None => {
                    eprintln!("no demo called {label}; showing the menu instead");
                    Status::bad(format!("no demo called {label}"))
                }
            },
            None => Status::good(format!("{} demos found", demos.len())),
        };
        let hint = Hint::read(hint.as_deref());
        loop {
            match show(&demos, face.as_ref(), &status, &hint) {
                Ok(Choice::Quit) => return ExitCode::SUCCESS,
                Ok(Choice::Launch(chosen)) => status = launch(&demos[chosen]),
                // The console guard is dropped by returning, which is what puts
                // the terminal back into text mode — the operator is looking at a
                // console again before this process is finished exiting.
                Ok(Choice::Console) => {
                    eprintln!("launcher: the operator asked for the console");
                    return ExitCode::from(EXIT_TO_CONSOLE);
                }
                Err(e) => {
                    eprintln!("launcher: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    /// Folds command-line entries into the scanned list, by label.
    ///
    /// An entry whose label matches a scanned demo *replaces* it rather than
    /// adding a second button, which is the case that matters: `video-player`
    /// scans fine and then exits 2 because it wants a file, and naming it on the
    /// command line is how it gets one without losing every other button.
    fn merge(demos: &mut Vec<Demo>, listed: Vec<Demo>) {
        for demo in listed {
            match demos.iter().position(|d| same_label(&d.label, &demo.label)) {
                // The scanned label wins: `Video-player=` on a command line is
                // naming the button, not renaming it, and the button already says
                // "Video player".
                Some(index) => {
                    demos[index].command = demo.command;
                    demos[index].args = demo.args;
                }
                None => demos.push(demo),
            }
        }
    }

    /// Whether two labels name the same demo.
    ///
    /// Case-insensitive, and a hyphen counts as a space — the scan turns
    /// `denise-video-player` into `Video player`, and a label with a space in it
    /// cannot be written in `/etc/conf.d` without being split in two.
    fn same_label(a: &str, b: &str) -> bool {
        let key = |s: &str| s.to_ascii_lowercase().replace(['-', '_'], " ");
        key(a) == key(b)
    }

    /// Reads `Label=command args...`.
    fn parse_entry(entry: &str) -> Option<Demo> {
        let (label, rest) = entry.split_once('=')?;
        let mut words = rest.split_whitespace();
        let command = PathBuf::from(words.next()?);
        Some(Demo {
            label: label.trim().to_string(),
            command,
            args: words.map(str::to_string).collect(),
        })
    }

    /// Every `denise-*` executable in `dir` that is not on the blocklist.
    ///
    /// Sorted, because `read_dir` is in whatever order the filesystem feels like
    /// and a menu whose buttons move between boots is a menu nobody learns.
    fn discover(dir: &Path) -> Vec<Demo> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut demos: Vec<Demo> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?;
                let rest = name.strip_prefix("denise-")?;
                if NOT_DEMOS.contains(&name) {
                    return None;
                }
                // Directories have permission bits too, so both halves matter.
                let meta = entry.metadata().ok()?;
                if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
                    return None;
                }
                Some(Demo {
                    label: title_case(rest),
                    command: path,
                    args: Vec::new(),
                })
            })
            .collect();
        demos.sort_by(|a, b| a.command.cmp(&b.command));
        demos
    }

    /// `video-player` becomes `Video player`.
    fn title_case(name: &str) -> String {
        let spaced = name.replace(['-', '_'], " ");
        let mut chars = spaced.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => spaced,
        }
    }

    /// Runs one demo to completion and describes how it went.
    ///
    /// Standard error is inherited rather than captured, so whatever the demo says
    /// about the display it found lands in the same log as everything else. The
    /// return value is for the person looking at the panel, who has no log.
    fn launch(demo: &Demo) -> Status {
        eprintln!("\nlauncher: starting {}", demo.command.display());
        let result = Command::new(&demo.command).args(&demo.args).status();
        let outcome = match result {
            Ok(status) if status.success() => {
                Status::good(format!("{} exited cleanly", demo.label))
            }
            // A demo that wanted an argument is the common one, and it comes back
            // in well under a second — fast enough that the menu never looks like
            // it left, which is why the status line has to say so.
            Ok(status) => match status.code() {
                Some(code) => Status::bad(format!("{} exited with status {code}", demo.label)),
                // Escape quits most of these, but `kill` is how a stuck one goes,
                // and "signal" tells those two apart on the panel itself.
                None => Status::bad(format!("{} was stopped by a signal", demo.label)),
            },
            Err(e) => Status::bad(format!("could not start {}: {e}", demo.label)),
        };
        eprintln!("launcher: {}\n", outcome.text);
        outcome
    }

    /// Everything the menu owns for as long as one display does.
    struct Menu {
        ui: Ui<Msg>,
        started: Instant,
    }

    /// Opens the display, shows the menu, and returns what was chosen.
    ///
    /// `Ok(None)` is Escape. The display and the input devices are local to this
    /// function on purpose — see the header — so by the time it returns, the
    /// machine is free for whatever runs next.
    fn show(
        demos: &[Demo],
        face: Option<&Face>,
        status: &Status,
        hint: &Hint,
    ) -> Result<Choice, Box<dyn std::error::Error>> {
        let mut surface = open_display()?;
        let size = surface.size();
        let (mut input, _keymap) = open_input(size)?;

        let mut menu = Menu::list(size, demos, face, status);
        eprintln!("Tab moves, Enter starts, Escape quits");

        // Refreshed by `wait` whenever a device is opened or closed, which is
        // why this is a `Waits` and not a list built once.
        let mut waits = Waits::new(&input);

        // The first frame before anything blocks: a loop that waits before it
        // draws sets a mode on the display and then shows black.
        present(&mut surface, &mut menu, false)?;

        let deadline = Instant::now() + Duration::from_secs(60 * 60 * 24);
        let opened = Instant::now();
        let mut events = Vec::new();
        let mut shoot = false;

        loop {
            let now = menu.started.elapsed().as_millis() as u64;
            let timeout = poll_timeout(menu.ui.next_wake_ms(), now, deadline);
            waits.wait(&mut input, timeout.as_ref())?;

            events.clear();
            input.poll(&mut events);

            // Read and thrown away, not left unread: the descriptors have to be
            // drained or `poll` returns immediately for as long as they hold
            // anything, and the loop spins instead of sleeping.
            if opened.elapsed() < SETTLE {
                continue;
            }

            for event in &events {
                if let InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    ..
                } = event
                {
                    match code {
                        KeyCode::Escape => return Ok(Choice::Quit),
                        KeyCode::F12 => shoot = true,
                        _ => {}
                    }
                }
            }

            menu.ui.handle(&events);
            menu.ui.tick(now);
            // The two screens swap in place rather than through another trip
            // round `show`: reopening the display to ask one question would blank
            // the panel and set a mode again, for a screen the operator is meant
            // to be able to back out of.
            let message = menu.ui.drain_messages().next();
            match message {
                Some(Msg::Launch(chosen)) => return Ok(Choice::Launch(chosen)),
                Some(Msg::ConfirmExit) => return Ok(Choice::Console),
                Some(Msg::AskExit) => menu = Menu::confirm(size, face, hint),
                Some(Msg::Cancel) => menu = Menu::list(size, demos, face, status),
                None => {}
            }

            if menu.ui.needs_paint() || shoot {
                present(&mut surface, &mut menu, core::mem::take(&mut shoot))?;
            }
        }
    }

    /// Opens the display, allowing for the demo that just exited.
    ///
    /// The child held DRM master until its last descriptor closed, and that can
    /// happen a moment after `wait` returned. One retry loop here is cheaper than
    /// a launcher that dies every so often on the way back to its own menu.
    fn open_display() -> Result<Display, String> {
        let mut last = String::new();
        for attempt in 0..40 {
            match Display::open(PresentMode::Immediate) {
                Ok(display) => return Ok(display),
                Err(e) => {
                    last = e;
                    if attempt == 0 {
                        eprintln!("launcher: display still busy, waiting for it");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        Err(last)
    }

    /// Paints and presents one frame, capturing it first if one was asked for.
    ///
    /// The capture happens between painting and presenting, so what lands in the
    /// file is what the display is about to show rather than a re-render of it.
    /// With KMS on there is no reading the menu back off `/dev/fb0` afterwards,
    /// which makes this the only way to get a picture of it.
    fn present(
        surface: &mut Display,
        menu: &mut Menu,
        shoot: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut frame = surface.acquire()?;
        menu.ui.paint(&mut frame);
        if shoot {
            match capture(&frame, SHOT_PATH) {
                Ok(()) => eprintln!("wrote {SHOT_PATH}"),
                Err(e) => eprintln!("could not write {SHOT_PATH}: {e}"),
            }
        }
        drop(frame);
        surface.present(menu.ui.damage())?;
        menu.ui.presented();
        Ok(())
    }

    /// The three sizes a screen draws in, registered in its own tree.
    struct Styles {
        body: TextStyle,
        heading: TextStyle,
        small: TextStyle,
    }

    impl Menu {
        /// A tree with the fonts registered and one centred card of the height
        /// asked for, ready to have rows put into it.
        ///
        /// Both screens want exactly this and differ only in what goes inside, so
        /// the card being centred by arithmetic — there is no layout engine, and a
        /// display whose size is known before the first node exists does not need
        /// one — is written once here rather than twice below.
        fn card(size: Size, face: Option<&Face>, card_h: i32) -> (Ui<Msg>, NodeId, Styles, i32) {
            let mut ui: Ui<Msg> = Ui::new(size, Theme::BUILT_IN[1]);

            // Registered before the first node, so every widget is built with its
            // final style and nothing needs restyling afterwards.
            let styles = match face.and_then(Face::source) {
                Some(source) => {
                    let id = ui.add_font(source);
                    Styles {
                        body: TextStyle {
                            font: id,
                            size_px: 16,
                        },
                        heading: TextStyle {
                            font: id,
                            size_px: 24,
                        },
                        small: TextStyle {
                            font: id,
                            size_px: 13,
                        },
                    }
                }
                None => Styles {
                    body: TextStyle::built_in(16),
                    heading: TextStyle::built_in(24),
                    small: TextStyle::built_in(8),
                },
            };

            let root = ui.root();
            let card_w = (size.width as i32 * 2 / 5).clamp(360, 620);
            let card_h = card_h.min(size.height as i32 - 2 * PAD);
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
            (ui, card, styles, card_w - PAD * 2)
        }

        /// One button per demo, a status line, and the way out.
        fn list(size: Size, demos: &[Demo], face: Option<&Face>, status: &Status) -> Self {
            let list_h = demos.len() as i32 * ROW + (demos.len() as i32 - 1).max(0) * GAP;
            // Title, list, the exit row, the status line, and the padding around
            // all of it. The card is as tall as its contents — the other way round
            // from `panel`, because here the contents are a list whose length is
            // not known when the file is written.
            let (mut ui, card, styles, inner) = Self::card(
                size,
                face,
                PAD + 34 + 18 + list_h + 24 + ROW + 14 + 22 + PAD,
            );

            let mut y = PAD;
            ui.add(
                card,
                Label::new("Denise demos").with_style(styles.heading),
                Rect::new(PAD, y, inner, 30),
            )
            .expect("title");
            y += 34 + 18;

            for (index, demo) in demos.iter().enumerate() {
                ui.add(
                    card,
                    Button::new(demo.label.clone(), Msg::Launch(index)).with_style(styles.body),
                    Rect::new(PAD, y, inner, ROW),
                )
                .expect("demo button");
                y += ROW + GAP;
            }

            // Set apart from the demos and in the warning role, because it is the
            // one button here that does not come back on its own.
            y += 14;
            ui.add(
                card,
                Button::new("Exit to console", Msg::AskExit)
                    .with_style(styles.body)
                    .with_role(Role::Warning),
                Rect::new(PAD, y, inner, ROW),
            )
            .expect("exit button");
            y += ROW + 14;

            ui.add(
                card,
                Label::new(status.text.clone())
                    .with_style(styles.small)
                    .with_role(status.role)
                    .with_align(Align::Center, Align::Center),
                Rect::new(PAD, y, inner, 22),
            )
            .expect("status");

            Self {
                ui,
                started: Instant::now(),
            }
        }

        /// The screen that says what is about to happen and how to undo it.
        ///
        /// This exists because the button it guards is the only one on the panel
        /// that a person cannot get back from by pressing something else. Telling
        /// them how to return *before* they commit is the whole point; a message
        /// printed on the way out would be advice given to somebody who has
        /// already stopped looking at this screen.
        fn confirm(size: Size, face: Option<&Face>, hint: &Hint) -> Self {
            let lines = hint.lines();
            let hint_h = lines.len() as i32 * 22;
            let (mut ui, card, styles, inner) = Self::card(
                size,
                face,
                PAD + 34 + 18 + 24 + 14 + hint_h + 24 + ROW + PAD,
            );

            let mut y = PAD;
            ui.add(
                card,
                Label::new("Exit to console").with_style(styles.heading),
                Rect::new(PAD, y, inner, 30),
            )
            .expect("title");
            y += 34 + 18;

            ui.add(
                card,
                Label::new("The panel stops and the screen goes back to text.")
                    .with_style(styles.small)
                    .with_role(Role::Neutral),
                Rect::new(PAD, y, inner, 22),
            )
            .expect("explanation");
            y += 24 + 14;

            for line in lines {
                ui.add(
                    card,
                    Label::new(line.to_string())
                        .with_style(styles.small)
                        .with_role(Role::Info),
                    Rect::new(PAD, y, inner, 22),
                )
                .expect("hint line");
                y += 22;
            }
            y += 24;

            // Cancel first and wider: Tab starts on it, so the safe answer is the
            // one a person gets by pressing Enter without reading carefully.
            let cancel_w = inner * 3 / 5 - GAP;
            ui.add(
                card,
                Button::new("Cancel", Msg::Cancel).with_style(styles.body),
                Rect::new(PAD, y, cancel_w, ROW),
            )
            .expect("cancel");
            ui.add(
                card,
                Button::new("Exit", Msg::ConfirmExit)
                    .with_style(styles.body)
                    .with_role(Role::Error),
                Rect::new(PAD + cancel_w + GAP, y, inner - cancel_w - GAP, ROW),
            )
            .expect("confirm");

            Self {
                ui,
                started: Instant::now(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_entry_splits_into_a_command_and_its_arguments() {
            let demo = parse_entry("Video=/usr/local/bin/denise-video-player /root/promo.h264")
                .expect("parsed");
            assert_eq!(demo.label, "Video");
            assert_eq!(
                demo.command,
                PathBuf::from("/usr/local/bin/denise-video-player")
            );
            assert_eq!(demo.args, vec!["/root/promo.h264".to_string()]);
        }

        /// The case the video button failed on: a named entry has to *replace*
        /// the scanned one, not sit beside it as a second button.
        #[test]
        fn a_named_entry_replaces_the_scanned_demo_of_the_same_label() {
            let mut demos = vec![
                Demo {
                    label: "Gallery".into(),
                    command: PathBuf::from("/usr/local/bin/denise-gallery"),
                    args: Vec::new(),
                },
                Demo {
                    label: "Video player".into(),
                    command: PathBuf::from("/usr/local/bin/denise-video-player"),
                    args: Vec::new(),
                },
            ];
            merge(
                &mut demos,
                vec![
                    parse_entry("video player=/usr/local/bin/denise-video-player /p.h264").unwrap(),
                ],
            );
            assert_eq!(demos.len(), 2, "no second Video button");
            assert_eq!(demos[1].args, vec!["/p.h264".to_string()]);
            assert_eq!(demos[0].label, "Gallery", "the others are untouched");
        }

        /// The `/etc/conf.d` case: the value is one word list, so the file
        /// argument arrives as its own `argv` entry rather than attached.
        #[test]
        fn a_bare_argument_after_an_entry_belongs_to_it() {
            let mut demos = vec![Demo {
                label: "Video player".into(),
                command: PathBuf::from("/usr/local/bin/denise-video-player"),
                args: Vec::new(),
            }];
            let mut entry = parse_entry("Video-player=/usr/local/bin/denise-video-player").unwrap();
            entry.args.push("/home/bisand/promo.h264".to_string());
            merge(&mut demos, vec![entry]);
            assert_eq!(demos.len(), 1);
            assert_eq!(demos[0].args, vec!["/home/bisand/promo.h264".to_string()]);
            assert_eq!(demos[0].label, "Video player", "the button keeps its name");
        }

        #[test]
        fn a_hyphen_and_a_space_name_the_same_demo() {
            assert!(same_label("Video player", "video-player"));
            assert!(same_label("Table editor", "TABLE_EDITOR"));
            assert!(!same_label("Gallery", "Browser"));
        }

        #[test]
        fn an_unmatched_entry_becomes_a_new_button() {
            let mut demos = Vec::new();
            merge(
                &mut demos,
                vec![parse_entry("Slides=/usr/local/bin/x").unwrap()],
            );
            assert_eq!(demos.len(), 1);
            assert_eq!(demos[0].label, "Slides");
        }

        #[test]
        fn an_entry_without_an_equals_sign_is_not_one() {
            assert!(parse_entry("/usr/local/bin/denise-gallery").is_none());
        }

        #[test]
        fn a_label_is_the_binary_name_made_readable() {
            assert_eq!(title_case("video-player"), "Video player");
            assert_eq!(title_case("gallery"), "Gallery");
        }

        /// The blocklist is by full file name, not by the part after the prefix,
        /// which is the sort of thing that silently stops matching.
        #[test]
        fn the_blocklist_names_files_that_exist() {
            for name in NOT_DEMOS {
                assert!(name.starts_with("denise-"), "{name} would never be scanned");
            }
        }

        /// The machinery that runs the demos is installed beside them and shares
        /// their prefix, so every piece of it has to be named here. Getting this
        /// wrong puts a button on the panel that starts the supervisor, or the
        /// boot splash, and there is no way back from either.
        #[test]
        fn the_things_that_run_demos_are_not_demos() {
            for helper in ["denise-run", "denise-console", "denise-splash"] {
                assert!(
                    NOT_DEMOS.contains(&helper),
                    "{helper} would appear as a demo"
                );
            }
        }
    }
}
