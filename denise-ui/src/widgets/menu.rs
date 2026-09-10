//! A menu bar, and the popup a menu — or a right-click — opens as.

use alloc::string::String;
use alloc::vec::Vec;

use denise::Pen;
use denise::{
    Color, ElementState, InputEvent, KeyCode, Point, PointerButton, Radius, Rect, Role, Size, Theme,
};
use denise_text::{TextEngine, TextStyle};

use crate::motion::Wake;
use crate::overlay::{Side, anchored};
use crate::widget::{
    Animation, Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};

/// Horizontal padding either side of a title.
const PAD: i32 = 10;

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
    /// A rule between two groups of rows rather than a row: no label, never
    /// highlighted, never chosen.
    pub separator: bool,
    /// The rows of the submenu this row opens, when it opens one. Empty for
    /// a row that is a command.
    pub items: Vec<MenuItem>,
}

impl MenuItem {
    /// A row that does something.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            checked: false,
            enabled: true,
            separator: false,
            items: Vec::new(),
        }
    }

    /// A row that names the group under it, for the rare group a rule cannot
    /// explain on its own.
    pub fn heading(label: impl Into<String>) -> Self {
        Self {
            enabled: false,
            ..Self::new(label)
        }
    }

    /// A rule between two groups of rows: the commands that open and close
    /// things above it, the ones that leave below.
    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            ..Self::new("")
        }
    }

    /// A row that opens `items` beside it as a submenu, on hover or on the
    /// right arrow.
    ///
    /// In the order a menu's rows are numbered — the order
    /// [`MenuEvent::Picked`] reports — a submenu's own row comes first and its
    /// rows follow it, before whatever row comes after it in this menu.
    pub fn submenu(label: impl Into<String>, items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
            ..Self::new(label)
        }
    }

    /// Whether the row can be highlighted and chosen, or opened.
    fn selectable(&self) -> bool {
        self.enabled && !self.separator
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
/// # use denise_ui::widgets::MenuEvent;
/// # #[derive(Clone, Copy, PartialEq, Eq)]
/// # enum Msg { Menu(usize), Picked(MenuEvent) }
/// let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
/// let root = ui.root();
/// let bar = ui
///     .add(root, MenuBar::new(["File", "Edit"], Msg::Menu), Rect::new(0, 0, 800, 28))
///     .expect("bar");
///
/// // When `Msg::Menu(i)` arrives, open that title's rows beneath it.
/// let rows = [MenuItem::new("Open…").with_shortcut("Cmd+O")];
/// open_menu(&mut ui, bar, 0, &rows, Msg::Picked);
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

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
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
    const ICON: &'static denise::icon::Icon = &super::icons::MENU_BAR;

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

/// What an open menu reports, through the message [`open_menu`] was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    /// A row was chosen, by Enter or by a release over it.
    ///
    /// The index counts every row of the items the menu opened with, depth
    /// first: a submenu's own row, then each of its rows, then the row after
    /// it. Separators and headings count, so a row's number is its position
    /// in the list the application wrote.
    Picked(usize),
    /// The pointer went to another title of the bar this menu hangs from, or
    /// the arrow keys walked there: close this menu and open that one. Only
    /// a menu opened with [`open_menu`] reports it.
    Title(usize),
    /// A press outside every panel, or on the title that opened the menu:
    /// nothing was chosen and the menu should close.
    Dismissed,
}

/// Opens `items` as a menu below title `index` of a [`MenuBar`].
///
/// One popup scene holds the whole cascade — the rows, and every submenu
/// hovered open beside them — so the menu behaves the way a desktop's do:
/// the pointer opens a submenu by resting on its row and closes it by
/// moving to another, a press anywhere outside dismisses it, and sliding
/// along the bar to another title reports [`MenuEvent::Title`] so the
/// application can open that one instead. Escape closes it, as it does any
/// popup, before anything else sees the key.
///
/// `message` carries a [`MenuEvent`]. The application closes the popup with
/// [`Ui::close_popup`](crate::Ui::close_popup) and acts on the choice: closing here would be
/// deciding, for every menu, that one pick ends it. Since Escape closes the
/// popup without a message, an application that lights the open title
/// should also ask [`Ui::popup_open`](crate::Ui::popup_open) each frame.
///
/// Returns the [`Menu`] node, or `None` when `bar` is not a live [`MenuBar`]
/// or `index` is past its last title.
pub fn open_menu<M: Clone + 'static>(
    ui: &mut crate::Ui<M>,
    bar: crate::NodeId,
    index: usize,
    items: &[MenuItem],
    message: fn(MenuEvent) -> M,
) -> Option<crate::NodeId> {
    let widget = ui.widget::<MenuBar<M>>(bar)?;
    let titles = widget.titles().to_vec();
    let style = widget.style();
    let bounds = ui.bounds(bar)?;
    let rects = title_layout(&titles, bounds, style, ui.text_mut());
    let at = *rects.get(index)?;
    open(
        ui,
        bar,
        at,
        Side::Below,
        items,
        style,
        message,
        rects,
        Some(index),
    )
}

/// Opens `items` beside `at` — the shape a context menu wants.
///
/// `anchor` is only where focus returns when the menu closes: the widget that
/// was right-clicked, usually. `at` is where the menu goes, and a one-pixel
/// rectangle at the pointer is the usual answer. Everything [`open_menu`]
/// says about behaviour holds, except that there is no bar to slide along.
///
/// ```no_run
/// # use denise::Rect;
/// # use denise_ui::{NodeId, Ui};
/// # use denise_ui::widgets::{MenuEvent, MenuItem, open_menu_at};
/// # use denise_text::TextStyle;
/// # #[derive(Clone, Copy, PartialEq, Eq)]
/// # enum Msg { Menu(MenuEvent) }
/// # fn demo(ui: &mut Ui<Msg>, list: NodeId, at: denise::Point) {
/// let rows = [
///     MenuItem::new("Rename…"),
///     MenuItem::separator(),
///     MenuItem::new("Delete"),
/// ];
/// open_menu_at(
///     ui,
///     list,
///     Rect::new(at.x, at.y, 1, 1),
///     &rows,
///     TextStyle::built_in(14),
///     Msg::Menu,
/// );
/// # }
/// ```
pub fn open_menu_at<M: Clone + 'static>(
    ui: &mut crate::Ui<M>,
    anchor: crate::NodeId,
    at: Rect,
    items: &[MenuItem],
    style: TextStyle,
    message: fn(MenuEvent) -> M,
) -> Option<crate::NodeId> {
    open(
        ui,
        anchor,
        at,
        Side::Below,
        items,
        style,
        message,
        Vec::new(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn open<M: Clone + 'static>(
    ui: &mut crate::Ui<M>,
    anchor: crate::NodeId,
    at: Rect,
    side: Side,
    items: &[MenuItem],
    style: TextStyle,
    message: fn(MenuEvent) -> M,
    titles: Vec<Rect>,
    open_title: Option<usize>,
) -> Option<crate::NodeId> {
    if items.is_empty() {
        return None;
    }
    let surface = ui.size();
    let theme = *ui.theme();
    let geometry = Geometry::of(style, &theme, ui.text_mut());
    let size = geometry.panel_size(items, style, ui.text_mut());
    let rect = anchored(surface, at, size, side, 0);

    // The scene covers the surface: every pointer event while the menu is up
    // is the menu's to judge, which is what lets a press outside dismiss it
    // and a submenu open beside it without either being another scene.
    let container = ui.push_popup_at(anchor, Rect::ZERO, surface, Side::Below)?;
    let menu = ui.add(
        container,
        Menu {
            items: items.to_vec(),
            style,
            message,
            titles,
            open_title,
            announced: None,
            panels: alloc::vec![Panel {
                path: Vec::new(),
                rect,
                hovered: None,
                selected: None,
            }],
            pending: None,
            geometry,
        },
        Rect::from_size(surface),
    )?;
    // So the keyboard works the moment it opens, and Escape has somewhere to
    // return focus from.
    ui.focus(Some(menu));
    Some(menu)
}

/// The tick before a checked row.
const CHECK: &str = "\u{2713}";
/// How long the pointer rests on another row before the submenu that is
/// open beside the row it left follows it. Long enough to cross the corner
/// of a row on the way into the submenu; short enough not to feel late.
const FOLLOW_MS: u64 = 140;

/// Every measurement a menu is drawn with, from the size of its type.
///
/// Nothing here is a theme token: a menu is a piece of text with a highlight
/// behind it, and every gap in it is the text's size times something. That
/// keeps the design the same at every scale, because the size the caller
/// passes already carries the scale.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// A row of a command.
    row_h: i32,
    /// A separator's row.
    sep_h: i32,
    /// Above the first row and below the last, inside the panel.
    pad_y: i32,
    /// The panel edge to a row's highlight.
    inset: i32,
    /// A row's highlight edge to its text.
    pad_x: i32,
    /// The tick column: the tick and a gap, before the label.
    tick_w: i32,
    /// Between a label and its accelerator, at least.
    keys_gap: i32,
    /// The triangle that says a row opens a submenu, and the gap before it.
    chevron_w: i32,
    radius_panel: i32,
    radius_row: i32,
    border: i32,
    ascent: i32,
    line_h: i32,
}

impl Geometry {
    fn of(style: TextStyle, theme: &Theme, text: &mut TextEngine) -> Self {
        let metrics = text.metrics(style);
        let s = i32::from(style.size_px).max(8);
        let pad_y = (s * 2 / 5).max(3);
        Self {
            row_h: metrics.line_height() + (s * 11 / 20).max(4),
            sep_h: (s * 3 / 5).max(5),
            pad_y,
            inset: pad_y,
            pad_x: (s * 4 / 5).max(6),
            tick_w: text.measure_line(style, CHECK) + (s / 2).max(3),
            keys_gap: s * 2,
            chevron_w: (s * 3 / 5).max(4) + (s / 2).max(3),
            radius_panel: theme.radius(Radius::Box).max(s / 3),
            radius_row: (s * 2 / 5).max(2),
            border: theme.metrics.border.max(1),
            ascent: metrics.ascent,
            line_h: metrics.line_height(),
        }
    }

    fn row_height(&self, item: &MenuItem) -> i32 {
        if item.separator {
            self.sep_h
        } else {
            self.row_h
        }
    }

    /// How big a panel of `items` is.
    fn panel_size(&self, items: &[MenuItem], style: TextStyle, text: &mut TextEngine) -> Size {
        let s = i32::from(style.size_px).max(8);
        let content = items
            .iter()
            .map(|item| {
                let label = text.measure_line(style, &item.label);
                let keys = if item.shortcut.is_empty() {
                    0
                } else {
                    self.keys_gap + text.measure_line(style, &item.shortcut)
                };
                let chevron = if item.items.is_empty() {
                    0
                } else {
                    self.chevron_w
                };
                label + keys.max(chevron)
            })
            .max()
            .unwrap_or(0)
            .max(s * 8);
        let width = self.inset * 2 + self.pad_x * 2 + self.tick_w + content;
        let height = self.pad_y * 2 + items.iter().map(|i| self.row_height(i)).sum::<i32>();
        Size::new(width as u32, height as u32)
    }

    /// The rectangle of row `index` inside a panel at `rect`: the highlight's
    /// rectangle, which is also what the pointer is tested against.
    fn row_rect(&self, rect: Rect, items: &[MenuItem], index: usize) -> Rect {
        let mut y = rect.y + self.pad_y;
        for item in &items[..index] {
            y += self.row_height(item);
        }
        Rect::new(
            rect.x + self.inset,
            y,
            rect.width - self.inset * 2,
            self.row_height(&items[index]),
        )
    }

    fn row_at(&self, rect: Rect, items: &[MenuItem], p: Point) -> Option<usize> {
        if !rect.contains(p) {
            return None;
        }
        let mut y = rect.y + self.pad_y;
        for (i, item) in items.iter().enumerate() {
            let h = self.row_height(item);
            if p.y >= y && p.y < y + h {
                return Some(i);
            }
            y += h;
        }
        None
    }
}

/// One open panel of a menu: the root, or a submenu hovered open.
#[derive(Clone, Debug)]
struct Panel {
    /// Which item's rows this shows: the indices down from the root, so the
    /// root is the empty path.
    path: Vec<usize>,
    rect: Rect,
    /// The row under the pointer.
    hovered: Option<usize>,
    /// The row the keyboard is on.
    selected: Option<usize>,
}

/// An open menu: its rows and every submenu open beside them.
///
/// Made by [`open_menu`] and [`open_menu_at`], never placed in a form. It is
/// public so an application can ask where its panels are — for a test, or a
/// screenshot — through [`Menu::panels`] and [`Menu::row_rect`].
///
/// # How it behaves
///
/// - Resting the pointer on a row that has a submenu opens it beside the row.
///   Moving to another row of the same panel closes it again — after a
///   moment, so that a pointer cutting the corner on its way into the submenu
///   is not punished for it.
/// - A release over a row chooses it. A release, not a press, so the press
///   that opened the menu cannot also choose whatever is under it; and so a
///   press on a title, a drag down and a release on a row is one gesture.
/// - A press outside every panel dismisses the menu, and is swallowed: a
///   click that closes a menu does not also press what was under it.
/// - Up and Down walk the deepest panel, Right opens a submenu or moves to
///   the next title, Left closes a submenu or moves to the previous title,
///   Enter chooses.
pub struct Menu<M> {
    items: Vec<MenuItem>,
    style: TextStyle,
    message: fn(MenuEvent) -> M,
    /// The bar's titles, so sliding along it switches menus; empty for a
    /// context menu.
    titles: Vec<Rect>,
    open_title: Option<usize>,
    /// The title last reported, so hovering it reports once.
    announced: Option<usize>,
    /// Root first, each submenu after the panel it opened from.
    panels: Vec<Panel>,
    /// A row the pointer moved to while a submenu was open from another: what
    /// to do about it once the pointer has rested there, and since when.
    pending: Option<Pending>,
    geometry: Geometry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    panel: usize,
    row: usize,
    since: u64,
}

/// The rows an item path leads to.
fn items_at<'a>(root: &'a [MenuItem], path: &[usize]) -> &'a [MenuItem] {
    let mut items = root;
    for &i in path {
        items = &items[i].items;
    }
    items
}

/// Rows in `items` and every submenu under them.
fn count(items: &[MenuItem]) -> usize {
    items.iter().map(|item| 1 + count(&item.items)).sum()
}

/// The depth-first number of row `row` of the panel at `path`.
fn flat_index(items: &[MenuItem], path: &[usize], row: usize) -> usize {
    match path.split_first() {
        None => count(&items[..row]),
        Some((&first, rest)) => {
            count(&items[..first]) + 1 + flat_index(&items[first].items, rest, row)
        }
    }
}

impl<M: 'static> Menu<M> {
    /// Where each open panel is, the root first.
    pub fn panels(&self) -> impl Iterator<Item = Rect> + '_ {
        self.panels.iter().map(|p| p.rect)
    }

    /// The highlight rectangle of row `row` of open panel `panel`, if both
    /// exist. Panel 0 is the root; a submenu is the panel after the one it
    /// opened from.
    pub fn row_rect(&self, panel: usize, row: usize) -> Option<Rect> {
        let p = self.panels.get(panel)?;
        let items = items_at(&self.items, &p.path);
        (row < items.len()).then(|| self.geometry.row_rect(p.rect, items, row))
    }

    fn panel_items(&self, panel: usize) -> &[MenuItem] {
        items_at(&self.items, &self.panels[panel].path)
    }

    /// The deepest panel under `p`, and the row of it.
    fn hit(&self, p: Point) -> Option<(usize, Option<usize>)> {
        (0..self.panels.len()).rev().find_map(|k| {
            let panel = &self.panels[k];
            panel
                .rect
                .contains(p)
                .then(|| (k, self.geometry.row_at(panel.rect, self.panel_items(k), p)))
        })
    }

    /// Opens the submenu of row `row` of panel `k`, closing whatever was open
    /// under `k`. Where it goes: beside the row, its first row level with it,
    /// and on the other side when that does not fit.
    fn open_submenu(&mut self, k: usize, row: usize, text: &mut TextEngine, surface: Size) {
        self.panels.truncate(k + 1);
        let parent = self.panels[k].rect;
        let items = self.panel_items(k);
        let item = &items[row];
        if item.items.is_empty() || !item.selectable() {
            return;
        }
        let g = self.geometry;
        let row_rect = g.row_rect(parent, items, row);
        let size = g.panel_size(&item.items, self.style, text);
        let (w, h) = (size.width as i32, size.height as i32);
        // Overlapping the parent by the inset, as desktops do, so the two
        // read as one thing; the first row of the child level with the row
        // that opened it.
        let mut x = parent.right() - g.inset;
        if x + w > surface.width as i32 && parent.x - w + g.inset >= 0 {
            x = parent.x - w + g.inset;
        }
        let x = x.clamp(0, (surface.width as i32 - w).max(0));
        let y = (row_rect.y - g.pad_y).clamp(0, (surface.height as i32 - h).max(0));
        let mut path = self.panels[k].path.clone();
        path.push(row);
        self.panels[k].hovered = Some(row);
        self.panels.push(Panel {
            path,
            rect: Rect::new(x, y, w, h),
            hovered: None,
            selected: None,
        });
    }

    /// The submenu open from row `row` of panel `k`, if any.
    fn submenu_of(&self, k: usize, row: usize) -> bool {
        self.panels
            .get(k + 1)
            .is_some_and(|child| child.path.last() == Some(&row))
    }

    fn title_at(&self, p: Point) -> Option<usize> {
        self.titles.iter().position(|r| r.contains(p))
    }

    /// The next selectable row of `items` from `from`, walking `step`.
    fn step(items: &[MenuItem], from: Option<usize>, step: isize) -> Option<usize> {
        let n = items.len() as isize;
        if n == 0 {
            return None;
        }
        let mut at = from.map_or(if step > 0 { -1 } else { n }, |i| i as isize);
        for _ in 0..n {
            at = (at + step).rem_euclid(n);
            if items[at as usize].selectable() {
                return Some(at as usize);
            }
        }
        None
    }

    fn pointer_moved(&mut self, p: Point, ctx: &mut EventCtx<'_, M>) -> Handled {
        // Sliding along the bar: another title opens that menu instead.
        if let Some(title) = self.title_at(p)
            && Some(title) != self.open_title
        {
            if self.announced != Some(title) {
                self.announced = Some(title);
                ctx.emit((self.message)(MenuEvent::Title(title)));
            }
            return Handled::Yes;
        }
        self.announced = None;

        let Some((k, row)) = self.hit(p) else {
            // Off every panel: the deepest panel drops its highlight unless
            // it is holding a submenu open, which the pointer may be on its
            // way to.
            let deepest = self.panels.len() - 1;
            let changed = self.panels[deepest].hovered.take().is_some();
            self.pending = None;
            return if changed { Handled::Yes } else { Handled::No };
        };
        let (row, opens) = {
            let items = self.panel_items(k);
            let row = row.filter(|&r| items[r].selectable());
            (row, row.is_some_and(|r| !items[r].items.is_empty()))
        };
        let mut changed = false;

        // Panels deeper than the one under the pointer lose their highlight;
        // the one under it follows the pointer.
        for deeper in &mut self.panels[k + 1..] {
            changed |= deeper.hovered.take().is_some();
        }
        if self.panels[k].hovered != row {
            self.panels[k].hovered = row;
            changed = true;
        }

        match row {
            Some(r) if self.submenu_of(k, r) => {
                // Back on the row whose submenu is open: nothing to change.
                self.pending = None;
            }
            Some(r) if opens => {
                // A row with a submenu opens it now, closing any other.
                let surface = Size::new(ctx.bounds.width as u32, ctx.bounds.height as u32);
                self.open_submenu(k, r, ctx.text, surface);
                self.pending = None;
                changed = true;
            }
            Some(r) if self.panels.len() > k + 1 => {
                // Another row while a submenu is open from this panel: the
                // submenu closes once the pointer has rested here, so a
                // pointer cutting the corner on its way into the submenu
                // is not punished for it.
                if self.pending.map(|p| (p.panel, p.row)) != Some((k, r)) {
                    self.pending = Some(Pending {
                        panel: k,
                        row: r,
                        since: ctx.now_ms,
                    });
                }
                ctx.request_animation();
            }
            _ => self.pending = None,
        }
        if changed { Handled::Yes } else { Handled::No }
    }

    fn released(&mut self, p: Point, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some((k, Some(row))) = self.hit(p) else {
            return Handled::No;
        };
        let items = self.panel_items(k);
        let item = &items[row];
        if !item.selectable() {
            return Handled::No;
        }
        if !item.items.is_empty() {
            if !self.submenu_of(k, row) {
                let surface = Size::new(ctx.bounds.width as u32, ctx.bounds.height as u32);
                self.open_submenu(k, row, ctx.text, surface);
            }
            self.pending = None;
            return Handled::Yes;
        }
        let index = flat_index(&self.items, &self.panels[k].path, row);
        ctx.emit((self.message)(MenuEvent::Picked(index)));
        Handled::Yes
    }

    fn pressed(&mut self, p: Point, ctx: &mut EventCtx<'_, M>) -> Handled {
        if self.hit(p).is_some() {
            return Handled::Yes;
        }
        match self.title_at(p) {
            Some(title) if Some(title) != self.open_title => {
                ctx.emit((self.message)(MenuEvent::Title(title)));
            }
            _ => ctx.emit((self.message)(MenuEvent::Dismissed)),
        }
        Handled::Yes
    }

    fn key(&mut self, code: KeyCode, ctx: &mut EventCtx<'_, M>) -> Handled {
        let deepest = self.panels.len() - 1;
        let items = self.panel_items(deepest);
        let selected = self.panels[deepest].selected;
        match code {
            KeyCode::ArrowDown => {
                self.panels[deepest].selected = Self::step(items, selected, 1);
                self.panels[deepest].hovered = None;
                Handled::Yes
            }
            KeyCode::ArrowUp => {
                self.panels[deepest].selected = Self::step(items, selected, -1);
                self.panels[deepest].hovered = None;
                Handled::Yes
            }
            KeyCode::ArrowRight => {
                match selected {
                    Some(row) if !items[row].items.is_empty() => {
                        let surface = Size::new(ctx.bounds.width as u32, ctx.bounds.height as u32);
                        self.open_submenu(deepest, row, ctx.text, surface);
                        let last = self.panels.len() - 1;
                        let child = self.panel_items(last);
                        self.panels[last].selected = Self::step(child, None, 1);
                    }
                    _ => {
                        if let Some(open) = self.open_title
                            && !self.titles.is_empty()
                        {
                            let next = (open + 1) % self.titles.len();
                            ctx.emit((self.message)(MenuEvent::Title(next)));
                        }
                    }
                }
                Handled::Yes
            }
            KeyCode::ArrowLeft => {
                if deepest > 0 {
                    self.panels.truncate(deepest);
                } else if let Some(open) = self.open_title
                    && !self.titles.is_empty()
                {
                    let prev = (open + self.titles.len() - 1) % self.titles.len();
                    ctx.emit((self.message)(MenuEvent::Title(prev)));
                }
                Handled::Yes
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(row) = selected {
                    if !items[row].items.is_empty() {
                        let surface = Size::new(ctx.bounds.width as u32, ctx.bounds.height as u32);
                        self.open_submenu(deepest, row, ctx.text, surface);
                    } else {
                        let index = flat_index(&self.items, &self.panels[deepest].path, row);
                        ctx.emit((self.message)(MenuEvent::Picked(index)));
                    }
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn paint_panel(&self, k: usize, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let theme = ctx.theme;
        let g = self.geometry;
        let panel = &self.panels[k];
        let rect = panel.rect;
        let items = self.panel_items(k);

        // A shadow under the panel, two soft steps of it, then the panel.
        let s = i32::from(self.style.size_px).max(8);
        let drop = (s / 6).max(2);
        canvas.fill_rounded_rect(
            rect.translate(0, drop).inflate(1),
            g.radius_panel + 1,
            Color::rgba(0, 0, 0, 28),
        );
        canvas.fill_rounded_rect(
            rect.translate(0, drop / 2),
            g.radius_panel,
            Color::rgba(0, 0, 0, 40),
        );
        canvas.fill_rounded_rect(rect, g.radius_panel, theme.color(Role::Base200));
        canvas.stroke_rounded_rect(rect, g.radius_panel, g.border, theme.color(Role::Base300));

        let fg_normal = theme.color(Role::BaseContent);
        let dim = fg_normal.mix(theme.color(Role::Base200), 140);
        let rule = theme.color(Role::Base300).mix(fg_normal, 40);
        let tick_gap = g.tick_w - ctx.text.measure_line(self.style, CHECK);

        for (i, item) in items.iter().enumerate() {
            let row = g.row_rect(rect, items, i);
            if item.separator {
                let y = row.y + row.height / 2;
                canvas.fill_rect(
                    Rect::new(row.x + g.pad_x / 2, y, row.width - g.pad_x, g.border),
                    rule,
                );
                continue;
            }
            let highlighted = item.selectable()
                && (panel.selected == Some(i) || panel.hovered == Some(i) || self.submenu_of(k, i));
            let fg = if highlighted {
                canvas.fill_rounded_rect(row, g.radius_row, theme.color(Role::Primary));
                theme.content_of(Role::Primary)
            } else if item.enabled {
                fg_normal
            } else {
                dim
            };
            let baseline = row.y + (row.height - g.line_h) / 2 + g.ascent;
            let x = row.x + g.pad_x;
            if item.checked {
                ctx.text
                    .draw_line(canvas, self.style, Point::new(x, baseline), CHECK, fg);
            }
            ctx.text.draw_line(
                canvas,
                self.style,
                Point::new(x + g.tick_w, baseline),
                &item.label,
                fg,
            );
            let right = row.right() - g.pad_x;
            if !item.items.is_empty() {
                // A small triangle pointing the way the submenu opens.
                let w = (g.chevron_w - tick_gap).max(3);
                let h = (w * 5 / 4).max(4);
                let cy = row.y + row.height / 2;
                let x0 = right - w;
                let fx = |x: i32, y: i32| (x * 256, y * 256);
                canvas.fill_polygon_fx(
                    &[fx(x0, cy - h / 2), fx(x0 + w, cy), fx(x0, cy + h / 2)],
                    if highlighted { fg } else { dim },
                );
            } else if !item.shortcut.is_empty() {
                let keys = ctx.text.measure_line(self.style, &item.shortcut);
                ctx.text.draw_line(
                    canvas,
                    self.style,
                    Point::new(right - keys, baseline),
                    &item.shortcut,
                    if highlighted { fg } else { dim },
                );
            }
        }
    }
}

impl<M: 'static> Widget<M> for Menu<M> {
    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        for k in 0..self.panels.len() {
            self.paint_panel(k, ctx, canvas);
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Event::Input(input) = event else {
            return Handled::No;
        };
        match input {
            InputEvent::PointerMoved { position } => self.pointer_moved(*position, ctx),
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position,
                ..
            } => self.pressed(*position, ctx),
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                position,
                ..
            } => self.released(*position, ctx),
            InputEvent::PointerButton {
                state: ElementState::Down,
                position,
                ..
            } => {
                // Any other button outside the menu dismisses it too.
                if self.hit(*position).is_none() {
                    ctx.emit((self.message)(MenuEvent::Dismissed));
                }
                Handled::Yes
            }
            InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            } => self.key(*code, ctx),
            _ => Handled::No,
        }
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        let Some(pending) = self.pending else {
            return Animation::NONE;
        };
        if now_ms.saturating_sub(pending.since) < FOLLOW_MS {
            return Animation {
                repaint: false,
                next: Wake::At(pending.since + FOLLOW_MS),
            };
        }
        // The pointer has rested on the row: the submenu that was open beside
        // another row closes.
        self.pending = None;
        if pending.panel < self.panels.len() {
            self.panels.truncate(pending.panel + 1);
        }
        Animation {
            repaint: true,
            next: Wake::Never,
        }
    }
}
