//! The gallery's tree: every widget live, and the theme editor beside them.
//!
//! Platform-independent on purpose, like every `app.rs` in this repository:
//! this file never learns whether it is drawing into a window or onto a
//! scanout buffer. The backends live in `main.rs`.
//!
//! # The one trick worth stealing
//!
//! The theme editor does not talk to the widgets and the widgets do not know
//! the editor exists. The sliders rebuild a [`Theme`] from nine seed colours
//! and hand it to [`Ui::set_theme`]; every widget names *roles*, so the whole
//! surface follows. That is the entire mechanism — there is no second channel.

use std::time::Instant;

use crate::clock;

use denise::theme::{AA, ColorScheme, Metrics, Radius, contrast_x100};
use denise::{Color, Rect, Role, Size, Theme};
use denise_render::Canvas;
use denise_ui::widgets::{
    Accordion, Alert, Align, Avatar, Badge, Button, Carousel, Checkbox, Collapse, Column, Divider,
    Fit, Image, Label, List, ListItem, Presence, Progress, RadialProgress, RadioGroup, Rating,
    Select, Slider, Spinner, Table, Tabs, TextInput, Timeline, TimelineItem, Toggle, open_select,
};
use denise_ui::{
    Animation, Event, EventCtx, Handled, Motion, NodeId, PaintCtx, Side, TextStyle, Ui, Wake,
    Widget,
};

const HEADER_H: i32 = 52;
const SIDEBAR_W: i32 = 300;
const GAP: i32 = 12;
/// Left edge inside a panel.
const PAD: i32 = 16;

/// The nine colours a theme is derived from, and what to call them.
///
/// The order matches [`Theme::from_seeds`]'s parameters. The role listed is
/// where the seed lands unchanged, which is how the editor reads a built-in
/// theme's seeds back out of it.
const SEEDS: [(Role, &str); 9] = [
    (Role::Base100, "Base"),
    (Role::Primary, "Primary"),
    (Role::Secondary, "Secondary"),
    (Role::Accent, "Accent"),
    (Role::Neutral, "Neutral"),
    (Role::Info, "Info"),
    (Role::Success, "Success"),
    (Role::Warning, "Warning"),
    (Role::Error, "Error"),
];

/// What the widgets send back.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A built-in theme was chosen in the sidebar list.
    UseTheme(usize),
    /// The seed dropdown wants its option list opened.
    OpenSeeds,
    /// A seed was chosen in that popup.
    Seed(usize),
    /// One channel slider of the current seed moved. `0..=2` is R, G, B.
    Channel(usize, f32),
    Dark(bool),
    Touch(bool),
    Roundness(f32),
    Depth(f32),
    Surprise,
    // The gallery floor.
    Level(f32),
    Stars(f32),
    Spin(bool),
    Mode(usize),
    Tab(usize),
    PickRow(usize),
    OpenRow(usize),
    GridRow(usize),
    OpenModes,
    Chose(usize),
    Page(usize),
    Fold(usize),
    Remember(bool),
    ShowToast,
    ShowDialog,
    CloseDialog,
    ShowDrawer,
    ToggleShelf,
}

// `Slider` and `Collapse` take a plain `fn` constructor, so a message that
// needs an index carried alongside the payload gets one small fn per index.
fn chan_r(v: f32) -> Message {
    Message::Channel(0, v)
}
fn chan_g(v: f32) -> Message {
    Message::Channel(1, v)
}
fn chan_b(v: f32) -> Message {
    Message::Channel(2, v)
}
fn fold_0(_: bool) -> Message {
    Message::Fold(0)
}
fn fold_1(_: bool) -> Message {
    Message::Fold(1)
}
fn fold_2(_: bool) -> Message {
    Message::Fold(2)
}

/// A rectangle of one colour: the seed swatch.
///
/// The one widget this application builds itself, here as the demonstration
/// that building one is small: `paint` against [`PaintCtx`], nothing else
/// required. It shows a *colour*, which is the single job the role-based
/// widgets rightly refuse.
struct Swatch {
    color: Color,
    /// The frame around the colour, in physical pixels: the application scales
    /// its own drawing exactly as it scales its own rectangles.
    inset: i32,
}

impl Widget<Message> for Swatch {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let b = ctx.bounds;
        let radius = ctx.theme.radius(Radius::Field);
        let i = self.inset;
        canvas.fill_rounded_rect(b, radius, ctx.theme.color(Role::Base300));
        canvas.fill_rounded_rect(
            Rect::new(b.x + i, b.y + i, b.width - 2 * i, b.height - 2 * i),
            (radius - i).max(0),
            self.color,
        );
    }

    // Unused here, but shows the other half of the trait: events arrive
    // already routed, and a widget answers with messages, not callbacks.
    fn on_event(&mut self, _event: &Event<'_>, _ctx: &mut EventCtx<'_, Message>) -> Handled {
        Handled::No
    }
}

/// The nodes that get written to after startup.
/// The date and time, ticking.
///
/// A widget rather than a label the application pokes, because only a widget can
/// name its own deadline. [`Wake::At`] the next second boundary is not touched by
/// [`Motion`], so this keeps time even when the tree is told not to animate — and
/// it costs one wake a second rather than one a frame.
struct Clock {
    /// The drawing is a label's; only the knowing-when is this widget's.
    face: Label,
}

impl Clock {
    fn new(style: TextStyle) -> Self {
        Self {
            face: Label::new("")
                .with_style(style)
                .with_role(Role::Base300)
                .with_align(Align::Center, Align::Center),
        }
    }

    /// Reads the clock and asks to be woken when the second turns.
    fn tick(&mut self, now_ms: u64) -> Animation {
        let Some(now) = clock::now() else {
            return Animation::NONE;
        };
        Animation {
            repaint: self.face.update(&now.text),
            // The remainder of *this* second, not a flat thousand: waking on the
            // boundary keeps the displayed second honest, where a fixed interval
            // would drift until the label lagged the world by most of a second.
            next: Wake::At(now_ms.saturating_add(1000 - u64::from(now.sub_ms))),
        }
    }
}

impl Widget<Message> for Clock {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        Widget::<Message>::paint(&self.face, ctx, canvas);
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        self.tick(now_ms)
    }

    /// Under `Motion::None` a widget is asked to land and stop. A clock cannot:
    /// its next deadline is a fact about the world rather than about animation,
    /// and reduced motion is a request not to move things, not a request to stop
    /// telling the time.
    fn snap(&mut self, now_ms: u64) -> Animation {
        self.tick(now_ms)
    }
}

struct Nodes {
    theme_name: NodeId,
    contrast: NodeId,
    theme_list: NodeId,
    swatch: NodeId,
    seed_select: NodeId,
    seed_hex: NodeId,
    channels: [NodeId; 3],
    channel_values: [NodeId; 3],
    level_bar: NodeId,
    level_ring: NodeId,
    stars: NodeId,
    spinner: NodeId,
    tab_body: NodeId,
    mode_select: NodeId,
    grid_caption: NodeId,
}

pub struct App {
    pub ui: Ui<Message>,
    /// The nine seeds the custom theme is derived from.
    seeds: [Color; 9],
    /// `Some(i)` while an untouched built-in is active; the first edit clears
    /// it and the seeds become the theme.
    builtin: Option<usize>,
    /// Physical pixels per logical pixel. Every number in this file is logical;
    /// this is the only place the difference exists.
    scale: f32,
    dark: bool,
    touch: bool,
    roundness: f32,
    depth: f32,
    /// Which seed the channel sliders edit.
    seed: usize,
    accordion: Accordion,
    nodes: Nodes,
    started: Instant,
    body: TextStyle,
    heading: TextStyle,
    small: TextStyle,
    toasts_sent: u32,
}

impl App {
    /// Builds the tree for a surface of `size` **physical** pixels at `scale`.
    ///
    /// The layout below is written in logical pixels and multiplied here, once —
    /// Denise's whole DPI story, and the reason a panel drawn for an 800×480 Pi
    /// is legible on a 2× laptop instead of half size. The theme scales its
    /// metrics, every layout rectangle goes through [`Rect::scaled`] on its way
    /// into the tree (see [`App::add`]), and text sizes are multiplied like any
    /// other measurement.
    pub fn new(
        size: Size,
        scale: f32,
        font: Option<(String, Box<dyn denise_text::GlyphSource>)>,
        motion: Motion,
    ) -> Self {
        let start_theme = 1; // dark
        let mut ui: Ui<Message> = Ui::new(size, Theme::BUILT_IN[start_theme].scaled(scale));
        // How fast everything here moves — the spinner, the carousel's slides,
        // the drawer, the toggles — in one call, before a single node exists.
        ui.set_motion(motion);
        let px = |v: u16| ((v as f32) * scale + 0.5) as u16;

        // Registered before the first node exists, so every widget is built
        // with its final style and nothing needs restyling afterwards.
        let (body, heading, small) = match font {
            Some((name, source)) => {
                eprintln!("using {name}");
                let id = ui.add_font(source);
                (
                    TextStyle {
                        font: id,
                        size_px: px(15),
                    },
                    TextStyle {
                        font: id,
                        size_px: px(22),
                    },
                    TextStyle {
                        font: id,
                        size_px: px(12),
                    },
                )
            }
            None => {
                eprintln!("no TrueType font found; using the built-in 8x8 bitmap font");
                (
                    TextStyle::built_in(px(16)),
                    TextStyle::built_in(px(24)),
                    TextStyle::built_in(px(8)),
                )
            }
        };

        let theme = *ui.theme();
        let seeds = SEEDS.map(|(role, _)| theme.color(role));

        let mut app = App {
            ui,
            seeds,
            builtin: Some(start_theme),
            scale,
            dark: true,
            touch: false,
            roundness: 1.0,
            depth: 4.0,
            seed: 1, // primary: the first thing anybody wants to change
            accordion: Accordion::new([]),
            nodes: Nodes {
                theme_name: NodeId::default(),
                contrast: NodeId::default(),
                theme_list: NodeId::default(),
                swatch: NodeId::default(),
                seed_select: NodeId::default(),
                seed_hex: NodeId::default(),
                channels: [NodeId::default(); 3],
                channel_values: [NodeId::default(); 3],
                level_bar: NodeId::default(),
                level_ring: NodeId::default(),
                stars: NodeId::default(),
                spinner: NodeId::default(),
                tab_body: NodeId::default(),
                mode_select: NodeId::default(),
                grid_caption: NodeId::default(),
            },
            started: Instant::now(),
            body,
            heading,
            small,
            toasts_sent: 0,
        };
        // Built against the logical extent of that surface: the numbers below
        // are the ones the layout was designed with, on any display.
        app.build(app.logical(size));
        app.apply_theme();
        app
    }

    /// A physical surface size back in the logical units the layout is written in.
    fn logical(&self, size: Size) -> Size {
        let to = |v: u32| ((v as f32) / self.scale + 0.5) as u32;
        Size::new(to(size.width), to(size.height))
    }

    /// A logical measurement in physical pixels, for the few numbers that do not
    /// arrive as a [`Rect`].
    fn px(&self, v: i32) -> i32 {
        ((v as f32) * self.scale + 0.5) as i32
    }

    // ------------------------------------------------------------ the tree

    fn build(&mut self, size: Size) {
        let root = self.ui.root();
        let w = size.width as i32;
        let h = size.height as i32;

        // Header: name, active theme, and the contrast verdict.
        self.add(
            root,
            Label::new("Denise").with_style(self.heading),
            Rect::new(PAD, 12, 200, 28),
        );
        self.add(
            root,
            Label::new("every widget, live — the theme editor is on the left")
                .with_style(self.small)
                .with_role(Role::Base300),
            Rect::new(120, 20, 400, 16),
        );
        // Centred, between the subtitle and the theme name, because a clock in a
        // top bar is furniture: it should be findable without being the first
        // thing the eye lands on.
        let clock = self.add(
            root,
            Clock::new(self.body),
            Rect::new(w / 2 - 90, 14, 180, 24),
        );
        // Nothing else will ask on its behalf: a widget joins the animating set
        // by requesting it, and this one has no event to request it from.
        self.ui.request_animation(clock);

        self.nodes.theme_name = self.add(
            root,
            Label::new("")
                .with_style(self.body)
                .with_align(Align::End, Align::Center),
            Rect::new(w - 360, 14, 240, 24),
        );
        self.nodes.contrast = self.add(
            root,
            Badge::new("")
                .with_role(Role::Success)
                .with_style(self.small),
            Rect::new(w - 104, 14, 88, 26),
        );
        self.ui.set_tooltip(
            self.nodes.contrast,
            "worst surface/content contrast in this theme",
        );

        self.build_sidebar(Rect::new(GAP, HEADER_H, SIDEBAR_W, h - HEADER_H - GAP));

        // The gallery floor: a scrollable stacked column of sections. The
        // stack places them, the viewport scrolls them, and no rectangle in
        // this file knows how tall the whole thing came to.
        let content = self.add(
            root,
            denise_ui::Void,
            Rect::new(
                GAP + SIDEBAR_W + GAP,
                HEADER_H,
                w - SIDEBAR_W - 3 * GAP,
                h - HEADER_H - GAP,
            ),
        );
        self.ui.set_scrollable(content, true);
        self.ui.set_stack(content, GAP);
        let cw = w - SIDEBAR_W - 3 * GAP;

        self.section_roles(content, cw);
        self.section_form(content, cw);
        self.section_values(content, cw);
        self.section_choice(content, cw);
        self.section_data(content, cw);
        self.section_pictures(content, cw);
        self.section_folding(content, cw);
    }

    fn build_sidebar(&mut self, at: Rect) {
        let root = self.ui.root();
        let side = self.add(root, denise_ui::widgets::Panel::default(), at);
        self.ui.set_scrollable(side, true);
        let w = at.width - 2 * PAD;

        self.add(
            side,
            Label::new("Theme").with_style(self.heading),
            Rect::new(PAD, 12, w, 26),
        );
        self.nodes.theme_list = self.add(
            side,
            List::new(
                [
                    ListItem::new("Light"),
                    ListItem::new("Dark"),
                    ListItem::new("High contrast"),
                ],
                Message::UseTheme,
            )
            .with_style(self.body)
            .with_selected(self.builtin),
            Rect::new(PAD, 46, w, 96),
        );

        self.add(
            side,
            Divider::labelled("seeds").with_style(self.small),
            Rect::new(PAD, 152, w, 20),
        );
        self.nodes.swatch = self.add(
            side,
            Swatch {
                color: self.seeds[self.seed],
                inset: self.px(2),
            },
            Rect::new(PAD, 182, 44, 36),
        );
        self.nodes.seed_select = self.add(
            side,
            Select::new(SEEDS.map(|(_, name)| name), Message::OpenSeeds)
                .with_selected(Some(self.seed))
                .with_style(self.body),
            Rect::new(PAD + 52, 182, w - 52, 36),
        );
        self.nodes.seed_hex = self.add(
            side,
            Label::new("")
                .with_style(self.small)
                .with_role(Role::Base300),
            Rect::new(PAD, 222, w, 16),
        );
        for (i, name) in ["R", "G", "B"].iter().enumerate() {
            let y = 246 + i as i32 * 32;
            self.add(
                side,
                Label::new(*name).with_style(self.body),
                Rect::new(PAD, y + 4, 18, 18),
            );
            let message = [chan_r, chan_g, chan_b][i];
            self.nodes.channels[i] = self.add(
                side,
                Slider::new(0.0, 255.0, 0.0, message),
                Rect::new(PAD + 24, y, w - 74, 26),
            );
            self.nodes.channel_values[i] = self.add(
                side,
                Label::new("")
                    .with_style(self.small)
                    .with_align(Align::End, Align::Center),
                Rect::new(PAD + w - 44, y + 4, 44, 18),
            );
        }

        self.add(
            side,
            Divider::labelled("shape").with_style(self.small),
            Rect::new(PAD, 348, w, 20),
        );
        self.add(
            side,
            Toggle::new("Dark derivation", Message::Dark)
                .with_checked(self.dark)
                .with_style(self.body),
            Rect::new(PAD, 378, w, 28),
        );
        self.add(
            side,
            Toggle::new("Touch targets", Message::Touch).with_style(self.body),
            Rect::new(PAD, 412, w, 28),
        );
        self.add(
            side,
            Label::new("Radius").with_style(self.small),
            Rect::new(PAD, 450, 70, 18),
        );
        self.add(
            side,
            Slider::new(0.0, 2.0, self.roundness, Message::Roundness),
            Rect::new(PAD + 74, 446, w - 74, 26),
        );
        self.add(
            side,
            Label::new("Depth").with_style(self.small),
            Rect::new(PAD, 482, 70, 18),
        );
        self.add(
            side,
            Slider::new(0.0, 16.0, self.depth, Message::Depth).with_step(1.0),
            Rect::new(PAD + 74, 478, w - 74, 26),
        );

        self.add(
            side,
            Button::new("Surprise me", Message::Surprise)
                .with_role(Role::Accent)
                .with_style(self.body),
            Rect::new(PAD, 520, w, 40),
        );
        self.add(
            side,
            Label::new("seeds in, theme out, contrast derived")
                .with_style(self.small)
                .with_role(Role::Base300),
            Rect::new(PAD, 568, w, 32),
        );
    }

    /// A section: a panel on the stack with a heading, returning the panel.
    fn section(&mut self, content: NodeId, cw: i32, height: i32, title: &str) -> NodeId {
        let panel = self.add(
            content,
            denise_ui::widgets::Panel::default(),
            Rect::new(0, 0, cw, height),
        );
        self.add(
            panel,
            Label::new(title).with_style(self.heading),
            Rect::new(PAD, 10, 400, 26),
        );
        panel
    }

    fn section_roles(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 164, "Roles");
        let roles = [
            Role::Primary,
            Role::Secondary,
            Role::Accent,
            Role::Neutral,
            Role::Info,
            Role::Success,
            Role::Warning,
            Role::Error,
        ];
        let bw = (cw - 2 * PAD - 7 * 8) / 8;
        for (i, role) in roles.into_iter().enumerate() {
            let button = self.add(
                s,
                Button::new(role_name(role), Message::ShowToast)
                    .with_role(role)
                    .with_style(self.body),
                Rect::new(PAD + i as i32 * (bw + 8), 46, bw, 38),
            );
            if i == 0 {
                self.ui
                    .set_tooltip(button, "every widget names roles, never colours");
            }
        }
        let off = self.add(
            s,
            Button::new("Disabled", Message::ShowToast).with_style(self.body),
            Rect::new(PAD, 96, bw, 38),
        );
        self.ui.set_enabled(off, false);
        let mut x = PAD + bw + 12;
        for (text, role) in [
            ("3", Role::Primary),
            ("ON", Role::Success),
            ("WAIT", Role::Warning),
            ("ERR", Role::Error),
        ] {
            let badge = Badge::new(text).with_role(role).with_style(self.small);
            let bw = badge.preferred_width(self.ui.text_mut());
            let bh = badge.preferred_height(self.ui.text_mut());
            self.add(s, badge, Rect::new(x, 104, bw, bh));
            x += bw + 8;
        }
    }

    fn section_form(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 216, "Form");
        self.add(
            s,
            Label::new("Name").with_style(self.small),
            Rect::new(PAD, 42, 200, 16),
        );
        self.add(
            s,
            TextInput::<Message>::new()
                .with_placeholder("Ola Nordmann")
                .with_style(self.body),
            Rect::new(PAD, 60, 280, 36),
        );
        self.add(
            s,
            Label::new("PIN").with_style(self.small),
            Rect::new(316, 42, 200, 16),
        );
        self.add(
            s,
            TextInput::<Message>::new()
                .with_password(true)
                .with_style(self.body),
            Rect::new(316, 60, 200, 36),
        );

        self.add(
            s,
            Checkbox::new("Remember me", Message::Remember)
                .with_checked(true)
                .with_style(self.body),
            Rect::new(PAD, 108, 220, 28),
        );
        self.add(
            s,
            Checkbox::new("Send report", Message::Remember).with_style(self.body),
            Rect::new(246, 108, 220, 28),
        );
        let locked = self.add(
            s,
            Checkbox::new("Locked", Message::Remember)
                .with_checked(true)
                .with_style(self.body),
            Rect::new(476, 108, 200, 28),
        );
        self.ui.set_enabled(locked, false);
        self.add(
            s,
            Toggle::new("Night mode", Message::Remember)
                .with_checked(true)
                .with_style(self.body),
            Rect::new(PAD, 144, 220, 28),
        );
        self.add(
            s,
            Toggle::new("Mute", Message::Remember).with_style(self.body),
            Rect::new(246, 144, 220, 28),
        );

        self.add(
            s,
            RadioGroup::new(["Automatic", "Manual", "Off"], Message::Mode)
                .with_selected(0)
                .with_style(self.body),
            Rect::new(706, 42, 200, 90),
        );
        let alert = Alert::new(Role::Info, "Text is live: click a field and type")
            .with_icon('i')
            .with_style(self.small);
        let ah = alert.preferred_height(self.ui.text_mut(), cw - 706 - PAD);
        self.add(s, alert, Rect::new(706, 140, cw - 706 - PAD, ah));

        self.add(
            s,
            Label::new("Kjærlighet på Øy — æøå ÆØÅ 0123456789 ±25 °C")
                .with_style(self.small)
                .with_role(Role::Base300),
            Rect::new(PAD, 184, 500, 18),
        );
    }

    fn section_values(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 150, "Values");
        // One slider drives a bar, a ring and a number: the message loop on
        // display. The slider emits, the application assigns, the setters stay
        // silent — nothing here can echo.
        self.add(
            s,
            Slider::new(0.0, 1.0, 0.35, Message::Level),
            Rect::new(PAD, 52, 320, 26),
        );
        self.nodes.level_bar = self.add(s, Progress::new(0.35), Rect::new(PAD, 92, 320, 12));
        self.nodes.level_ring = self.add(
            s,
            RadialProgress::new(0.35)
                .with_label("35 %")
                .with_style(self.small),
            Rect::new(376, 42, 92, 92),
        );

        self.nodes.stars = self.add(
            s,
            Rating::new(3.0, Message::Stars),
            Rect::new(508, 52, 180, 30),
        );
        self.add(
            s,
            Rating::<Message>::display(4.3),
            Rect::new(508, 92, 180, 30),
        );

        self.nodes.spinner = self.add(s, Spinner::new(), Rect::new(736, 46, 48, 48));
        self.ui.request_animation(self.nodes.spinner);
        self.add(
            s,
            Toggle::new("Awake", Message::Spin)
                .with_checked(true)
                .with_style(self.body),
            Rect::new(800, 56, 140, 28),
        );
        let _ = cw;
    }

    fn section_choice(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 200, "Choice");
        self.nodes.mode_select = self.add(
            s,
            Select::new(["Automatic", "Manual", "Off"], Message::OpenModes)
                .with_placeholder("Choose a mode")
                .with_style(self.body),
            Rect::new(PAD, 52, 220, 36),
        );

        let tabs = Tabs::new(["Overview", "Details", "About"], Message::Tab).with_style(self.body);
        let tw = tabs.preferred_width(self.ui.text_mut());
        self.add(s, tabs, Rect::new(276, 52, tw, 34));
        self.nodes.tab_body = self.add(
            s,
            Label::new(TAB_BODIES[0])
                .with_style(self.body)
                .with_role(Role::Base300),
            Rect::new(276, 100, 380, 22),
        );

        self.add(
            s,
            List::new(
                [
                    ListItem::new("Network").with_trailing("DHCP"),
                    ListItem::new("Display").with_trailing("70 %"),
                    ListItem::new("Defrost").with_trailing("off").disabled(),
                    ListItem::new("About").with_leading(">"),
                ],
                Message::PickRow,
            )
            .on_activate(Message::OpenRow)
            .with_selected(Some(1))
            .with_style(self.body),
            Rect::new(706, 44, cw - 706 - PAD, 140),
        );
        let _ = cw;
    }

    fn section_data(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 250, "Data");
        self.nodes.grid_caption = self.add(
            s,
            Label::new("50 rows, 6 on screen: the table windows its data")
                .with_style(self.small)
                .with_role(Role::Base300)
                .with_align(Align::End, Align::Center),
            Rect::new(cw - 420 - PAD, 16, 420, 16),
        );
        self.add(
            s,
            Table::new(
                [
                    Column::new("Name", 200),
                    Column::flex("Role"),
                    Column::new("No", 60).align_end(),
                ],
                Message::GridRow,
            )
            .with_rows((0..50).map(|i| {
                [
                    format!("Person {i}"),
                    if i % 3 == 0 { "Operator" } else { "Guard" }.to_owned(),
                    (100 + i).to_string(),
                ]
            }))
            .with_row_height(self.px(30))
            .with_style(self.body),
            Rect::new(PAD, 44, 520, 190),
        );
        self.add(
            s,
            Timeline::new([
                TimelineItem::new("Pump started")
                    .with_time("12:01")
                    .with_role(Role::Success),
                TimelineItem::new("Pressure reached").with_time("12:04:30"),
                TimelineItem::new("Valve opening").with_time("12:06"),
                TimelineItem::new("Valve open").pending(),
                TimelineItem::new("Done").pending(),
            ])
            .with_row_height(self.px(36))
            .with_style(self.body),
            Rect::new(570, 44, cw - 570 - PAD, 190),
        );
    }

    fn section_pictures(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 216, "Pictures");
        let (px, size) = picture();
        let mut x = PAD;
        for (fit, caption) in [
            (Fit::Contain, "Contain"),
            (Fit::Cover, "Cover"),
            (Fit::Center, "Center"),
            (Fit::Fill, "Fill"),
        ] {
            self.add(
                s,
                Image::new(px.clone(), size).with_fit(fit),
                Rect::new(x, 44, 84, 84),
            );
            self.add(
                s,
                Label::new(caption)
                    .with_style(self.small)
                    .with_align(Align::Center, Align::Center),
                Rect::new(x, 132, 84, 16),
            );
            x += 96;
        }

        let mut x = PAD;
        self.add(
            s,
            Avatar::new(px.clone(), size).with_presence(Presence::Online),
            Rect::new(x, 156, 48, 48),
        );
        x += 58;
        for (name, presence) in [
            ("Ola Nordmann", Some(Presence::Online)),
            ("Kari Traa", None),
            ("Øystein Åsen", Some(Presence::Busy)),
        ] {
            let mut avatar = Avatar::initials(name).with_style(self.body);
            if let Some(p) = presence {
                avatar = avatar.with_presence(p);
            }
            self.add(s, avatar, Rect::new(x, 156, 48, 48));
            x += 58;
        }
        self.add(
            s,
            Avatar::initials("Nils Aas")
                .with_ring(Role::Primary)
                .with_style(self.body),
            Rect::new(x, 156, 48, 48),
        );

        // Three tinted pages; arrows, keys and a drag all slide it.
        let mut rotator = Carousel::new(Message::Page);
        for tint in [0xFF6688_u32, 0x88CC66, 0x6688EE] {
            let (mut page, size) = picture();
            for word in &mut page {
                *word = (*word & 0xFF00_0000)
                    | (((*word & 0x00FE_FEFE) >> 1) + (tint & 0x00FF_FFFF) / 2);
            }
            rotator = rotator.with_picture(page, size);
        }
        self.add(s, rotator, Rect::new(cw - 300 - PAD, 44, 300, 160));
    }

    fn section_folding(&mut self, content: NodeId, cw: i32) {
        let s = self.section(content, cw, 260, "Folding & overlays");
        let stack = self.add(s, denise_ui::Void, Rect::new(PAD, 44, 560, 200));
        self.ui.set_stack(stack, 6);
        let mut sections = Vec::new();
        for (title, message) in [
            ("Network", fold_0 as fn(bool) -> Message),
            ("Display", fold_1),
            ("About", fold_2),
        ] {
            let section = self.add(
                stack,
                Collapse::new(title, message).with_style(self.body),
                Rect::new(0, 0, 560, 34 + 52),
            );
            self.add(
                section,
                Label::new("The body is any subtree; the fold is a layout tween")
                    .with_style(self.small)
                    .with_role(Role::Base300),
                Rect::new(20, 40, 520, 18),
            );
            sections.push(section);
        }
        // The first section starts open and the rest folded — folded *silently*,
        // by writing the folded layout, because this is initial state and not
        // a change: there is nothing on screen yet for a tween to animate
        // from, and a snapshot never ticks.
        let theme = *self.ui.theme();
        for (i, &section) in sections.iter().enumerate() {
            let expanded = self.ui.layout(section).expect("section layout");
            let Some(collapse) = self.ui.widget_mut::<Collapse<Message>>(section) else {
                continue;
            };
            collapse.set_expanded_height(expanded.height);
            let header = collapse.header_height(&theme);
            if i > 0 {
                collapse.set_open_silent(false);
                self.ui.set_layout(
                    section,
                    Rect::new(expanded.x, expanded.y, expanded.width, header),
                );
            }
        }
        self.accordion = Accordion::new(sections.iter().copied());
        // Tell the controller which one is open; the layouts already agree.
        self.accordion.toggle(&mut self.ui, 0);

        self.add(
            s,
            Button::new("Toast", Message::ShowToast)
                .with_role(Role::Info)
                .with_style(self.body),
            Rect::new(cw - 220 - PAD, 44, 220, 40),
        );
        self.add(
            s,
            Button::new("Dialog…", Message::ShowDialog)
                .with_role(Role::Primary)
                .with_style(self.body),
            Rect::new(cw - 220 - PAD, 96, 220, 40),
        );
        self.add(
            s,
            Button::new("Drawer", Message::ShowDrawer)
                .with_role(Role::Neutral)
                .with_style(self.body),
            Rect::new(cw - 220 - PAD, 148, 220, 40),
        );
        self.add(
            s,
            Button::new("Shelf", Message::ToggleShelf)
                .with_role(Role::Neutral)
                .with_style(self.body),
            Rect::new(cw - 220 - PAD, 200, 220, 40),
        );
    }

    /// `Ui::add` with the panic this application wants: the tree is built once
    /// at startup from constants, so a failure here is a bug, not a condition.
    ///
    /// `layout` is logical and comes out physical. Doing the multiplication here
    /// rather than at each of the seventy-odd call sites is why the layout above
    /// still reads as a layout — and [`Rect::scaled`] scales *edges*, so panels
    /// designed to touch still touch at 1.5× and 1.75×.
    fn add(&mut self, parent: NodeId, widget: impl Widget<Message>, layout: Rect) -> NodeId {
        self.ui
            .add(parent, widget, layout.scaled(self.scale))
            .expect("build")
    }

    // ------------------------------------------------------- theme plumbing

    /// Rebuilds the theme from the editor's state and hands it to the tree.
    fn apply_theme(&mut self) {
        let theme = match self.builtin {
            Some(i) => Theme::BUILT_IN[i],
            None => {
                let s = self.seeds;
                Theme::from_seeds(
                    "custom",
                    if self.dark {
                        ColorScheme::Dark
                    } else {
                        ColorScheme::Light
                    },
                    s[0],
                    s[1],
                    s[2],
                    s[3],
                    s[4],
                    s[5],
                    s[6],
                    s[7],
                    s[8],
                )
            }
        };
        // Shape applies over whichever palette is active.
        let base = if self.touch {
            Metrics::TOUCH
        } else {
            Metrics::DEFAULT
        };
        let round = |v: i32| ((v as f32) * self.roundness + 0.5) as i32;
        let metrics = Metrics {
            radius_selector: round(base.radius_selector),
            radius_field: round(base.radius_field),
            radius_box: round(base.radius_box),
            ..base
        };
        // Scaled last: the editor's shape controls are logical like everything
        // else, and the theme reaches the widgets in physical pixels.
        let theme = theme
            .with_metrics(metrics)
            .with_depth(self.depth as u8)
            .scaled(self.scale);
        self.ui.set_theme(theme);
        self.refresh_readouts();
    }

    /// Everything in the chrome that reports on the theme, brought current.
    fn refresh_readouts(&mut self) {
        let theme = *self.ui.theme();
        let name = match self.builtin {
            Some(_) => theme.name,
            None => "custom",
        };
        if let Some(label) = self.ui.widget_mut::<Label>(self.nodes.theme_name) {
            label.set_text(name);
        }

        // The worst surface/content pair, as a ratio. The derivation aims at
        // AA, so this going red takes deliberate sabotage — which the editor
        // permits, and the badge then says so instead of anybody hoping.
        let worst = Role::SURFACES
            .iter()
            .map(|&s| contrast_x100(theme.color(s), theme.content_of(s)))
            .min()
            .unwrap_or(0);
        if let Some(badge) = self.ui.widget_mut::<Badge>(self.nodes.contrast) {
            badge.set_text(format!("{}.{:01}:1", worst / 100, (worst % 100) / 10));
            badge.set_role(if worst >= AA {
                Role::Success
            } else {
                Role::Error
            });
        }
        self.ui.invalidate(self.nodes.contrast);

        if let Some(list) = self.ui.widget_mut::<List<Message>>(self.nodes.theme_list) {
            list.set_selected(self.builtin);
        }
        self.ui.invalidate(self.nodes.theme_list);

        let seed = self.seeds[self.seed];
        if let Some(swatch) = self.ui.widget_mut::<Swatch>(self.nodes.swatch) {
            swatch.color = seed;
        }
        self.ui.invalidate(self.nodes.swatch);
        if let Some(hex) = self.ui.widget_mut::<Label>(self.nodes.seed_hex) {
            hex.set_text(format!(
                "{} · #{:02X}{:02X}{:02X}",
                SEEDS[self.seed].1, seed.r, seed.g, seed.b
            ));
        }
        self.ui.invalidate(self.nodes.seed_hex);
        for (i, value) in [seed.r, seed.g, seed.b].into_iter().enumerate() {
            if let Some(slider) = self
                .ui
                .widget_mut::<Slider<Message>>(self.nodes.channels[i])
            {
                slider.set_value(value as f32);
            }
            self.ui.invalidate(self.nodes.channels[i]);
            if let Some(label) = self.ui.widget_mut::<Label>(self.nodes.channel_values[i]) {
                label.set_text(format!("{value}"));
            }
            self.ui.invalidate(self.nodes.channel_values[i]);
        }
    }

    // ------------------------------------------------------------- messages

    pub fn handle(&mut self, _now_ms: u64) {
        let messages: Vec<Message> = self.ui.drain_messages().collect();
        for message in messages {
            self.on_message(message);
        }
    }

    pub fn on_message(&mut self, message: Message) {
        match message {
            Message::UseTheme(i) => {
                self.builtin = Some(i);
                let theme = Theme::BUILT_IN[i];
                self.seeds = SEEDS.map(|(role, _)| theme.color(role));
                self.dark = theme.scheme == ColorScheme::Dark;
                self.apply_theme();
            }
            Message::OpenSeeds => {
                open_select(&mut self.ui, self.nodes.seed_select, Message::Seed);
            }
            Message::Seed(i) => {
                self.seed = i;
                if let Some(select) = self
                    .ui
                    .widget_mut::<Select<Message>>(self.nodes.seed_select)
                {
                    select.set_selected(Some(i));
                }
                self.ui.invalidate(self.nodes.seed_select);
                self.ui.close_popup();
                self.refresh_readouts();
            }
            Message::Channel(c, value) => {
                let seed = &mut self.seeds[self.seed];
                let value = value as u8;
                match c {
                    0 => seed.r = value,
                    1 => seed.g = value,
                    _ => seed.b = value,
                }
                // The first edit leaves the built-in: the seeds are now the
                // theme.
                self.builtin = None;
                self.apply_theme();
            }
            Message::Dark(dark) => {
                self.dark = dark;
                self.builtin = None;
                self.apply_theme();
            }
            Message::Touch(touch) => {
                self.touch = touch;
                self.apply_theme();
            }
            Message::Roundness(r) => {
                self.roundness = r;
                self.apply_theme();
            }
            Message::Depth(d) => {
                self.depth = d;
                self.apply_theme();
            }
            Message::Surprise => {
                self.surprise();
            }

            Message::Level(value) => {
                if let Some(bar) = self.ui.widget_mut::<Progress>(self.nodes.level_bar) {
                    bar.set_value(value);
                }
                self.ui.invalidate(self.nodes.level_bar);
                if let Some(ring) = self.ui.widget_mut::<RadialProgress>(self.nodes.level_ring) {
                    ring.set_value(value);
                    ring.set_label(format!("{:.0} %", value * 100.0));
                }
                self.ui.invalidate(self.nodes.level_ring);
            }
            Message::Stars(_) => {} // the widget shows its own value
            Message::Spin(awake) => {
                // Hiding the spinner is what stops its animation from keeping
                // the device awake; `Ui::animating` proves it from a test.
                self.ui.set_visible(self.nodes.spinner, awake);
                if awake {
                    self.ui.request_animation(self.nodes.spinner);
                }
            }
            Message::Mode(_) | Message::Remember(_) | Message::PickRow(_) | Message::Page(_) => {}
            Message::Tab(i) => {
                if let Some(label) = self.ui.widget_mut::<Label>(self.nodes.tab_body) {
                    label.set_text(TAB_BODIES[i.min(TAB_BODIES.len() - 1)]);
                }
                self.ui.invalidate(self.nodes.tab_body);
            }
            Message::OpenRow(i) => {
                self.ui.toast(format!("Opening row {i}…"), Role::Info);
            }
            Message::GridRow(i) => {
                if let Some(label) = self.ui.widget_mut::<Label>(self.nodes.grid_caption) {
                    label.set_text(format!("Person {i} selected — row {} of 50", i + 1));
                }
                self.ui.invalidate(self.nodes.grid_caption);
            }
            Message::OpenModes => {
                open_select(&mut self.ui, self.nodes.mode_select, Message::Chose);
            }
            Message::Chose(i) => {
                if let Some(select) = self
                    .ui
                    .widget_mut::<Select<Message>>(self.nodes.mode_select)
                {
                    select.set_selected(Some(i));
                }
                self.ui.invalidate(self.nodes.mode_select);
                self.ui.close_popup();
            }
            Message::Fold(i) => {
                let mut accordion = std::mem::replace(&mut self.accordion, Accordion::new([]));
                accordion.toggle(&mut self.ui, i);
                self.accordion = accordion;
            }
            Message::ShowToast => {
                self.toasts_sent += 1;
                let (text, role) = match self.toasts_sent % 3 {
                    0 => ("Saved at 12:01", Role::Success),
                    1 => ("Sensor 3 is not answering", Role::Error),
                    _ => ("9 new messages", Role::Info),
                };
                self.ui.toast(text, role);
            }
            Message::ShowDialog => self.open_dialog(),
            Message::CloseDialog => {
                self.ui.pop_scene();
            }
            Message::ShowDrawer => self.open_drawer(),
            Message::ToggleShelf => self.toggle_shelf(),
        }
    }

    fn open_dialog(&mut self) {
        // `Ui::size` is physical; centring is arithmetic on the logical extent
        // like every other rectangle here, and `add` scales the result.
        let size = self.logical(self.ui.size());
        let scene = self.ui.push_scene(110);
        let w = 420;
        let h = 170;
        let dialog = self.add(
            scene,
            denise_ui::widgets::Panel::default().with_border(Role::Primary, self.px(2)),
            Rect::new(
                (size.width as i32 - w) / 2,
                (size.height as i32 - h) / 2,
                w,
                h,
            ),
        );
        self.add(
            dialog,
            Label::new("Save changes?")
                .with_style(self.heading)
                .with_align(Align::Center, Align::Center),
            Rect::new(20, 18, w - 40, 30),
        );
        self.add(
            dialog,
            Label::new("nothing below takes input while this is up")
                .with_style(self.small)
                .with_role(Role::Base300)
                .with_align(Align::Center, Align::Center),
            Rect::new(20, 56, w - 40, 20),
        );
        let yes = self.add(
            dialog,
            Button::new("Yes", Message::CloseDialog).with_style(self.body),
            Rect::new(20, h - 62, (w - 60) / 2, 44),
        );
        self.add(
            dialog,
            Button::new("Cancel", Message::CloseDialog)
                .with_role(Role::Neutral)
                .with_style(self.body),
            Rect::new(40 + (w - 60) / 2, h - 62, (w - 60) / 2, 44),
        );
        self.ui.focus(Some(yes));
    }

    fn open_drawer(&mut self) {
        // The tree places a drawer itself, so this width is physical; its
        // contents below are logical and go through `add` like the rest.
        let Some(drawer) = self.ui.push_drawer(Side::After, self.px(300)) else {
            return;
        };
        self.add(
            drawer,
            Label::new("A drawer").with_style(self.heading),
            Rect::new(PAD, 16, 200, 26),
        );
        self.add(
            drawer,
            Label::new("Escape, or a press on the dim, slides it out.")
                .with_style(self.small)
                .with_role(Role::Base300),
            Rect::new(PAD, 52, 300 - 2 * PAD, 40),
        );
        for (i, text) in ["Network", "Display", "About this panel"]
            .iter()
            .enumerate()
        {
            self.add(
                drawer,
                Button::new(*text, Message::ShowToast).with_style(self.body),
                Rect::new(PAD, 108 + i as i32 * 50, 300 - 2 * PAD, 40),
            );
        }
    }

    /// The overlay that is *not* modal, and the whole demonstration is what
    /// keeps working while it is up: the fields above stay focused and typable,
    /// the buttons behind it still press, and nothing dims. A drawer offers the
    /// opposite of each, which is why both are here to compare.
    fn toggle_shelf(&mut self) {
        if self.ui.shelf_open() {
            self.ui.close_shelf();
            return;
        }
        let Some(shelf) = self.ui.push_shelf(Side::Below, self.px(96)) else {
            return;
        };
        // The tree places a shelf itself, so that height was physical; the
        // contents below are logical and go through `add` like the rest.
        let width = self.logical(self.ui.size()).width as i32;
        self.add(
            shelf,
            denise_ui::widgets::Panel::default(),
            Rect::new(0, 0, width, 96),
        );
        self.add(
            shelf,
            Label::new("A shelf").with_style(self.heading),
            Rect::new(PAD, 14, 200, 26),
        );
        self.add(
            shelf,
            Label::new(
                "Focus and input carry on underneath — type in the fields, press the buttons.",
            )
            .with_style(self.small)
            .with_role(Role::Base300),
            Rect::new(PAD, 46, 560, 20),
        );
        self.add(
            shelf,
            Button::new("Close", Message::ToggleShelf).with_style(self.body),
            Rect::new(width - 120 - PAD, 28, 120, 40),
        );
    }

    /// New seeds from a corner of the colour wheel nobody planned.
    fn surprise(&mut self) {
        // A xorshift over the elapsed nanoseconds: all this needs is "not the
        // same every press", which is not worth a dependency.
        let mut state = self.started.elapsed().as_nanos() as u64 | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let hue = |n: u64| hsv((n % 360) as f32, 0.55 + (n % 40) as f32 / 100.0, 0.85);
        self.seeds[0] = if self.dark {
            hsv((next() % 360) as f32, 0.25, 0.14)
        } else {
            hsv((next() % 360) as f32, 0.06, 0.95)
        };
        for seed in self.seeds.iter_mut().skip(1) {
            *seed = hue(next());
        }
        self.builtin = None;
        self.apply_theme();
    }

    /// The built-in theme F2 cycles to next.
    pub fn next_builtin(&self) -> usize {
        match self.builtin {
            Some(i) => (i + 1) % Theme::BUILT_IN.len(),
            None => 0,
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

const TAB_BODIES: [&str; 3] = [
    "One tab strip, one node, one tab stop.",
    "Arrow keys move the selection; Enter is not needed.",
    "Built from the same Label everything else uses.",
];

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Primary => "Primary",
        Role::Secondary => "Secondary",
        Role::Accent => "Accent",
        Role::Neutral => "Neutral",
        Role::Info => "Info",
        Role::Success => "Success",
        Role::Warning => "Warning",
        Role::Error => "Error",
        _ => "Role",
    }
}

/// A little landscape, drawn with the rasteriser itself: the gallery needs a
/// picture and has no business shipping an asset file to get one.
fn picture() -> (Vec<u32>, Size) {
    use denise::{PixelFormat, Point};

    let size = Size::new(64, 48);
    let mut pixels = vec![0u32; (size.width * size.height) as usize];
    {
        let mut c = Canvas::from_pixels(&mut pixels, size, size.width, PixelFormat::Argb8888)
            .expect("picture buffer");
        c.clear(Color::from_rgb888(0x89B4FA));
        c.fill_rect(Rect::new(0, 36, 64, 12), Color::from_rgb888(0xA6E3A1));
        c.fill_circle(Point::new(48, 12), 8, Color::from_rgb888(0xF9E2AF));
        c.fill_rounded_rect(Rect::new(8, 22, 20, 14), 3, Color::from_rgb888(0xF38BA8));
    }
    // Every pixel is opaque, so the buffer is already premultiplied.
    (pixels, size)
}

/// Hue/saturation/value to a colour, enough for the surprise button.
#[allow(clippy::many_single_char_names)]
fn hsv(h: f32, s: f32, v: f32) -> Color {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h as u32 / 60) % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::{ElementState, InputEvent, KeyCode, Modifiers, Point, PointerButton};

    fn app() -> App {
        // No font: the tests are about wiring, and font discovery is I/O.
        App::new(Size::new(1280, 800), 1.0, None, Motion::default())
    }

    /// The same tree on a 2× display is the same tree, twice the size — the
    /// property the whole scaling path exists for, checked at the front door.
    #[test]
    fn a_hidpi_surface_gets_the_same_layout_at_twice_the_size() {
        let one = App::new(Size::new(1280, 800), 1.0, None, Motion::default());
        let two = App::new(Size::new(2560, 1600), 2.0, None, Motion::default());

        let (a, b) = (
            one.ui.layout(one.nodes.contrast).expect("badge at 1x"),
            two.ui.layout(two.nodes.contrast).expect("badge at 2x"),
        );
        assert_eq!(b, a.scaled(2.0), "the badge is where 2× says it is");
        assert_eq!(
            two.ui.theme().metrics,
            one.ui.theme().metrics.scaled(2.0),
            "the furniture scaled with it"
        );
        assert_eq!(
            two.heading.size_px,
            one.heading.size_px * 2,
            "and so did the text"
        );
    }

    /// The dialog is a scene: opening pushes one, either button pops it.
    #[test]
    fn the_dialog_opens_as_a_scene_and_closes_from_its_buttons() {
        let mut app = app();
        assert_eq!(app.ui.scene_count(), 1);
        app.on_message(Message::ShowDialog);
        assert_eq!(app.ui.scene_count(), 2, "the dialog is a pushed scene");
        app.on_message(Message::CloseDialog);
        assert_eq!(app.ui.scene_count(), 1, "either button pops it");
    }

    /// The drawer opens through the tree's own overlay, so Escape and the dim
    /// already work; the app only has to ask.
    #[test]
    fn the_drawer_opens_and_the_tree_owns_closing_it() {
        let mut app = app();
        app.on_message(Message::ShowDrawer);
        assert!(app.ui.drawer_open());
        assert!(app.ui.close_drawer(), "the exit slide starts");
    }

    /// One slider drives the bar and the ring: the message loop, not a
    /// callback web.
    #[test]
    fn the_level_slider_drives_the_bar_and_the_ring() {
        let mut app = app();
        app.on_message(Message::Level(0.8));
        let ring = app
            .ui
            .widget::<RadialProgress>(app.nodes.level_ring)
            .expect("ring");
        assert_eq!(ring.label(), "80 %");
    }

    /// Editing a channel leaves the built-in: the seeds become the theme, and
    /// the derived theme still clears AA — the editor cannot produce
    /// grey-on-grey by accident.
    #[test]
    fn an_edit_leaves_the_builtin_and_the_result_stays_readable() {
        let mut app = app();
        assert_eq!(app.ui.theme().name, "dark");
        app.on_message(Message::Channel(0, 200.0));
        assert_eq!(app.ui.theme().name, "custom");
        assert!(app.ui.theme().validate(AA).is_ok());
        // And picking a built-in again reads its seeds back out.
        app.on_message(Message::UseTheme(0));
        assert_eq!(app.ui.theme().name, "light");
        assert_eq!(app.seeds[0], Theme::BUILT_IN[0].color(Role::Base100));
    }

    /// Whatever the surprise button rolls, the derivation keeps it readable.
    /// This is the theme system's promise, checked through the front door.
    #[test]
    fn every_surprise_theme_clears_aa() {
        let mut app = app();
        for _ in 0..25 {
            app.on_message(Message::Surprise);
            assert!(
                app.ui.theme().validate(AA).is_ok(),
                "surprise produced an unreadable theme: {:?}",
                app.seeds
            );
        }
    }

    /// The accordion starts with one section open and folds through messages;
    /// heights land exactly when the tween does.
    #[test]
    fn the_accordion_folds_through_messages() {
        let mut app = app();
        assert_eq!(app.accordion.open(), Some(0));
        app.on_message(Message::Fold(1));
        assert_eq!(app.accordion.open(), Some(1));
        // Land every tween; time only ever moves forward. The spinner is put to
        // bed first, which leaves the wall clock — that one has no toggle and is
        // not supposed to have one.
        app.on_message(Message::Spin(false));
        app.ui.tick(10_000);
        assert_eq!(app.ui.animating(), 1, "only the clock left mid-fold");
    }

    /// Flicker is a *property*, and this is it: **the buffer just presented
    /// holds what a full repaint of the same tree would hold.**
    ///
    /// A panel flickering at the refresh rate is two buffers disagreeing, shown
    /// alternately. That is what damage tracking risks and what buffer age is
    /// for — repaint only this frame's damage into a buffer that is two frames
    /// old and last frame's pixels stay in it, alternating with the right ones
    /// for as long as nothing damages them again. It looks exactly like a tear
    /// to a person watching a panel, and unlike a tear it does not need
    /// hardware to find.
    ///
    /// Note what the property is *not*: at rest the two buffers do not agree,
    /// and must not be expected to. Only the presented one has been brought up
    /// to date; the other is a frame behind by design, and buffer age is the
    /// promise that it will be caught up before it is shown. What has to be
    /// true is that whatever went to the panel is right.
    ///
    /// So: two trees, the same events. One runs the DRM kiosk loop with the
    /// display taken out — two buffers, age two, `paint` then `presented` — and
    /// the other repaints everything from scratch every time, which is slow and
    /// cannot be wrong. They must agree at every checkpoint. The spinner is put
    /// to bed first, because a tree that never stops changing never settles.
    #[test]
    fn the_presented_buffer_is_what_a_full_repaint_would_have_drawn() {
        const SIZE: Size = Size::new(1280, 800);
        const PIXELS: usize = (SIZE.width * SIZE.height) as usize;

        fn blank() -> Vec<u32> {
            vec![0u32; PIXELS]
        }

        fn paint_into(ui: &mut Ui<Message>, buffer: &mut [u32], age: denise::BufferAge) {
            let mut frame =
                denise::Frame::new(buffer, SIZE, SIZE.width, denise::PixelFormat::Xrgb8888, age)
                    .expect("frame");
            ui.paint(&mut frame);
            drop(frame);
            ui.presented();
        }

        /// The kiosk loop: alternate buffers, report which one went out.
        struct Swapped {
            buffers: [Vec<u32>; 2],
            frame: u64,
        }

        impl Swapped {
            /// Presents until the tree stops asking, the way the loop's
            /// `needs_paint` guard does, and answers with the buffer the panel
            /// would be showing. Capped, so a tree that never settles fails
            /// here rather than hanging.
            fn settle(&mut self, ui: &mut Ui<Message>, now_ms: u64) -> &[u32] {
                let mut shown = 0;
                for _ in 0..64 {
                    ui.tick(now_ms);
                    if !ui.needs_paint() {
                        return &self.buffers[shown];
                    }
                    // The first pass over the buffers has nothing to be old
                    // relative to, which is what `Undefined` means and what the
                    // swapchain reports.
                    let age = if self.frame < 2 {
                        denise::BufferAge::Undefined
                    } else {
                        denise::BufferAge::Frames(2)
                    };
                    shown = (self.frame % 2) as usize;
                    paint_into(ui, &mut self.buffers[shown], age);
                    self.frame += 1;
                }
                panic!("the tree never came to rest");
            }
        }

        let mut app = App::new(SIZE, 1.0, None, Motion::default());
        let mut reference = App::new(SIZE, 1.0, None, Motion::default());
        // The one legitimate insomniac; see the test below.
        for tree in [&mut app, &mut reference] {
            tree.on_message(Message::Spin(false));
        }

        let mut swap = Swapped {
            buffers: [blank(), blank()],
            frame: 0,
        };
        let mut truth = blank();

        // Both trees see the same events at the same clock, so any difference
        // in pixels is the incremental path's alone.
        let check = |app: &mut App,
                     reference: &mut App,
                     swap: &mut Swapped,
                     truth: &mut Vec<u32>,
                     events: &[InputEvent],
                     now: u64,
                     what: &str| {
            app.ui.handle(events);
            reference.ui.handle(events);

            let shown = swap.settle(&mut app.ui, now);

            reference.ui.tick(now);
            reference.ui.invalidate_all();
            paint_into(&mut reference.ui, truth, denise::BufferAge::Undefined);

            if let Some(at) = shown.iter().zip(truth.iter()).position(|(a, b)| a != b) {
                let (x, y) = (at % SIZE.width as usize, at / SIZE.width as usize);
                panic!(
                    "{what}: the panel is showing stale pixels from ({x}, {y}) \
                     — incremental {:#010x}, full repaint {:#010x}",
                    shown[at], truth[at]
                );
            }
        };

        let at = |ui: &Ui<Message>, id: NodeId| {
            let bounds = ui.layout(id).expect("laid out");
            Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
        };

        check(
            &mut app,
            &mut reference,
            &mut swap,
            &mut truth,
            &[],
            0,
            "the first frame",
        );

        // Over a button, over a list, over a rating — the gestures the flicker
        // was reported on, each settled before the next.
        let stops = [
            at(&app.ui, app.nodes.theme_list),
            at(&app.ui, app.nodes.mode_select),
            at(&app.ui, app.nodes.stars),
            at(&app.ui, app.nodes.seed_select),
        ];
        for (step, position) in stops.into_iter().enumerate() {
            check(
                &mut app,
                &mut reference,
                &mut swap,
                &mut truth,
                &[InputEvent::PointerMoved { position }],
                1_000 + step as u64 * 1_000,
                &format!("hovering stop {step} at {position:?}"),
            );
        }

        // And a popup, which is the case with the most to go wrong: a scene
        // pushed over the tree, painted above everything, then dismissed —
        // and dismissing it is what leaves a footprint behind.
        let target = stops[3];
        let click = |state| InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position: target,
            modifiers: Modifiers::default(),
        };
        check(
            &mut app,
            &mut reference,
            &mut swap,
            &mut truth,
            &[click(ElementState::Down), click(ElementState::Up)],
            5_000,
            "with a popup open",
        );
        check(
            &mut app,
            &mut reference,
            &mut swap,
            &mut truth,
            &[InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                repeat: false,
                modifiers: Modifiers::default(),
            }],
            6_000,
            "after the popup closed",
        );
    }

    /// The same property, but checked on **every frame of a moving tree**
    /// rather than only once it stops.
    ///
    /// The test above settles before it looks, so it only ever judges the last
    /// frame of an animation — and the last frame of an animation is the one
    /// most likely to be right, because whatever damage was missed on the way
    /// has usually been marked by something else by then. Flicker is reported
    /// on *moving* things: a hover highlight crossing in, a toast arriving, a
    /// popup opening. Those are the frames nobody was checking.
    ///
    /// So this steps the clock on the tree's own motion grid — sixteen
    /// milliseconds, the rate both trees are set to, so neither can be caught
    /// mid-sample by the other — and compares what the panel would be showing
    /// against a full repaint after every single tick, through a hover, a
    /// toast, and a popup.
    ///
    /// A frame the incremental tree declines to paint is compared too. If the
    /// tree moved and did not ask for a repaint, the panel is showing a stale
    /// frame, and that is the same defect seen from the other side.
    #[test]
    fn every_frame_of_a_moving_tree_is_what_a_full_repaint_would_have_drawn() {
        const SIZE: Size = Size::new(1280, 800);
        const PIXELS: usize = (SIZE.width * SIZE.height) as usize;
        /// The motion rate both trees run at, so their samples line up.
        const STEP_MS: u64 = 16;

        fn paint_into(ui: &mut Ui<Message>, buffer: &mut [u32], age: denise::BufferAge) {
            let mut frame =
                denise::Frame::new(buffer, SIZE, SIZE.width, denise::PixelFormat::Xrgb8888, age)
                    .expect("frame");
            ui.paint(&mut frame);
            drop(frame);
            ui.presented();
        }

        let mut app = App::new(SIZE, 1.0, None, Motion::default());
        let mut reference = App::new(SIZE, 1.0, None, Motion::default());

        let mut buffers = [vec![0u32; PIXELS], vec![0u32; PIXELS]];
        let mut truth = vec![0u32; PIXELS];
        let mut frame: u64 = 0;
        let mut shown = 0usize;

        let at = |ui: &Ui<Message>, id: NodeId| {
            let bounds = ui.layout(id).expect("laid out");
            Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
        };
        let press = |position, state| InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position,
            modifiers: Modifiers::default(),
        };

        // What the panel would be showing after one turn of the kiosk loop,
        // against what the tree actually looks like at that instant.
        let mut step =
            |app: &mut App, reference: &mut App, events: &[InputEvent], now: u64, what: &str| {
                app.ui.handle(events);
                reference.ui.handle(events);
                app.ui.tick(now);
                reference.ui.tick(now);

                if app.ui.needs_paint() {
                    let age = if frame < 2 {
                        denise::BufferAge::Undefined
                    } else {
                        denise::BufferAge::Frames(2)
                    };
                    shown = (frame % 2) as usize;
                    paint_into(&mut app.ui, &mut buffers[shown], age);
                    frame += 1;
                }

                reference.ui.invalidate_all();
                paint_into(&mut reference.ui, &mut truth, denise::BufferAge::Undefined);

                // The shape of a disagreement says what it is: a rectangle is a
                // region that was never repainted, a scatter of edge pixels is
                // something drawn slightly differently.
                let mut wrong = 0usize;
                let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                let mut first = None;
                for (offset, (a, b)) in buffers[shown].iter().zip(truth.iter()).enumerate() {
                    if a == b {
                        continue;
                    }
                    let (x, y) = (
                        (offset % SIZE.width as usize) as i32,
                        (offset / SIZE.width as usize) as i32,
                    );
                    wrong += 1;
                    first.get_or_insert((x, y, *a, *b));
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                if let Some((x, y, got, want)) = first {
                    let repainted = format!("{:?}", app.ui.damage());
                    // Which of the two things this can be: the trees have drifted
                    // apart in state, or painting a region differs from painting
                    // the surface. Ask the incremental tree to draw everything and
                    // see which buffer it agrees with.
                    let mut its_own_full = vec![0u32; PIXELS];
                    app.ui.invalidate_all();
                    paint_into(&mut app.ui, &mut its_own_full, denise::BufferAge::Undefined);
                    let same_tree = its_own_full == truth;
                    // The pattern says which failure this is: a ring is an edge
                    // blended against the wrong backdrop, a solid block is
                    // something not drawn at all, a shifted disc is a position.
                    let mut map = String::new();
                    for row in (y0 - 1).max(0)..=(y1 + 1).min(SIZE.height as i32 - 1) {
                        for col in (x0 - 1).max(0)..=(x1 + 1).min(SIZE.width as i32 - 1) {
                            let at = row as usize * SIZE.width as usize + col as usize;
                            map.push(match (buffers[shown][at], truth[at]) {
                                (a, b) if a == b => '.',
                                (a, b) if (a & 0xff).abs_diff(b & 0xff) > 64 => '#',
                                _ => '+',
                            });
                        }
                        map.push('\n');
                    }
                    panic!(
                        "{what} at {now} ms: {wrong} stale pixels in \
                     ({x0}, {y0})..=({x1}, {y1}), {} by {} — first at ({x}, {y}), \
                     panel {got:#010x}, tree {want:#010x}\n\
                     repainted: {repainted}\n\
                     the same tree painted whole matches the reference: {same_tree}\n{map}",
                        x1 - x0 + 1,
                        y1 - y0 + 1,
                    );
                }
            };

        let mut now = 0;
        // Settle the opening frame first; the spinner keeps both trees moving
        // from here on, which is the point.
        for _ in 0..4 {
            step(&mut app, &mut reference, &[], now, "opening");
            now += STEP_MS;
        }

        // A hover crossing in, held while its transition runs.
        let list = at(&app.ui, app.nodes.theme_list);
        step(
            &mut app,
            &mut reference,
            &[InputEvent::PointerMoved { position: list }],
            now,
            "hover arriving",
        );
        now += STEP_MS;
        for _ in 0..12 {
            step(&mut app, &mut reference, &[], now, "hover settling");
            now += STEP_MS;
        }

        // A toast, which arrives at the bottom of the screen and moves.
        app.on_message(Message::ShowToast);
        reference.on_message(Message::ShowToast);
        for _ in 0..24 {
            step(&mut app, &mut reference, &[], now, "toast arriving");
            now += STEP_MS;
        }

        // And a popup over the lot.
        let select = at(&app.ui, app.nodes.seed_select);
        let click = |state| InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position: select,
            modifiers: Modifiers::default(),
        };
        step(
            &mut app,
            &mut reference,
            &[
                InputEvent::PointerMoved { position: select },
                click(ElementState::Down),
                click(ElementState::Up),
            ],
            now,
            "popup opening",
        );
        now += STEP_MS;
        for _ in 0..12 {
            step(&mut app, &mut reference, &[], now, "popup settling");
            now += STEP_MS;
        }

        // Escape the popup, then sweep the pointer over the whole surface.
        // Hover is where the flicker was reported and the gallery has far more
        // widgets than there are named fields on `Nodes`, so rather than pick
        // four and hope, this walks a grid over every one of them and lets each
        // frame answer for itself.
        step(
            &mut app,
            &mut reference,
            &[InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                repeat: false,
                modifiers: Modifiers::default(),
            }],
            now,
            "popup dismissed",
        );
        now += STEP_MS;

        let mut y = 8;
        while y < SIZE.height as i32 {
            let mut x = 8;
            while x < SIZE.width as i32 {
                let position = Point::new(x, y);
                step(
                    &mut app,
                    &mut reference,
                    &[InputEvent::PointerMoved { position }],
                    now,
                    &format!("sweeping onto {position:?}"),
                );
                now += STEP_MS;
                // A second frame on the same spot: a state change that lands
                // one frame late shows up here and nowhere else.
                step(
                    &mut app,
                    &mut reference,
                    &[],
                    now,
                    &format!("resting on {position:?}"),
                );
                now += STEP_MS;

                // Hovering is half the report; the other half is interacting.
                // Press, release and scroll wherever the pointer has got to,
                // whatever is under it — a press that opens a dialog or sends a
                // toast is coverage, not a problem, because both trees get the
                // same events and have to agree about the result.
                for events in [
                    &[press(position, ElementState::Down)][..],
                    &[press(position, ElementState::Up)][..],
                    &[InputEvent::PointerScroll {
                        delta_x: 0.0,
                        delta_y: 40.0,
                        position,
                    }][..],
                    &[][..],
                ] {
                    step(
                        &mut app,
                        &mut reference,
                        events,
                        now,
                        &format!("interacting at {position:?}"),
                    );
                    now += STEP_MS;
                }
                x += 64;
            }
            y += 64;
        }
    }

    /// A tree at rest holds two things awake, and they cost very different
    /// amounts.
    ///
    /// The spinner is the frame-rate one, and its toggle puts it to bed. The
    /// wall clock cannot be put to bed — a clock that stops is not a clock — but
    /// it names its own deadline instead of asking for the animation rate, so
    /// what it actually costs is one wake a second rather than sixty. That
    /// difference is the whole reason `Wake::At` exists, and asserting the count
    /// without asserting the interval would miss it entirely.
    #[test]
    fn only_the_spinner_and_the_clock_keep_the_tree_awake() {
        let mut app = app();
        app.ui.tick(10_000);
        assert_eq!(app.ui.animating(), 2, "the spinner and the clock");

        app.on_message(Message::Spin(false));
        app.ui.tick(10_001);
        assert_eq!(app.ui.animating(), 1, "the spinner sleeps on request");

        // The clock alone now sets the deadline, and it is the turn of the
        // second — never the frame interval.
        let wake = app.ui.next_wake_ms().expect("the clock is still due");
        assert!(
            (10_002..=11_001).contains(&wake),
            "the clock woke for a frame, not a second: {wake}"
        );
    }
}
