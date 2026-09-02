//! A vertical list of rows, one of them selected.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role, Theme};
use denise_render::Pen;
use denise_render::icon::Icon;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{
    Align, ClickPair, Intent, RowKind, columns, draw_aligned, focus_ring, hovered_row, row_colors,
};

/// One row: a label, and optionally something before and after it.
///
/// ```
/// # use denise_ui::ListItem;
/// ListItem::new("Kjøletemperatur").with_trailing("4 °C");
/// ListItem::new("Avriming").with_leading(">").disabled();
/// ListItem::new("button").with_leading_icon(&denise_ui::widgets::icons::BUTTON);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    text: String,
    leading: String,
    icon: Option<&'static Icon>,
    trailing: String,
    enabled: bool,
}

impl ListItem {
    /// An enabled row carrying `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            leading: String::new(),
            icon: None,
            trailing: String::new(),
            enabled: true,
        }
    }

    /// Puts a short run of text in a column before the label.
    ///
    /// The column is as wide as the widest leading text in the list, so the
    /// labels line up rather than each starting wherever its own marker ended.
    pub fn with_leading(mut self, leading: impl Into<String>) -> Self {
        self.leading = leading.into();
        self
    }

    /// Puts a glyph in the column before the label, instead of leading text.
    ///
    /// Drawn square at the text size, centred in the leading column, in the
    /// row's own content colour — so it mutes with a disabled row and inverts
    /// on a selected one, exactly as the label does. A row with both an icon
    /// and leading text shows the icon; the column is shared, and two markers
    /// in it would jostle.
    pub fn with_leading_icon(mut self, icon: &'static Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Puts text at the trailing edge of the row: a value, a unit, a state.
    pub fn with_trailing(mut self, trailing: impl Into<String>) -> Self {
        self.trailing = trailing.into();
        self
    }

    /// Makes the row unselectable: skipped by the keyboard, inert to the mouse.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The label.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The leading text, empty when there is none.
    #[inline]
    pub fn leading(&self) -> &str {
        &self.leading
    }

    /// The leading glyph, if the row has one.
    #[inline]
    pub const fn leading_icon(&self) -> Option<&'static Icon> {
        self.icon
    }

    /// The trailing text, empty when there is none.
    #[inline]
    pub fn trailing(&self) -> &str {
        &self.trailing
    }

    /// Whether this row can be selected.
    #[inline]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Width the trailing text occupies, or zero when there is none.
    fn trailing_width(&self, engine: &mut TextEngine, style: TextStyle) -> i32 {
        if self.trailing.is_empty() {
            0
        } else {
            engine.measure_line(style, &self.trailing)
        }
    }

    /// Replaces the label.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Replaces the trailing text.
    pub fn set_trailing(&mut self, trailing: impl Into<String>) {
        self.trailing = trailing.into();
    }

    /// Enables or disables the row.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl From<&str> for ListItem {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for ListItem {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

/// A column of rows, at most one of them selected.
///
/// The backbone of a settings screen and of a menu.
///
/// ```
/// # use denise_ui::List;
/// enum Message { Row(usize), Open(usize) }
/// List::new(["Nettverk", "Skjerm", "Om"], Message::Row).on_activate(Message::Open);
/// ```
///
/// # Scrolling belongs to the tree, and this widget cooperates with it
///
/// A list longer than its rectangle draws the rows that fit and stops — the
/// widget itself never scrolls. The scrolling version is a *viewport*: put the
/// list, sized with [`preferred_height`](List::preferred_height), inside a node
/// marked [`Ui::set_scrollable`](crate::Ui::set_scrollable), and the tree does
/// the rest — wheel, page keys, touch-drag on the background, clipping and hit
/// testing all agree because one reflow computes them.
///
/// ```
/// # use denise::{Rect, Size, theme};
/// # use denise_ui::{Ui, widgets::Panel};
/// # #[derive(Clone, Debug)] enum Msg { Noop }
/// # fn demo() -> Option<()> {
/// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// # let root = ui.root();
/// # let list = denise_ui::List::new(["Nettverk", "Skjerm"], |_| Msg::Noop);
/// let viewport = ui.add(root, Panel::default(), Rect::new(20, 20, 240, 160))?;
/// ui.set_scrollable(viewport, true);
/// let height = list.preferred_height(ui.theme());
/// let list = ui.add(viewport, list, Rect::new(0, 0, 240, height))?;
/// # Some(()) }
/// ```
///
/// The one thing the widget contributes: **a keyboard selection below the fold
/// pulls the viewport along.** Moving the selection reveals the selected row's
/// rectangle ([`EventCtx::reveal`](crate::EventCtx::reveal)), the same way
/// focus reveals a focused widget.
///
/// On a panel with one fixed resolution, "it fits" is still a fine answer —
/// [`visible_rows`](List::visible_rows) says how many rows a rectangle holds.
///
/// # Selection, and why it is optional here
///
/// [`RadioGroup`](super::RadioGroup) makes *nothing selected* unrepresentable,
/// because a group of options always has an answer. A list does not: a settings
/// screen opens with no row chosen, and a search result list with no match has
/// nothing to choose. So the selection is an [`Option`], and the keyboard's first
/// press lands on the near end rather than on a row nobody picked.
///
/// # Keyboard
///
/// The list is **one node, so one tab stop**. Up and Down move the selection and
/// **do not wrap** — deliberately unlike `RadioGroup` and [`Tabs`](super::Tabs),
/// where the whole set is visible at once and coming round from the end is
/// obvious. A hundred-row list that jumped from the bottom to the top under a held
/// key would be disorienting. `Home` and `End` go to the ends. `Enter` activates.
///
/// # Activation
///
/// Selecting a row and *acting* on it are two different things, so they are two
/// different messages. `Enter` activates, and so does a double-click.
///
/// A double-click is a mouse convention, and this toolkit's usual home is a
/// touchscreen where a double-tap is unreliable and unexpected. For a list whose
/// rows are commands rather than choices — a menu — use
/// [`activate_on_click`](List::activate_on_click), and one tap both selects and
/// activates.
///
/// **Double-click detection reads [`Ui::tick`](crate::Ui::tick)'s clock.** An
/// application that never calls `tick` has a clock frozen at zero, and every
/// second click on a row will read as a double-click.
///
/// [#21]: https://github.com/bisand/denise/issues/21
#[derive(Clone, Debug)]
pub struct List<M> {
    items: Vec<ListItem>,
    selected: Option<usize>,
    hovered: Option<usize>,
    row_height: Option<i32>,
    selection: Option<fn(usize) -> M>,
    activation: Option<fn(usize) -> M>,
    single_click: bool,
    clicks: ClickPair,
    role: Role,
    style: TextStyle,
}

impl<M> List<M> {
    /// A list with nothing selected, reporting selection through `message`.
    pub fn new(
        items: impl IntoIterator<Item = impl Into<ListItem>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            selected: None,
            hovered: None,
            row_height: None,
            selection: Some(message),
            activation: None,
            single_click: false,
            clicks: ClickPair::default(),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// A list that emits nothing, for a selection the application reads rather
    /// than reacts to.
    pub fn inert(items: impl IntoIterator<Item = impl Into<ListItem>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            selected: None,
            hovered: None,
            row_height: None,
            selection: None,
            activation: None,
            single_click: false,
            clicks: ClickPair::default(),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the message sent when a row is activated by `Enter` or a
    /// double-click.
    ///
    /// Without one, a list is a chooser and nothing happens on activation.
    pub fn on_activate(mut self, message: fn(usize) -> M) -> Self {
        self.activation = Some(message);
        self
    }

    /// Makes a single click or tap activate as well as select.
    ///
    /// For rows that are commands rather than choices. See the note on the type:
    /// a double-tap on a panel is unreliable, and this is the honest answer for a
    /// menu driven by a finger.
    pub fn activate_on_click(mut self) -> Self {
        self.single_click = true;
        self
    }

    /// Sets the initially selected row. Out of range selects nothing.
    pub fn with_selected(mut self, index: Option<usize>) -> Self {
        self.set_selected(index);
        self
    }

    /// Sets the height of every row, overriding the theme's field height.
    pub fn with_row_height(mut self, height: i32) -> Self {
        self.row_height = Some(height.max(1));
        self
    }

    /// Sets the colour role of the selected row.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the rows' font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The selected row, if any.
    #[inline]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The selected row's item, if any.
    #[inline]
    pub fn selected_item(&self) -> Option<&ListItem> {
        self.items.get(self.selected?)
    }

    /// Selects a row **without emitting anything**.
    ///
    /// An index that does not exist selects nothing, rather than being clamped to
    /// the nearest row that does: unlike [`RadioGroup`](super::RadioGroup), having
    /// nothing selected is a state this widget can represent, so it is the honest
    /// answer to being asked for a row that is not there.
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| *index < self.items.len());
    }

    /// The rows, in order.
    #[inline]
    pub fn items(&self) -> &[ListItem] {
        &self.items
    }

    /// Replaces the rows, dropping a selection that no longer exists.
    pub fn set_items(&mut self, items: impl IntoIterator<Item = impl Into<ListItem>>) {
        self.items = items.into_iter().map(Into::into).collect();
        self.set_selected(self.selected);
        // The remembered click points at a row that may now be something else
        // entirely, and a stale half-pair would activate the wrong thing.
        self.clicks.forget();
    }

    /// Enables or disables one row. Out of range does nothing.
    pub fn set_row_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(item) = self.items.get_mut(index) {
            item.set_enabled(enabled);
        }
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the rows' font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Height every row is drawn at.
    pub fn row_height(&self, theme: &Theme) -> i32 {
        self.row_height.unwrap_or(theme.metrics.size_field).max(1)
    }

    /// How many rows fit in `height`.
    ///
    /// The counterpart to [`preferred_height`](List::preferred_height) for a
    /// caller who has a rectangle and needs to know what it can show. See the
    /// note on the type: what does not fit is not drawn.
    pub fn visible_rows(&self, theme: &Theme, height: i32) -> usize {
        if height <= 0 {
            return 0;
        }
        (height / self.row_height(theme)) as usize
    }

    /// Height this list needs to show every row.
    pub fn preferred_height(&self, theme: &Theme) -> i32 {
        let rows = self.items.len().max(1) as i64;
        // Widened: a caller is entitled to a list longer than `i32::MAX / 36`,
        // and a panic in a sizing query is a black screen on a kiosk.
        (i64::from(self.row_height(theme)) * rows).min(i64::from(i32::MAX)) as i32
    }

    /// Width the widest row needs, columns and padding included.
    pub fn preferred_width(&self, engine: &mut TextEngine) -> i32 {
        let pad = padding(self.style.size_px);
        let mut widest = |pick: fn(&ListItem) -> &str| -> i32 {
            self.items
                .iter()
                .map(|item| engine.measure_line(self.style, pick(item)))
                .max()
                .unwrap_or(0)
        };
        // The widest label and the widest trailing value may be on different
        // rows. Summing them is the width at which *no* row has to be squeezed,
        // which is the question a caller sizing a column is actually asking.
        let trailing = widest(ListItem::trailing);
        let text = widest(ListItem::text);
        let leading = self.leading_width(engine);
        let gap = |width: i32| if width > 0 { pad } else { 0 };
        pad * 2 + leading + gap(leading) + text + trailing + gap(trailing)
    }

    /// The side of the square a row's glyph is drawn in: the text size, so a
    /// glyph and the label beside it read at the same weight.
    const fn icon_side(&self) -> i32 {
        self.style.size_px as i32
    }

    /// Width of the leading column: the widest one in the list, so labels align.
    ///
    /// A glyph counts at its drawn side, so a list mixing icon rows and text
    /// rows still lines its labels up.
    fn leading_width(&self, engine: &mut TextEngine) -> i32 {
        let text = self
            .items
            .iter()
            .map(|item| engine.measure_line(self.style, &item.leading))
            .max()
            .unwrap_or(0);
        if self.items.iter().any(|item| item.icon.is_some()) {
            text.max(self.icon_side())
        } else {
            text
        }
    }

    /// Which enabled row is under `point`, if any.
    ///
    /// Disabled rows are inert to the mouse: they return `None` rather than the
    /// row they are, so a click on one falls through to nothing instead of
    /// selecting something the keyboard would refuse to reach.
    fn enabled_row_at(&self, bounds: Rect, row_height: i32, point: Point) -> Option<usize> {
        let index = row_at(bounds, row_height, self.items.len(), point)?;
        self.items.get(index).filter(|item| item.enabled)?;
        Some(index)
    }

    /// What a click on `row` at `at_ms` means. The pairing rules live in
    /// [`ClickPair`], shared with [`Table`](super::Table).
    fn classify_click(&mut self, row: usize, at_ms: u64) -> Intent {
        self.clicks.classify(row, at_ms, self.single_click)
    }

    /// Moves the selection, reporting it. `None` means the end of the list.
    fn select(&mut self, target: Option<usize>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some(target) = target else {
            // Already at the end. The key was still this widget's to handle:
            // letting Down escape from the bottom row would hand it to whatever
            // the toolkit does with an unhandled key next.
            return Handled::Yes;
        };
        if self.selected == Some(target) {
            return Handled::Yes;
        }
        self.selected = Some(target);
        // A selection that walked below the fold pulls its viewport along: the
        // list reveals the selected row's rectangle and every scrollable
        // ancestor scrolls it into view — the same mechanism focus uses.
        ctx.reveal(row_rect(ctx.bounds, self.row_height(ctx.theme), target));
        if let Some(message) = self.selection {
            ctx.emit(message(target));
        }
        Handled::Yes
    }

    /// Reports an activation, if anybody asked for one.
    fn activate(&mut self, row: usize, ctx: &mut EventCtx<'_, M>) -> Handled {
        if let Some(message) = self.activation {
            ctx.emit(message(row));
        }
        Handled::Yes
    }
}

/// Space between a row's content and its edge, on one side.
#[inline]
const fn padding(size_px: u16) -> i32 {
    // `Ord::max` is not const yet.
    let half = size_px as i32 / 2;
    if half < 4 { 4 } else { half }
}

/// The first row that can be selected.
fn first_enabled(items: &[ListItem]) -> Option<usize> {
    items.iter().position(ListItem::is_enabled)
}

/// The last row that can be selected.
fn last_enabled(items: &[ListItem]) -> Option<usize> {
    items.iter().rposition(ListItem::is_enabled)
}

/// The next selectable row in the direction of travel, or `None` at the end.
///
/// **This does not wrap.** `None` means *stay where you are*, and it is the whole
/// difference between this widget's keyboard and `RadioGroup`'s: a set of options
/// a person can see all at once may come round from the last to the first, and a
/// long list may not.
///
/// Disabled rows are stepped over, however many of them are in a run. Starting
/// from nothing selected lands on the near end, which is what makes the first
/// key press after `Tab` do something visible.
fn step(items: &[ListItem], from: Option<usize>, forward: bool) -> Option<usize> {
    let count = items.len();
    if count == 0 {
        return None;
    }
    let Some(from) = from else {
        return if forward {
            first_enabled(items)
        } else {
            last_enabled(items)
        };
    };
    let mut index = from.min(count - 1);
    loop {
        index = if forward {
            if index + 1 >= count {
                return None;
            }
            index + 1
        } else {
            if index == 0 {
                return None;
            }
            index - 1
        };
        if items[index].enabled {
            return Some(index);
        }
    }
}

/// Where one row sits.
///
/// Rows are a **fixed** height stacked from the top, not an even division of the
/// bounds: adding a row to a list must not make every other row shorter.
fn row_rect(bounds: Rect, row_height: i32, index: usize) -> Rect {
    let height = row_height.max(1);
    // Widened and clamped at both ends: `height * index` overflows an `i32` for a
    // list a caller is entitled to build, and a `usize` index can overflow the
    // widened multiply too. This is arithmetic on somebody else's numbers, and a
    // panic inside a paint loop on a kiosk is a black screen.
    let index = index.min(i32::MAX as usize) as i64;
    let ceiling = i64::from(i32::MAX - height);
    let y = (i64::from(bounds.y) + i64::from(height) * index).min(ceiling) as i32;
    Rect::new(bounds.x, y, bounds.width, height)
}

/// Which row contains `point`, if any.
///
/// Unlike [`RadioGroup`](super::RadioGroup), which divides its bounds among its
/// options, a list has rows of a fixed height and empty space below the last one.
/// A click there is not a click on the last row.
fn row_at(bounds: Rect, row_height: i32, count: usize, point: Point) -> Option<usize> {
    if count == 0 || !bounds.contains(point) {
        return None;
    }
    let index = (i64::from(point.y - bounds.y) / i64::from(row_height.max(1))) as usize;
    (index < count).then_some(index)
}

impl<M: 'static> Widget<M> for List<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        Measured::both(
            self.preferred_width(ctx.text),
            self.preferred_height(ctx.theme),
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() || self.items.is_empty() {
            return;
        }
        // The list paints its own background, which is what lets it promise its
        // rows are readable at all: a widget drawing text straight onto whatever
        // a caller happened to put behind it can promise nothing.
        let (backdrop, _) = row_colors(ctx.theme, ctx.state, self.role, RowKind::Resting, true);
        canvas.fill_rect(bounds, backdrop);

        let row_height = self.row_height(ctx.theme);
        let pad = padding(self.style.size_px);
        let leading_width = self.leading_width(ctx.text);
        let radius = ctx.theme.radius(Radius::Field);
        let hovered = hovered_row(ctx.state, self.hovered);

        for (index, item) in self.items.iter().enumerate() {
            let row = row_rect(bounds, row_height, index);
            if row.y >= bounds.bottom() {
                // Past the bottom edge. Nothing below here is drawn or hittable —
                // this list does not scroll; see the note on the type.
                break;
            }
            let kind = if self.selected == Some(index) {
                RowKind::Selected
            } else if hovered == Some(index) {
                RowKind::Hovered
            } else {
                RowKind::Resting
            };
            let (fill, content) = row_colors(ctx.theme, ctx.state, self.role, kind, item.enabled);
            if kind != RowKind::Resting {
                canvas.fill_rounded_rect(row, radius, fill);
            }
            if kind == RowKind::Selected && ctx.state.contains(VisualState::FOCUSED) {
                focus_ring(ctx.theme, row, radius, canvas);
            }

            let (leading, label, trailing) = columns(
                row,
                pad,
                leading_width,
                item.trailing_width(ctx.text, self.style),
            );
            // A glyph takes the leading column's place; see `with_leading_icon`.
            if let Some(icon) = item.icon {
                let side = self.icon_side().min(leading.width).min(row.height).max(1);
                let box_ = Rect::new(
                    leading.x + (leading.width - side) / 2,
                    row.y + (row.height - side) / 2,
                    side,
                    side,
                );
                // The knockouts are cut in whatever this row is actually
                // painted on, which for a resting row is the list's backdrop.
                let back = if kind == RowKind::Resting {
                    backdrop
                } else {
                    fill
                };
                canvas.draw_icon(icon, box_, content, back);
            }
            let leading_text = if item.icon.is_some() {
                ""
            } else {
                item.leading.as_str()
            };
            for (box_of, text, align) in [
                (leading, leading_text, Align::Start),
                (label, item.text.as_str(), Align::Start),
                (trailing, item.trailing.as_str(), Align::End),
            ] {
                if text.is_empty() || box_of.is_empty() {
                    continue;
                }
                // Clipped to its own column, so a label longer than the space it
                // was given is cut off rather than drawn across the value at the
                // other end of the row.
                let mut column = canvas.with_clip(box_of);
                draw_aligned(
                    &mut column,
                    ctx.text,
                    self.style,
                    box_of,
                    (align, Align::Center),
                    text,
                    content,
                );
            }
        }

        // A focused list with nothing selected has no row to ring, and a keyboard
        // user who has just pressed Tab needs to see where they are.
        if ctx.state.contains(VisualState::FOCUSED) && self.selected.is_none() {
            focus_ring(ctx.theme, bounds, radius, canvas);
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        if self.items.is_empty() {
            return Handled::No;
        }
        let row_height = self.row_height(ctx.theme);

        match event {
            Event::Input(InputEvent::PointerMoved { position }) => {
                let row = self.enabled_row_at(ctx.bounds, row_height, *position);
                if row == self.hovered {
                    // Most moves are within the same row. Repainting a whole list
                    // for every one of them is how an idle panel stops being idle.
                    return Handled::No;
                }
                self.hovered = row;
                Handled::Yes
            }
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            })
            | Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => {
                let Some(row) = self.enabled_row_at(ctx.bounds, row_height, *position) else {
                    return Handled::No;
                };
                let intent = self.classify_click(row, ctx.now_ms);
                // Selection first, always: the second click of a pair finds the
                // row already selected and emits nothing, so an activation is one
                // selection and one activation rather than two of each.
                let handled = self.select(Some(row), ctx);
                if intent == Intent::Activate {
                    self.activate(row, ctx);
                }
                handled
            }
            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => match code {
                KeyCode::ArrowDown => {
                    let target = step(&self.items, self.selected, true);
                    self.select(target, ctx)
                }
                KeyCode::ArrowUp => {
                    let target = step(&self.items, self.selected, false);
                    self.select(target, ctx)
                }
                KeyCode::Home => {
                    let target = first_enabled(&self.items);
                    self.select(target, ctx)
                }
                KeyCode::End => {
                    let target = last_enabled(&self.items);
                    self.select(target, ctx)
                }
                KeyCode::Enter | KeyCode::NumpadEnter => match self.selected {
                    Some(row) if self.items.get(row).is_some_and(ListItem::is_enabled) => {
                        self.activate(row, ctx)
                    }
                    _ => Handled::No,
                },
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// A list with no row anybody can choose is not a tab stop. Focusing one
    /// strands a keyboard-only panel on a widget no key does anything to.
    fn focusable(&self) -> bool {
        self.items.iter().any(ListItem::is_enabled)
    }
}

impl<M> Describe for List<M> {
    const KIND: &'static str = "list";
    const DOC: &'static str = "Rows in a column, one of them selected.";
    const GROUP: Group = Group::Data;
    const ICON: &'static denise_render::icon::Icon = &super::icons::LIST;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "item",
            PropertyKind::List,
            "The rows, as `item` child nodes. Real data: a navigation list's rows are the form's own.",
        ),
        Property::new(
            "selected",
            PropertyKind::Int {
                min: 0,
                max: i32::MAX,
            },
            "The selected row. An index no row has selects nothing, rather than the nearest row.",
        ),
        Property::new(
            "on-select",
            PropertyKind::Message(Payload::Index),
            "Emitted with the row's index whenever the selection moves.",
        ),
        Property::new(
            "on-activate",
            PropertyKind::Message(Payload::Index),
            "Emitted on Enter, or a double press.",
        ),
        Property::new(
            "activate-on-click",
            PropertyKind::Bool,
            "A single press activates as well as selects, for rows that are commands.",
        ),
        Property::new(
            "row-height",
            PropertyKind::Int { min: 16, max: 200 },
            "Height of every row in logical pixels, overriding the theme's field height.",
        )
        .in_pixels(),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role of the selected row.",
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
            // Nothing selected and no row height of its own report nothing at
            // all, which is what lets a writer leave a property at its default
            // out of the file.
            "selected" => Value::Int(i32::try_from(self.selected?).unwrap_or(i32::MAX)),
            "activate-on-click" => Value::Bool(self.single_click),
            "row-height" => Value::Int(self.row_height?),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, so a row that is not there selects nothing
            // here exactly as it does everywhere else.
            "selected" => self.set_selected(Some(value.as_index()?)),
            // Built from the child nodes; an inspector edits them
            // where they live. See `PropertyKind::List`.
            "on-select" | "on-activate" | "item" => return Err(Mismatch::Supplied),
            "activate-on-click" => self.single_click = value.as_bool()?,
            "row-height" => self.row_height = Some(value.as_int()?.max(1)),
            "role" => self.role = value.as_role()?,
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::style::DOUBLE_CLICK_MS;
    use denise::theme;

    fn items() -> Vec<ListItem> {
        alloc::vec![
            ListItem::new("Nettverk"),
            ListItem::new("Skjerm").with_trailing("70 %"),
            ListItem::new("Om"),
        ]
    }

    fn list() -> List<usize> {
        List::new(items(), |index| index)
    }

    /// Rows are a fixed height stacked from the top, not an even division of the
    /// bounds. Adding a row must not make every other row shorter, which is what
    /// separates a list from a `RadioGroup`.
    #[test]
    fn rows_are_a_fixed_height_stacked_from_the_top() {
        let bounds = Rect::new(10, 20, 200, 500);
        let mut previous = bounds.y;
        for index in 0..8 {
            let row = row_rect(bounds, 36, index);
            assert_eq!(row.y, previous, "row {index} does not follow the last");
            assert_eq!(row.height, 36, "row {index} is not the height it was given");
            assert_eq!(row.x, bounds.x);
            assert_eq!(row.right(), bounds.right());
            previous = row.bottom();
        }
        // And the height of a row does not depend on how many there are.
        assert_eq!(row_rect(bounds, 36, 0), row_rect(bounds, 36, 0));
    }

    /// A list far longer than a rectangle can address must not overflow the
    /// arithmetic that places its rows. A panic inside a paint loop on a kiosk is
    /// a black screen.
    #[test]
    fn an_absurdly_long_list_neither_overflows_nor_panics() {
        let bounds = Rect::new(0, 0, 300, 400);
        let row = row_rect(bounds, 36, usize::MAX / 2);
        assert!(row.height > 0);
        assert!(row.bottom() >= row.y, "the rectangle inverted");
        assert!(
            row.y >= bounds.bottom(),
            "an unreachable row should be past it"
        );
    }

    /// Every point in a row belongs to it, and the empty space below the last row
    /// belongs to nothing. Unlike a `RadioGroup`, a list does not stretch its rows
    /// to fill the rectangle, so a click under the last one is a click on the
    /// list's background.
    #[test]
    fn a_point_below_the_last_row_is_not_the_last_row() {
        let bounds = Rect::new(10, 20, 200, 400);
        let count = 3;
        for index in 0..count {
            let row = row_rect(bounds, 36, index);
            for y in row.y..row.bottom() {
                assert_eq!(
                    row_at(bounds, 36, count, Point::new(bounds.x + 5, y)),
                    Some(index),
                    "y {y} should be row {index}"
                );
            }
        }
        assert_eq!(
            row_at(bounds, 36, count, Point::new(15, 20 + 36 * 3)),
            None,
            "the first pixel past the last row belongs to no row"
        );
        assert_eq!(row_at(bounds, 36, count, Point::new(5, 30)), None, "left");
        assert_eq!(row_at(bounds, 36, count, Point::new(15, 5)), None, "above");
        assert_eq!(row_at(bounds, 36, 0, Point::new(15, 30)), None, "no rows");
    }

    /// The headline difference from `RadioGroup` and `Tabs`: the ends are ends.
    #[test]
    fn the_selection_stops_at_the_ends_rather_than_wrapping() {
        let items = items();
        assert_eq!(step(&items, Some(0), true), Some(1));
        assert_eq!(step(&items, Some(1), true), Some(2));
        assert_eq!(
            step(&items, Some(2), true),
            None,
            "past the last row must stay on the last row"
        );
        assert_eq!(step(&items, Some(2), false), Some(1));
        assert_eq!(
            step(&items, Some(0), false),
            None,
            "and before the first must stay on the first"
        );
    }

    /// With nothing selected, the first press lands on the near end — otherwise
    /// tabbing into a list and pressing Down does nothing visible.
    #[test]
    fn the_first_press_with_nothing_selected_lands_on_the_near_end() {
        let items = items();
        assert_eq!(step(&items, None, true), Some(0));
        assert_eq!(step(&items, None, false), Some(2));
    }

    /// Disabled rows are stepped over in both directions, however long the run.
    #[test]
    fn disabled_rows_are_skipped_in_both_directions() {
        let items = alloc::vec![
            ListItem::new("first"),
            ListItem::new("a").disabled(),
            ListItem::new("b").disabled(),
            ListItem::new("middle"),
            ListItem::new("c").disabled(),
            ListItem::new("last"),
        ];
        assert_eq!(step(&items, Some(0), true), Some(3), "over a run of two");
        assert_eq!(step(&items, Some(3), true), Some(5), "over a run of one");
        assert_eq!(step(&items, Some(5), false), Some(3));
        assert_eq!(step(&items, Some(3), false), Some(0));

        // And the ends are the last *enabled* rows, not the last rows.
        assert_eq!(first_enabled(&items), Some(0));
        assert_eq!(last_enabled(&items), Some(5));
    }

    /// A disabled row at the end is not a place the keyboard can be stranded.
    #[test]
    fn a_disabled_row_at_the_end_is_not_reachable() {
        let items = alloc::vec![ListItem::new("only"), ListItem::new("tail").disabled(),];
        assert_eq!(step(&items, Some(0), true), None);
        assert_eq!(last_enabled(&items), Some(0));
        assert_eq!(step(&items, None, false), Some(0), "End skips the tail");
    }

    /// A list with nothing selectable must not spin looking for a row, and is not
    /// a tab stop.
    #[test]
    fn a_list_with_nothing_selectable_is_inert_rather_than_broken() {
        let items = alloc::vec![ListItem::new("a").disabled(), ListItem::new("b").disabled(),];
        assert_eq!(step(&items, None, true), None);
        assert_eq!(step(&items, None, false), None);
        assert_eq!(step(&items, Some(0), true), None);
        assert_eq!(first_enabled(&items), None);
        assert_eq!(last_enabled(&items), None);

        let list: List<usize> = List::new(items, |index| index);
        assert!(!Widget::<usize>::focusable(&list));
    }

    /// An empty list is not a tab stop and none of the arithmetic that assumes a
    /// row may panic on it.
    #[test]
    fn an_empty_list_is_inert_rather_than_broken() {
        let mut list: List<usize> = List::inert(Vec::<ListItem>::new());
        assert_eq!(list.selected(), None);
        assert_eq!(list.selected_item(), None);
        assert!(!Widget::<usize>::focusable(&list));
        list.set_selected(Some(0));
        assert_eq!(list.selected(), None);
        assert_eq!(step(&[], None, true), None);
        assert!(list.preferred_height(&theme::DARK) > 0);
    }

    /// A disabled row is inert to the mouse as well as to the keyboard. A row the
    /// keyboard refuses to reach must not be selectable by clicking it.
    #[test]
    fn a_disabled_row_is_inert_to_the_mouse() {
        let list: List<usize> = List::new(
            alloc::vec![
                ListItem::new("on"),
                ListItem::new("off").disabled(),
                ListItem::new("on again"),
            ],
            |index| index,
        );
        let bounds = Rect::new(0, 0, 200, 200);
        let at = |index: i32| Point::new(10, index * 36 + 5);
        assert_eq!(list.enabled_row_at(bounds, 36, at(0)), Some(0));
        assert_eq!(
            list.enabled_row_at(bounds, 36, at(1)),
            None,
            "a click on a disabled row selects nothing"
        );
        assert_eq!(list.enabled_row_at(bounds, 36, at(2)), Some(2));
    }

    /// Activation is a separate thing from selection, and it takes two clicks
    /// inside the window on the same row.
    #[test]
    fn two_clicks_on_one_row_inside_the_window_are_an_activation() {
        let mut list = list();
        assert_eq!(list.classify_click(1, 1_000), Intent::Select);
        assert_eq!(
            list.classify_click(1, 1_000 + DOUBLE_CLICK_MS),
            Intent::Activate,
            "the edge of the window is still inside it"
        );
        // And the pair is spent: a third click starts a new one rather than
        // firing a second activation.
        assert_eq!(
            list.classify_click(1, 1_000 + DOUBLE_CLICK_MS),
            Intent::Select
        );
    }

    /// Two slow clicks are two selections, and two clicks on different rows are
    /// never a pair however fast they are.
    #[test]
    fn a_pair_needs_one_row_and_a_short_gap() {
        let mut slow = list();
        assert_eq!(slow.classify_click(0, 0), Intent::Select);
        assert_eq!(
            slow.classify_click(0, DOUBLE_CLICK_MS + 1),
            Intent::Select,
            "a slow second click is a second selection, not an activation"
        );

        let mut other = list();
        assert_eq!(other.classify_click(0, 100), Intent::Select);
        assert_eq!(
            other.classify_click(2, 110),
            Intent::Select,
            "two rows are never a pair"
        );
    }

    /// A menu on a touch panel activates on one tap: a double-tap is unreliable
    /// on a resistive screen and unexpected on any of them.
    #[test]
    fn a_single_click_list_activates_on_the_first_tap() {
        let mut list = list().activate_on_click();
        assert_eq!(list.classify_click(0, 0), Intent::Activate);
        assert_eq!(list.classify_click(0, 5_000), Intent::Activate);
    }

    /// Replacing the rows drops a remembered click, which would otherwise pair
    /// with a click on whatever row now has that index.
    #[test]
    fn replacing_the_rows_forgets_a_half_finished_pair() {
        let mut list = list();
        assert_eq!(list.classify_click(1, 0), Intent::Select);
        list.set_items(["helt", "andre", "rader"]);
        assert_eq!(
            list.classify_click(1, 10),
            Intent::Select,
            "a click on a new row should not complete an old pair"
        );
    }

    /// The selection is always a row that exists, including after the rows change
    /// under it. Out of range is nothing selected rather than the nearest row.
    #[test]
    fn the_selection_is_always_a_row_that_exists() {
        let mut list = list();
        list.set_selected(Some(2));
        assert_eq!(list.selected_item().map(ListItem::text), Some("Om"));
        list.set_selected(Some(99));
        assert_eq!(
            list.selected(),
            None,
            "an index that is not there is nothing"
        );

        list.set_selected(Some(2));
        list.set_items(["bare én"]);
        assert_eq!(list.selected(), None, "a shorter list drops the selection");
    }

    /// Hover is only real while the tree still says the pointer is here. The tree
    /// clears `HOVERED` without sending an event, so a widget that trusted its own
    /// memory would leave a row lit under a pointer somewhere else entirely.
    #[test]
    fn hover_clears_when_the_pointer_leaves_even_though_no_event_arrives() {
        assert_eq!(hovered_row(VisualState::HOVERED, Some(2)), Some(2));
        assert_eq!(
            hovered_row(VisualState::NONE, Some(2)),
            None,
            "a remembered row must not survive the pointer leaving"
        );
        assert_eq!(
            hovered_row(VisualState::FOCUSED | VisualState::HOVERED, Some(0)),
            Some(0)
        );
        assert_eq!(hovered_row(VisualState::HOVERED, None), None);
    }

    /// Every row's text has to be readable on that row's own fill, in every theme
    /// and every state — the guarantee the whole `interactive_pair` convention
    /// exists for.
    #[test]
    fn every_row_keeps_its_text_readable_in_every_theme() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for role in [Role::Primary, Role::Secondary, Role::Accent, Role::Neutral] {
                for state in [
                    VisualState::NONE,
                    VisualState::HOVERED,
                    VisualState::PRESSED,
                    VisualState::FOCUSED,
                    VisualState::DISABLED,
                ] {
                    for kind in [RowKind::Resting, RowKind::Hovered, RowKind::Selected] {
                        for enabled in [true, false] {
                            let (fill, text) = row_colors(&theme, state, role, kind, enabled);
                            let ratio = contrast_x100(fill, text);
                            assert!(
                                ratio >= AA_LARGE,
                                "{} {role:?} {state:?} {kind:?} enabled={enabled}: text on \
                                 row is {ratio}, floor is {AA_LARGE}",
                                theme.name
                            );
                        }
                    }
                }
            }
        }
    }

    /// A disabled list is still showing something, and *which row is selected* is
    /// most of it. `interactive_pair` recesses every role to the same `Base200`
    /// when disabled, so this needs its own answer — the same lesson `RadioGroup`
    /// learned about its disabled disc.
    #[test]
    fn a_disabled_list_still_shows_which_row_is_selected() {
        for theme in Theme::BUILT_IN {
            for role in [Role::Primary, Role::Secondary, Role::Accent] {
                let (resting, _) =
                    row_colors(&theme, VisualState::DISABLED, role, RowKind::Resting, true);
                let (selected, _) =
                    row_colors(&theme, VisualState::DISABLED, role, RowKind::Selected, true);
                assert_ne!(
                    resting, selected,
                    "{} {role:?}: a disabled list forgot which row was chosen",
                    theme.name
                );
            }
        }
    }

    /// A hovered row has to actually look different from a resting one, or the
    /// highlight is not a highlight.
    #[test]
    fn a_hovered_row_looks_different_from_a_resting_one() {
        for theme in Theme::BUILT_IN {
            let resting = row_colors(
                &theme,
                VisualState::NONE,
                Role::Primary,
                RowKind::Resting,
                true,
            );
            let hovered = row_colors(
                &theme,
                VisualState::NONE,
                Role::Primary,
                RowKind::Hovered,
                true,
            );
            let selected = row_colors(
                &theme,
                VisualState::NONE,
                Role::Primary,
                RowKind::Selected,
                true,
            );
            assert_ne!(resting.0, hovered.0, "{}: hover is invisible", theme.name);
            assert_ne!(
                resting.0, selected.0,
                "{}: selection is invisible",
                theme.name
            );
        }
    }

    /// The tree's hover bit is about the whole list. If it reached
    /// `interactive_pair`, every row would shift the moment the pointer entered
    /// the widget — twenty rows all lit at once, which is the opposite of a
    /// hover highlight.
    #[test]
    fn hovering_the_list_does_not_tint_every_row_in_it() {
        for theme in Theme::BUILT_IN {
            let idle = row_colors(
                &theme,
                VisualState::NONE,
                Role::Primary,
                RowKind::Resting,
                true,
            );
            let pointer_inside = row_colors(
                &theme,
                VisualState::HOVERED | VisualState::PRESSED,
                Role::Primary,
                RowKind::Resting,
                true,
            );
            assert_eq!(idle, pointer_inside, "{}", theme.name);
        }
    }

    /// A disabled row mutes, and a row in a disabled list does not: the colour
    /// `interactive_pair` derives for a disabled widget sits exactly on the
    /// contrast floor and has nothing left to give.
    #[test]
    fn a_disabled_list_does_not_mute_a_colour_that_has_nothing_left_to_give() {
        for theme in Theme::BUILT_IN {
            let (_, enabled_row) = row_colors(
                &theme,
                VisualState::NONE,
                Role::Primary,
                RowKind::Resting,
                true,
            );
            let (_, disabled_row) = row_colors(
                &theme,
                VisualState::NONE,
                Role::Primary,
                RowKind::Resting,
                false,
            );
            assert_ne!(
                enabled_row, disabled_row,
                "{}: a row nobody can choose should look like it",
                theme.name
            );

            let (_, in_a_disabled_list) = row_colors(
                &theme,
                VisualState::DISABLED,
                Role::Primary,
                RowKind::Resting,
                false,
            );
            let (_, whole_list) = row_colors(
                &theme,
                VisualState::DISABLED,
                Role::Primary,
                RowKind::Resting,
                true,
            );
            assert_eq!(
                in_a_disabled_list, whole_list,
                "{}: a row in a disabled list was muted below its own floor",
                theme.name
            );
        }
    }

    /// No width produces an inverted rectangle — the case a caller hits by
    /// putting a settings list in a narrow column, and the one that draws garbage
    /// rather than failing loudly.
    #[test]
    fn no_width_produces_an_inverted_column() {
        for width in 0..300 {
            let row = Rect::new(5, 0, width, 36);
            let (leading, label, trailing) = columns(row, 8, 30, 40);
            for (name, part) in [
                ("leading", leading),
                ("label", label),
                ("trailing", trailing),
            ] {
                assert!(part.width >= 0, "width {width}: {name} inverted");
                assert_eq!(part.height, row.height, "width {width}: {name}");
            }
        }
    }

    /// Given room, the three columns are in order and do not overlap.
    #[test]
    fn the_columns_are_in_order_when_there_is_room_for_them() {
        let row = Rect::new(5, 0, 300, 36);
        let (leading, label, trailing) = columns(row, 8, 30, 40);
        assert!(
            leading.x >= row.x,
            "the leading column starts inside the row"
        );
        assert!(
            leading.right() <= label.x,
            "the label overlaps the leading column"
        );
        assert!(
            label.right() <= trailing.x,
            "the label overlaps the trailing column"
        );
        assert!(trailing.right() <= row.right());
        assert!(label.width > 0, "the label got nothing");
    }

    /// A row with neither leading nor trailing text gives the whole width to the
    /// label, less the padding.
    #[test]
    fn a_bare_row_gives_the_whole_width_to_its_label() {
        let row = Rect::new(0, 0, 200, 36);
        let (leading, label, trailing) = columns(row, 8, 0, 0);
        assert_eq!(leading.width, 0);
        assert_eq!(trailing.width, 0);
        assert_eq!(label.x, 8);
        assert_eq!(label.right(), 192);
    }

    /// Sizing: the height is rows times a fixed row, and the width is the widest
    /// of everything laid side by side.
    #[test]
    fn the_preferred_size_is_the_rows_and_the_widest_of_each_column() {
        let mut engine = TextEngine::new();
        let list = list();
        assert_eq!(
            list.preferred_height(&theme::DARK),
            theme::DARK.metrics.size_field * 3
        );
        // A taller row metric makes a taller list, which is the whole of the touch
        // story for this widget.
        assert!(
            list.preferred_height(&theme::DARK.with_metrics(denise::theme::Metrics::TOUCH))
                > list.preferred_height(&theme::DARK)
        );

        let style = TextStyle::built_in(16);
        let pad = padding(16);
        let widest_label = ["Nettverk", "Skjerm", "Om"]
            .iter()
            .map(|text| engine.measure_line(style, text))
            .max()
            .expect("three labels");
        let trailing = engine.measure_line(style, "70 %");
        assert_eq!(
            list.preferred_width(&mut engine),
            pad * 2 + widest_label + trailing + pad,
            "no leading column, so no gap for one"
        );
    }

    /// How many rows fit is what a caller needs in order to be honest about a
    /// list that does not scroll.
    #[test]
    fn the_visible_row_count_is_what_actually_fits() {
        let list = list().with_row_height(40);
        assert_eq!(list.visible_rows(&theme::DARK, 200), 5);
        assert_eq!(
            list.visible_rows(&theme::DARK, 199),
            4,
            "a part row does not count"
        );
        assert_eq!(list.visible_rows(&theme::DARK, 0), 0);
        assert_eq!(list.visible_rows(&theme::DARK, -10), 0);
        assert_eq!(list.row_height(&theme::DARK), 40);
    }

    /// A glyph sits in the leading column, so it widens the column to its own
    /// side when the leading texts are narrower — and the icon side follows the
    /// text size, so a scaled-up list scales its glyphs with it.
    #[test]
    fn a_leading_icon_widens_the_leading_column_to_its_side() {
        use crate::widgets::icons;

        let mut engine = TextEngine::new();
        let bare = list();
        assert_eq!(bare.leading_width(&mut engine), 0, "no icons, no column");

        let mut iconed: List<usize> = List::new(
            alloc::vec![
                ListItem::new("button").with_leading_icon(&icons::BUTTON),
                ListItem::new("label"),
            ],
            |index| index,
        );
        assert_eq!(iconed.items()[0].leading_icon(), Some(&icons::BUTTON));
        assert_eq!(iconed.items()[1].leading_icon(), None);
        assert_eq!(
            iconed.leading_width(&mut engine),
            16,
            "the column is the icon's side at the default text size"
        );
        iconed.set_style(TextStyle::built_in(24));
        assert_eq!(iconed.leading_width(&mut engine), 24, "and it scales");
    }

    /// The icon column is part of the preferred width, same as leading text:
    /// a palette sized to its rows must not squeeze the labels to fit glyphs in.
    #[test]
    fn a_leading_icon_is_part_of_the_preferred_width() {
        use crate::widgets::icons;

        let mut engine = TextEngine::new();
        let plain: List<usize> = List::new(["button"], |index| index);
        let iconed: List<usize> = List::new(
            [ListItem::new("button").with_leading_icon(&icons::BUTTON)],
            |index| index,
        );
        assert_eq!(
            iconed.preferred_width(&mut engine),
            plain.preferred_width(&mut engine) + 16 + padding(16),
            "the glyph and its gap"
        );
    }

    /// A string is a row, so the common case does not need a builder.
    #[test]
    fn a_bare_string_is_a_row() {
        let list: List<usize> = List::new(["Auto", "Manuell"], |index| index);
        assert_eq!(list.items().len(), 2);
        assert_eq!(list.items()[1].text(), "Manuell");
        assert!(list.items()[0].is_enabled());
        assert_eq!(list.items()[0].leading(), "");
        assert_eq!(list.items()[0].trailing(), "");
    }

    /// Rows can be disabled after the fact, which is how a settings screen greys
    /// out what the current mode does not offer.
    #[test]
    fn a_row_can_be_disabled_after_the_list_is_built() {
        let mut list = list();
        assert!(Widget::<usize>::focusable(&list));
        list.set_row_enabled(1, false);
        assert!(!list.items()[1].is_enabled());
        assert_eq!(step(list.items(), Some(0), true), Some(2));
        list.set_row_enabled(99, false);
    }
}
