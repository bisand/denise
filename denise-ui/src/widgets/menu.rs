//! A menu bar, and the popup a menu — or a right-click — opens as.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, Point, PointerButton, Rect, Role, Size, Theme};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};

/// Horizontal padding either side of a title, and either side of a menu row.
const PAD: i32 = 10;
/// The gap between a row's label and its accelerator, at minimum.
const KEYS_GAP: i32 = 28;

/// One row of a menu.
///
/// A row is a *record*, not a widget: menus are rebuilt every time they open,
/// because what they offer depends on what is true at that moment — whether
/// there is a selection to copy, whether the list of recent files is empty.
/// Building a tree for something that lives for one click and is then thrown
/// away would cost more than it saves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuItem {
    /// What the row says.
    pub label: String,
    /// The accelerator, shown right-aligned. Empty for none.
    ///
    /// Written the way the platform writes it — [`shortcut`] does that.
    pub shortcut: String,
    /// Whether the row is a setting that is currently on.
    pub checked: bool,
    /// A disabled row is readable and unselectable: a heading, or a command
    /// that has nothing to act on.
    pub enabled: bool,
}

impl MenuItem {
    /// A row that does something.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            checked: false,
            enabled: true,
        }
    }

    /// A row that names the group under it.
    ///
    /// There is no separator: a rule between rows says "these are different"
    /// and a heading says *how* they are different, for the same pixels.
    pub fn heading(label: impl Into<String>) -> Self {
        Self {
            enabled: false,
            ..Self::new(label)
        }
    }

    /// Sets the accelerator, given in the portable spelling `"Cmd+Shift+T"`.
    ///
    /// The spelling shown is the platform's; see [`shortcut`].
    #[must_use]
    pub fn with_shortcut(mut self, keys: &str) -> Self {
        self.shortcut = shortcut(keys);
        self
    }

    /// Marks the row as a setting that is on.
    #[must_use]
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    /// Offers the row only when there is something for it to act on.
    #[must_use]
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// A row that cannot be chosen.
    #[must_use]
    pub fn disabled(self) -> Self {
        self.enabled(false)
    }
}

/// Rewrites an accelerator the way the platform writes it: `⌘⇧T` on a Mac,
/// `Ctrl+Shift+T` everywhere else.
///
/// Input is the portable spelling — modifiers named in words, joined with `+`,
/// and `Cmd` meaning "the platform's command key". Menus are the one place a
/// user reads a shortcut rather than pressing it, and reading `Ctrl+O` on a Mac
/// is reading the wrong key.
pub fn shortcut(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        keys.split('+')
            .map(|part| match part {
                "Cmd" | "Ctrl" | "Super" => "\u{2318}",
                "Shift" => "\u{21e7}",
                "Alt" | "Option" => "\u{2325}",
                other => other,
            })
            .collect()
    } else {
        keys.replace("Cmd", "Ctrl").replace("Option", "Alt")
    }
}

/// A row of menu titles along the top of a window.
///
/// The titles only; a menu's *rows* are [`open_menu`], because they are built
/// fresh each time one opens. The bar reports which title was pressed and
/// nothing else — it does not open anything itself, since what a menu contains
/// is the application's business and cannot be known here.
///
/// ```no_run
/// # use denise::{Rect, Size, theme};
/// # use denise_ui::Ui;
/// # use denise_ui::widgets::{MenuBar, MenuItem, open_menu};
/// # #[derive(Clone, Copy, PartialEq, Eq)]
/// # enum Msg { Menu(usize), Pick(usize) }
/// let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
/// let root = ui.root();
/// let bar = ui
///     .add(root, MenuBar::new(["File", "Edit"], Msg::Menu), Rect::new(0, 0, 800, 28))
///     .expect("bar");
///
/// // When `Msg::Menu(i)` arrives, open that title's rows beneath it.
/// let rows = [MenuItem::new("Open…").with_shortcut("Cmd+O")];
/// open_menu(&mut ui, bar, 0, &rows, Msg::Pick);
/// ```
///
/// # Why the bar is one widget and the titles are not
///
/// A title is a word with a highlight behind it. Made of buttons, a bar would
/// need every button told which of its siblings is open so the others can drop
/// their highlight, and the arrow keys — which walk a menu bar sideways —
/// would have nowhere to live. One widget owns the row, so both are ordinary.
pub struct MenuBar<M> {
    titles: Vec<String>,
    message: Option<fn(usize) -> M>,
    /// Which menu is currently down, so its title stays lit.
    open: Option<usize>,
    hovered: Option<usize>,
    role: Role,
    style: TextStyle,
}

impl<M: 'static> MenuBar<M> {
    /// A bar of `titles`, reporting the index of the one pressed.
    pub fn new(
        titles: impl IntoIterator<Item = impl Into<String>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self {
            titles: titles.into_iter().map(Into::into).collect(),
            message: Some(message),
            open: None,
            hovered: None,
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// A bar that reports nothing: titles a form shows and does not wire.
    pub fn inert(titles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            message: None,
            ..Self::new(titles, |_| unreachable!("inert bars have no message"))
        }
    }

    /// Colour of the open title's highlight.
    #[must_use]
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Font and size for the titles.
    #[must_use]
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Title size in pixels, keeping the face.
    #[must_use]
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// The titles, in the order they are drawn.
    pub fn titles(&self) -> &[String] {
        &self.titles
    }

    /// Replaces the titles. An open menu whose title is gone closes.
    pub fn set_titles(&mut self, titles: impl IntoIterator<Item = impl Into<String>>) {
        self.titles = titles.into_iter().map(Into::into).collect();
        if self.open.is_some_and(|i| i >= self.titles.len()) {
            self.open = None;
        }
    }

    /// The style titles are measured and drawn with, for a caller placing a
    /// menu under one of them.
    pub fn style(&self) -> TextStyle {
        self.style
    }

    /// Which menu the application has open, or `None`.
    ///
    /// The bar cannot know: it reports a press and the application decides
    /// whether a menu opens, so it has to be told in order to keep that title
    /// lit while the popup is up.
    pub fn set_open(&mut self, index: Option<usize>) {
        self.open = index;
    }

    /// Which menu the bar is showing as open.
    pub fn open(&self) -> Option<usize> {
        self.open
    }

    /// The height a bar wants for its text.
    pub fn preferred_height(&self, theme: &Theme, text: &mut TextEngine) -> i32 {
        text.metrics(self.style).line_height() + theme.metrics.border * 2 + PAD
    }
}

/// Where each title sits, left to right.
///
/// A free function because a caller placing a menu under a title has the tree
/// borrowed to read the widget and needs the text engine to measure — and
/// because painting and hit testing must never disagree about where a title is.
pub fn title_layout(
    titles: &[String],
    bounds: Rect,
    style: TextStyle,
    text: &mut TextEngine,
) -> Vec<Rect> {
    let mut out = Vec::with_capacity(titles.len());
    let mut x = bounds.x;
    for title in titles {
        let width = text.measure_line(style, title) + PAD * 2;
        out.push(Rect::new(x, bounds.y, width, bounds.height));
        x += width;
    }
    out
}

impl<M: 'static> Widget<M> for MenuBar<M> {
    fn accepts_pointer(&self) -> bool {
        self.message.is_some()
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let theme = ctx.theme;
        let bounds = ctx.bounds;
        canvas.fill_rect(bounds, theme.color(Role::Base200));
        let rects = title_layout(&self.titles, bounds, self.style, ctx.text);
        let metrics = ctx.text.metrics(self.style);
        let baseline = bounds.y + (bounds.height - metrics.line_height()) / 2 + metrics.ascent;
        let radius = theme.radius(denise::Radius::Selector);

        for (i, rect) in rects.iter().enumerate() {
            let open = self.open == Some(i);
            let hovered = self.hovered == Some(i) && !open;
            let fg = if open {
                canvas.fill_rounded_rect(*rect, radius, theme.color(self.role));
                theme.content_of(self.role)
            } else {
                if hovered {
                    canvas.fill_rounded_rect(*rect, radius, theme.color(Role::Base300));
                }
                theme.color(Role::BaseContent)
            };
            ctx.text.draw_line(
                canvas,
                self.style,
                Point::new(rect.x + PAD, baseline),
                &self.titles[i],
                fg,
            );
        }
    }

    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        let width: i32 = self
            .titles
            .iter()
            .map(|t| ctx.text.measure_line(self.style, t) + PAD * 2)
            .sum();
        Measured {
            width: Some(width),
            height: Some(self.preferred_height(ctx.theme, ctx.text)),
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some(message) = self.message else {
            return Handled::No;
        };
        let Event::Input(input) = event else {
            return Handled::No;
        };
        match input {
            InputEvent::PointerMoved { position } => {
                let hovered = self.hit(ctx.bounds, ctx.text, *position);
                if hovered != self.hovered {
                    self.hovered = hovered;
                    return Handled::Yes;
                }
                Handled::No
            }
            InputEvent::PointerLeft => {
                if self.hovered.take().is_some() {
                    return Handled::Yes;
                }
                Handled::No
            }
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position,
                ..
            } => match self.hit(ctx.bounds, ctx.text, *position) {
                Some(index) => {
                    ctx.emit(message(index));
                    Handled::Yes
                }
                None => Handled::No,
            },
            _ => Handled::No,
        }
    }

    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
}

impl<M: 'static> MenuBar<M> {
    fn hit(&self, bounds: Rect, text: &mut TextEngine, p: Point) -> Option<usize> {
        title_layout(&self.titles, bounds, self.style, text)
            .into_iter()
            .position(|r| r.contains(p))
    }
}

impl<M> Describe for MenuBar<M> {
    const KIND: &'static str = "menubar";
    const DOC: &'static str = "A row of menu titles along the top of a window.";
    const GROUP: Group = Group::Container;
    const ICON: &'static denise_render::icon::Icon = &super::icons::MENU_BAR;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "title",
            PropertyKind::List,
            "The menu names, as `title` child nodes.",
        ),
        Property::new(
            "on-open",
            PropertyKind::Message(Payload::Index),
            "Emitted with the index of the title that was pressed. The application decides what opens.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour of the open title's highlight.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "role" => self.role = value.as_role()?,
            "size" => self.style.size_px = value.as_size()?,
            "title" | "on-open" => return Err(Mismatch::Supplied),
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

/// Opens `items` as a menu below title `index` of a [`MenuBar`].
///
/// The popup is an ordinary one — [`Ui::push_popup_at`] — so it flips near a
/// screen edge, closes on Escape or a press outside (swallowing that press),
/// and returns focus to the bar. `message` is emitted with the row's index when
/// a row is **chosen**, by Enter or by a tap; the arrow keys move the highlight
/// silently.
///
/// The application closes the popup and acts on the choice: closing here would
/// be deciding, for every menu, that one pick ends it.
///
/// Returns the popup's container, or `None` when `bar` is not a live
/// [`MenuBar`] or `index` is past its last title.
pub fn open_menu<M: Clone + 'static>(
    ui: &mut crate::Ui<M>,
    bar: crate::NodeId,
    index: usize,
    items: &[MenuItem],
    message: fn(usize) -> M,
) -> Option<crate::NodeId> {
    let widget = ui.widget::<MenuBar<M>>(bar)?;
    let titles = widget.titles().to_vec();
    let style = widget.style();
    let bounds = ui.bounds(bar)?;
    let at = *title_layout(&titles, bounds, style, ui.text_mut()).get(index)?;
    open_menu_at(ui, bar, at, items, style, message)
}

/// Opens `items` beside `at` — the shape a context menu wants.
///
/// `anchor` is only where focus returns when the menu closes: the widget that
/// was right-clicked, usually. `at` is where the menu goes, and a one-pixel
/// rectangle at the pointer is the usual answer.
///
/// ```no_run
/// # use denise::Rect;
/// # use denise_ui::{NodeId, Ui};
/// # use denise_ui::widgets::{MenuItem, open_menu_at};
/// # use denise_text::TextStyle;
/// # #[derive(Clone, Copy, PartialEq, Eq)]
/// # enum Msg { Pick(usize) }
/// # fn demo(ui: &mut Ui<Msg>, list: NodeId, at: denise::Point) {
/// let rows = [MenuItem::new("Rename…"), MenuItem::new("Delete")];
/// open_menu_at(
///     ui,
///     list,
///     Rect::new(at.x, at.y, 1, 1),
///     &rows,
///     TextStyle::built_in(14),
///     Msg::Pick,
/// );
/// # }
/// ```
pub fn open_menu_at<M: Clone + 'static>(
    ui: &mut crate::Ui<M>,
    anchor: crate::NodeId,
    at: Rect,
    items: &[MenuItem],
    style: TextStyle,
    message: fn(usize) -> M,
) -> Option<crate::NodeId> {
    if items.is_empty() {
        return None;
    }
    let row = ui.theme().metrics.size_field;
    let check = ui.text_mut().measure_line(style, CHECK);
    let width = items
        .iter()
        .map(|item| {
            let label = ui.text_mut().measure_line(style, &item.label);
            let keys = ui.text_mut().measure_line(style, &item.shortcut);
            check + PAD + label + if keys > 0 { KEYS_GAP + keys } else { 0 }
        })
        .max()
        .unwrap_or(0)
        + PAD * 2;
    let height = row * items.len() as i32;

    let container = ui.push_popup_at(
        anchor,
        at,
        Size::new(width as u32, height as u32),
        crate::Side::Below,
    )?;
    ui.add(
        container,
        super::Panel::default(),
        Rect::new(0, 0, width, height),
    )?;
    let rows = ui.add(
        container,
        Rows {
            items: items.to_vec(),
            selected: None,
            hovered: None,
            message,
            style,
        },
        Rect::new(0, 0, width, height),
    )?;
    // So the keyboard works the moment it opens, and Escape has somewhere to
    // return focus from.
    ui.focus(Some(rows));
    Some(container)
}

/// The tick before a checked row.
const CHECK: &str = "\u{2713}";

/// The rows inside an open menu.
///
/// Not a public widget: a menu's rows exist only while the menu is up, they are
/// never placed in a form, and their shape — a tick column, a label, an
/// accelerator — is the menu's rather than a list's.
struct Rows<M> {
    items: Vec<MenuItem>,
    selected: Option<usize>,
    hovered: Option<usize>,
    message: fn(usize) -> M,
    style: TextStyle,
}

impl<M: 'static> Rows<M> {
    fn row_height(&self, theme: &Theme) -> i32 {
        theme.metrics.size_field.max(1)
    }

    fn row_at(&self, bounds: Rect, height: i32, p: Point) -> Option<usize> {
        if !bounds.contains(p) {
            return None;
        }
        let index = ((p.y - bounds.y) / height) as usize;
        (index < self.items.len()).then_some(index)
    }

    /// The next row that can be chosen, walking `step` at a time. Headings and
    /// unavailable commands are stepped over rather than landed on.
    fn step(&self, from: Option<usize>, step: isize) -> Option<usize> {
        let n = self.items.len() as isize;
        if n == 0 {
            return None;
        }
        let mut at = from.map_or(if step > 0 { -1 } else { n }, |i| i as isize);
        for _ in 0..n {
            at = (at + step).rem_euclid(n);
            if self.items[at as usize].enabled {
                return Some(at as usize);
            }
        }
        None
    }
}

impl<M: 'static> Widget<M> for Rows<M> {
    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let theme = ctx.theme;
        let bounds = ctx.bounds;
        let height = self.row_height(theme);
        let metrics = ctx.text.metrics(self.style);
        let radius = theme.radius(denise::Radius::Selector);
        let check = ctx.text.measure_line(self.style, CHECK);

        for (i, item) in self.items.iter().enumerate() {
            let rect = Rect::new(bounds.x, bounds.y + i as i32 * height, bounds.width, height);
            let highlighted = item.enabled && (self.selected == Some(i) || self.hovered == Some(i));
            let dim = theme
                .color(Role::BaseContent)
                .mix(theme.color(Role::Base100), 128);
            let fg = if highlighted {
                let inner = Rect::new(rect.x + 2, rect.y + 1, rect.width - 4, rect.height - 2);
                canvas.fill_rounded_rect(inner, radius, theme.color(Role::Primary));
                theme.content_of(Role::Primary)
            } else if item.enabled {
                theme.color(Role::BaseContent)
            } else {
                dim
            };
            let baseline = rect.y + (height - metrics.line_height()) / 2 + metrics.ascent;
            if item.checked {
                ctx.text.draw_line(
                    canvas,
                    self.style,
                    Point::new(rect.x + PAD, baseline),
                    CHECK,
                    fg,
                );
            }
            ctx.text.draw_line(
                canvas,
                self.style,
                Point::new(rect.x + PAD + check + PAD, baseline),
                &item.label,
                fg,
            );
            if !item.shortcut.is_empty() {
                let keys = ctx.text.measure_line(self.style, &item.shortcut);
                ctx.text.draw_line(
                    canvas,
                    self.style,
                    Point::new(rect.right() - PAD - keys, baseline),
                    &item.shortcut,
                    if highlighted { fg } else { dim },
                );
            }
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Event::Input(input) = event else {
            return Handled::No;
        };
        let height = self.row_height(ctx.theme);
        match input {
            InputEvent::PointerMoved { position } => {
                let hovered = self.row_at(ctx.bounds, height, *position);
                if hovered != self.hovered {
                    self.hovered = hovered;
                    return Handled::Yes;
                }
                Handled::No
            }
            InputEvent::PointerLeft => {
                if self.hovered.take().is_some() {
                    return Handled::Yes;
                }
                Handled::No
            }
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                position,
                ..
            } => {
                // On release, so the press that opened a menu cannot also pick
                // the row that happens to be under the pointer.
                match self.row_at(ctx.bounds, height, *position) {
                    Some(i) if self.items[i].enabled => {
                        ctx.emit((self.message)(i));
                        Handled::Yes
                    }
                    _ => Handled::No,
                }
            }
            InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            } => match code {
                denise::KeyCode::ArrowDown => {
                    self.selected = self.step(self.selected, 1);
                    Handled::Yes
                }
                denise::KeyCode::ArrowUp => {
                    self.selected = self.step(self.selected, -1);
                    Handled::Yes
                }
                denise::KeyCode::Enter | denise::KeyCode::NumpadEnter => {
                    if let Some(i) = self.selected {
                        ctx.emit((self.message)(i));
                    }
                    Handled::Yes
                }
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}
