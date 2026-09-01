//! Rows at a depth, with a disclosure triangle and a hierarchy.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role, Theme};
use denise_render::Canvas;
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

/// One row of a tree: a label at a depth, and optionally something before and
/// after it.
///
/// ```
/// # use denise_ui::TreeItem;
/// TreeItem::new("Nettverk");
/// TreeItem::new("Wi-Fi").at_depth(1).with_trailing("på");
/// TreeItem::new("Ikke ferdig").at_depth(1).disabled();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeItem {
    text: String,
    leading: String,
    trailing: String,
    depth: u16,
    open: bool,
    enabled: bool,
}

impl TreeItem {
    /// A row at the top level, open, enabled.
    ///
    /// Open by default because a tree that arrives entirely shut shows one row
    /// per top-level branch and nothing of what it is for.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            leading: String::new(),
            trailing: String::new(),
            depth: 0,
            open: true,
            enabled: true,
        }
    }

    /// How deep this row sits. `0` is the top level.
    ///
    /// The hierarchy **is** this number: a row is a child of the nearest row
    /// above it with a smaller depth. See the note on [`Tree`] for why the rows
    /// are flat.
    pub fn at_depth(mut self, depth: u16) -> Self {
        self.depth = depth;
        self
    }

    /// Starts this row shut, so what is under it is not drawn until it is opened.
    pub fn shut(mut self) -> Self {
        self.open = false;
        self
    }

    /// Puts a short run of text in a column before the label.
    pub fn with_leading(mut self, leading: impl Into<String>) -> Self {
        self.leading = leading.into();
        self
    }

    /// Puts text at the trailing edge of the row: a value, a unit, a state.
    pub fn with_trailing(mut self, trailing: impl Into<String>) -> Self {
        self.trailing = trailing.into();
        self
    }

    /// Makes the row unselectable: skipped by the keyboard, inert to the mouse.
    ///
    /// A disabled row that has children still opens and shuts: what is under it
    /// may well be reachable even when it is not.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The row's label.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text before the label.
    pub fn leading(&self) -> &str {
        &self.leading
    }

    /// The text at the trailing edge.
    pub fn trailing(&self) -> &str {
        &self.trailing
    }

    /// How deep this row sits.
    #[inline]
    pub const fn depth(&self) -> u16 {
        self.depth
    }

    /// Whether what is under this row is drawn.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Whether this row can be selected.
    #[inline]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Replaces the label.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Opens or shuts this row.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Enables or disables this row.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn trailing_width(&self, engine: &mut TextEngine, style: TextStyle) -> i32 {
        if self.trailing.is_empty() {
            0
        } else {
            engine.measure_line(style, &self.trailing)
        }
    }
}

impl From<&str> for TreeItem {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for TreeItem {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

/// A hierarchy: rows at a depth, opened and shut by a disclosure triangle.
///
/// What [`List`](super::List) is to a column of choices, this is to a settings
/// hierarchy, a menu of menus, or a file list on a panel.
///
/// ```
/// # use denise_ui::{Tree, TreeItem};
/// enum Message { Pick(usize), Open(usize), Fold(usize) }
/// Tree::new(
///     [
///         TreeItem::new("Nettverk"),
///         TreeItem::new("Wi-Fi").at_depth(1),
///         TreeItem::new("Ethernet").at_depth(1),
///         TreeItem::new("Skjerm"),
///     ],
///     Message::Pick,
/// )
/// .on_activate(Message::Open)
/// .on_toggle(Message::Fold);
/// ```
///
/// # Flat rows with a depth, not a nest
///
/// The rows are a slice and each carries how deep it is. A row's **parent is the
/// nearest row above it with a smaller depth**, and its children are the run of
/// deeper rows immediately below it. That is the whole data structure.
///
/// A nested type would be the obvious other choice and is worse here in three
/// ways. It needs an allocation per branch in a crate that runs on a panel with
/// no allocator to spare; it makes "which row is at this y" a walk instead of a
/// count; and it cannot be handed to a widget from a form file without the file
/// growing real nesting, which `.dform` deliberately does not have for content.
///
/// Nothing says `has_children` — it is **derived**, because in this
/// representation it is not an independent fact: a row has children exactly when
/// the row after it is deeper. A field for it could disagree with the depths, and
/// then one of the two would be a lie.
///
/// # Scrolling belongs to the tree of nodes, and this widget cooperates with it
///
/// The same arrangement [`List`](super::List) documents at length: this draws the
/// rows that fit and stops, and the scrolling version is this inside a node
/// marked [`Ui::set_scrollable`](crate::Ui::set_scrollable), sized with
/// [`preferred_height`](Tree::preferred_height). A keyboard selection below the
/// fold reveals itself and the viewport follows.
///
/// [`preferred_height`](Tree::preferred_height) counts the rows that are
/// **shown**, so opening a branch makes the widget want to be taller — which is
/// the caller's cue to resize it and let the viewport scroll further.
///
/// # Keyboard
///
/// One node, so one tab stop. [`List`](super::List)'s keyboard, plus the two the
/// hierarchy adds:
///
/// | | |
/// |---|---|
/// | Up, Down | The previous and next **shown** row. Does not wrap. |
/// | Right | Opens a shut row; on an open one, moves to its first child. |
/// | Left | Shuts an open row; on a shut one or a leaf, moves to its parent. |
/// | Home, End | The first and last shown row. |
/// | Enter | Activates. |
///
/// Right and Left are the convention every tree control has used since the
/// Windows 95 explorer, and the reason they do two things each is that the
/// obvious one-thing version leaves a person on a shut row pressing Left with
/// nothing happening.
///
/// # Three messages, because there are three things a person does
///
/// Selecting a row, acting on it, and opening it are different, so they are
/// separate: [`new`](Tree::new) takes the selection,
/// [`on_activate`](Tree::on_activate) takes Enter and the double-click, and
/// [`on_toggle`](Tree::on_toggle) takes the triangle. Each reports the row's
/// index **into the rows as given** — not its position among the shown ones,
/// which changes whenever a branch above it folds, and not a path, which a
/// `fn(usize) -> M` could not carry.
#[derive(Clone, Debug)]
pub struct Tree<M> {
    items: Vec<TreeItem>,
    selected: Option<usize>,
    hovered: Option<usize>,
    row_height: Option<i32>,
    indent: i32,
    selection: Option<fn(usize) -> M>,
    activation: Option<fn(usize) -> M>,
    toggle: Option<fn(usize) -> M>,
    single_click: bool,
    clicks: ClickPair,
    role: Role,
    style: TextStyle,
}

/// How far one level is indented from the one above, when nobody says.
const INDENT: i32 = 14;

impl<M> Tree<M> {
    /// A tree with nothing selected, reporting selection through `message`.
    pub fn new(
        items: impl IntoIterator<Item = impl Into<TreeItem>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            selection: Some(message),
            ..Self::bare()
        }
    }

    /// A tree that emits nothing, for a hierarchy the application reads rather
    /// than reacts to.
    pub fn inert(items: impl IntoIterator<Item = impl Into<TreeItem>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            ..Self::bare()
        }
    }

    fn bare() -> Self {
        Self {
            items: Vec::new(),
            selected: None,
            hovered: None,
            row_height: None,
            indent: INDENT,
            selection: None,
            activation: None,
            toggle: None,
            single_click: false,
            clicks: ClickPair::default(),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the message sent when a row is activated by Enter or a double-click.
    pub fn on_activate(mut self, message: fn(usize) -> M) -> Self {
        self.activation = Some(message);
        self
    }

    /// Sets the message sent when a row is opened or shut.
    ///
    /// The row's own `open` has already changed by the time this is emitted: the
    /// widget owns what it draws, and an application that wants to veto a fold
    /// wants a different widget.
    pub fn on_toggle(mut self, message: fn(usize) -> M) -> Self {
        self.toggle = Some(message);
        self
    }

    /// Makes a single click or tap activate as well as select.
    ///
    /// The touch-panel answer; see [`List::activate_on_click`](super::List::activate_on_click).
    pub fn activate_on_click(mut self) -> Self {
        self.single_click = true;
        self
    }

    /// Sets the initially selected row, by its index into the rows as given.
    pub fn with_selected(mut self, index: Option<usize>) -> Self {
        self.set_selected(index);
        self
    }

    /// Sets the height of every row, overriding the theme's field height.
    pub fn with_row_height(mut self, height: i32) -> Self {
        self.row_height = Some(height.max(1));
        self
    }

    /// Sets how far one level is indented from the one above.
    pub fn with_indent(mut self, indent: i32) -> Self {
        self.indent = indent.max(0);
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

    /// The selected row, by its index into the rows as given.
    #[inline]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The selected row's item, if any.
    pub fn selected_item(&self) -> Option<&TreeItem> {
        self.items.get(self.selected?)
    }

    /// Selects a row. Out of range, disabled, or hidden under a shut branch
    /// selects nothing.
    ///
    /// Hidden is refused rather than silently opening the branch: a selection
    /// nobody can see is one the keyboard would then move from a place the
    /// person is not looking at.
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| {
            self.items.get(*index).is_some_and(TreeItem::is_enabled) && self.is_shown(*index)
        });
    }

    /// The rows, as given.
    pub fn items(&self) -> &[TreeItem] {
        &self.items
    }

    /// Replaces the rows.
    pub fn set_items(&mut self, items: impl IntoIterator<Item = impl Into<TreeItem>>) {
        self.items = items.into_iter().map(Into::into).collect();
        let selected = self.selected;
        self.set_selected(selected);
        self.hovered = None;
        // A remembered click points at a row that may now be something else.
        self.clicks.forget();
    }

    /// Opens or shuts one row, without reporting it.
    ///
    /// For an application driving the tree rather than answering it — restoring
    /// what was open when a screen was last shown, say.
    pub fn set_open(&mut self, index: usize, open: bool) {
        if let Some(item) = self.items.get_mut(index) {
            item.open = open;
        }
        let selected = self.selected;
        self.set_selected(selected);
    }

    /// Opens or shuts every row that has children.
    pub fn set_all_open(&mut self, open: bool) {
        for item in &mut self.items {
            item.open = open;
        }
        let selected = self.selected;
        self.set_selected(selected);
    }

    /// Enables or disables one row.
    pub fn set_row_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(item) = self.items.get_mut(index) {
            item.set_enabled(enabled);
        }
        let selected = self.selected;
        self.set_selected(selected);
    }

    /// Replaces the colour role of the selected row.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the rows' font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Whether the row at `index` has any children.
    ///
    /// Derived from the depths rather than stored; see the note on the type.
    pub fn has_children(&self, index: usize) -> bool {
        has_children(&self.items, index)
    }

    /// Whether the row at `index` is drawn — every branch above it being open.
    pub fn is_shown(&self, index: usize) -> bool {
        Shown::new(&self.items).any(|(shown, _)| shown == index)
    }

    /// How many rows are drawn.
    pub fn shown_rows(&self) -> usize {
        Shown::new(&self.items).count()
    }

    /// Height every row is drawn at.
    pub fn row_height(&self, theme: &Theme) -> i32 {
        self.row_height.unwrap_or(theme.metrics.size_field).max(1)
    }

    /// How many rows fit in `height`.
    pub fn visible_rows(&self, theme: &Theme, height: i32) -> usize {
        if height <= 0 {
            return 0;
        }
        (height / self.row_height(theme)) as usize
    }

    /// Height this tree needs to show every row that is **currently** shown.
    ///
    /// Changes as branches open and shut, which is the point: a caller sizing a
    /// tree inside a scrolling viewport asks again after a toggle.
    pub fn preferred_height(&self, theme: &Theme) -> i32 {
        let rows = self.shown_rows().max(1) as i64;
        (i64::from(self.row_height(theme)) * rows).min(i64::from(i32::MAX)) as i32
    }

    /// Width the widest shown row needs, indentation and columns included.
    pub fn preferred_width(&self, engine: &mut TextEngine) -> i32 {
        let pad = padding(self.style.size_px);
        let gutter = self.gutter();
        let mut widest = 0;
        let mut trailing = 0;
        for (index, item) in Shown::new(&self.items) {
            let _ = index;
            let text = engine.measure_line(self.style, &item.text);
            let leading = if item.leading.is_empty() {
                0
            } else {
                engine.measure_line(self.style, &item.leading) + pad
            };
            widest = widest.max(self.indent_of(item) + gutter + leading + text);
            trailing = trailing.max(item.trailing_width(engine, self.style));
        }
        let gap = if trailing > 0 { pad } else { 0 };
        pad * 2 + widest + trailing + gap
    }

    /// The column the disclosure triangle stands in, before a row's content.
    ///
    /// Always reserved, whether or not the row has children, so that siblings at
    /// one depth line up whatever they hold.
    fn gutter(&self) -> i32 {
        (i32::from(self.style.size_px) * 3 / 4).max(8)
    }

    /// How far this row's content is pushed in.
    fn indent_of(&self, item: &TreeItem) -> i32 {
        self.indent.saturating_mul(i32::from(item.depth))
    }

    /// The triangle's box inside a row, and the row's content after it.
    fn parts(&self, row: Rect, item: &TreeItem) -> (Rect, Rect) {
        let pad = padding(self.style.size_px);
        let start = row.x + pad;
        let right = (row.right() - pad).max(start);
        // Clamped into the row before anything is measured from it. A depth the
        // rectangle has no room for collapses to nothing at the right edge —
        // the text is clipped rather than drawn outside the widget, which is
        // what a row indented past its own width has to mean.
        let left = start
            .saturating_add(self.indent_of(item))
            .clamp(start, right);
        let triangle = Rect::from_edges(
            left,
            row.y,
            left.saturating_add(self.gutter()).min(right),
            row.bottom(),
        );
        let content = Rect::from_edges(triangle.right(), row.y, right, row.bottom());
        (triangle, content)
    }

    /// Which shown row is at `point`, and whether the point is on its triangle.
    ///
    /// The y is arithmetic — rows are a fixed height — and mapping that to a row
    /// is one walk of the depths, which is what a flat representation costs and
    /// is cheaper than the nest it replaces.
    fn hit(&self, bounds: Rect, row_height: i32, point: Point) -> Option<(usize, bool)> {
        if !bounds.contains(point) {
            return None;
        }
        let nth = (i64::from(point.y - bounds.y) / i64::from(row_height.max(1))) as usize;
        let (index, item) = Shown::new(&self.items).nth(nth)?;
        let row = row_rect(bounds, row_height, nth);
        let (triangle, _) = self.parts(row, item);
        let on_triangle = has_children(&self.items, index) && triangle.contains(point);
        Some((index, on_triangle))
    }

    /// Moves the selection, reporting it. `None` means the end of the tree.
    fn select(&mut self, target: Option<usize>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some(target) = target else {
            return Handled::Yes;
        };
        if self.selected == Some(target) {
            return Handled::Yes;
        }
        self.selected = Some(target);
        if let Some(nth) = Shown::new(&self.items).position(|(index, _)| index == target) {
            ctx.reveal(row_rect(ctx.bounds, self.row_height(ctx.theme), nth));
        }
        if let Some(message) = self.selection {
            ctx.emit(message(target));
        }
        Handled::Yes
    }

    /// Opens or shuts a row, reporting it.
    fn toggle(&mut self, index: usize, ctx: &mut EventCtx<'_, M>) -> Handled {
        if !has_children(&self.items, index) {
            return Handled::No;
        }
        let Some(item) = self.items.get_mut(index) else {
            return Handled::No;
        };
        item.open = !item.open;
        // Shutting a branch can hide the selection, which would otherwise leave
        // the keyboard walking from somewhere nobody can see.
        let selected = self.selected;
        self.set_selected(selected);
        if self.selected.is_none() && selected.is_some() {
            self.selected = Some(index);
        }
        if let Some(message) = self.toggle {
            ctx.emit(message(index));
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

    /// Right: open a shut row, or step into an open one.
    fn go_in(&mut self, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some(index) = self.selected else {
            let target = step(&self.items, None, true);
            return self.select(target, ctx);
        };
        if !has_children(&self.items, index) {
            return Handled::Yes;
        }
        if self.items.get(index).is_some_and(TreeItem::is_open) {
            let child = step(&self.items, Some(index), true);
            return self.select(child, ctx);
        }
        self.toggle(index, ctx)
    }

    /// Left: shut an open row, or step out to its parent.
    fn go_out(&mut self, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some(index) = self.selected else {
            let target = step(&self.items, None, false);
            return self.select(target, ctx);
        };
        let open = self.items.get(index).is_some_and(TreeItem::is_open);
        if has_children(&self.items, index) && open {
            return self.toggle(index, ctx);
        }
        match parent_of(&self.items, index) {
            // A parent that cannot be selected is still where Left goes; the
            // step from there carries on past it.
            Some(parent) if self.items[parent].enabled => self.select(Some(parent), ctx),
            _ => Handled::Yes,
        }
    }
}

/// The rows that are drawn, in order, as `(index, item)`.
///
/// A single forward pass: everything below a shut row is skipped until the depth
/// comes back up to it. No allocation, which is why every part of this widget
/// that needs "the rows in order" uses it rather than collecting a `Vec`.
struct Shown<'a> {
    items: &'a [TreeItem],
    at: usize,
    /// The depth of the shut row whose subtree is being skipped.
    shut_at: Option<u16>,
}

impl<'a> Shown<'a> {
    fn new(items: &'a [TreeItem]) -> Self {
        Self {
            items,
            at: 0,
            shut_at: None,
        }
    }
}

impl<'a> Iterator for Shown<'a> {
    type Item = (usize, &'a TreeItem);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let index = self.at;
            let item = self.items.get(index)?;
            self.at += 1;
            if let Some(depth) = self.shut_at {
                if item.depth > depth {
                    continue;
                }
                self.shut_at = None;
            }
            if !item.open && has_children(self.items, index) {
                self.shut_at = Some(item.depth);
            }
            return Some((index, item));
        }
    }
}

/// Whether the row at `index` has children: the next row is deeper.
fn has_children(items: &[TreeItem], index: usize) -> bool {
    let Some(item) = items.get(index) else {
        return false;
    };
    items
        .get(index + 1)
        .is_some_and(|next| next.depth > item.depth)
}

/// The nearest row above `index` with a smaller depth.
fn parent_of(items: &[TreeItem], index: usize) -> Option<usize> {
    let depth = items.get(index)?.depth;
    if depth == 0 {
        return None;
    }
    items[..index].iter().rposition(|item| item.depth < depth)
}

/// The first shown row that can be selected.
fn first_enabled(items: &[TreeItem]) -> Option<usize> {
    Shown::new(items)
        .find(|(_, item)| item.enabled)
        .map(|(index, _)| index)
}

/// The last shown row that can be selected.
fn last_enabled(items: &[TreeItem]) -> Option<usize> {
    Shown::new(items)
        .filter(|(_, item)| item.enabled)
        .map(|(index, _)| index)
        .last()
}

/// The next selectable **shown** row in the direction of travel.
///
/// `None` means *stay where you are*: like [`List`](super::List) and unlike
/// [`RadioGroup`](super::RadioGroup), this does not wrap.
fn step(items: &[TreeItem], from: Option<usize>, forward: bool) -> Option<usize> {
    let shown: Option<usize> = match from {
        None => {
            return if forward {
                first_enabled(items)
            } else {
                last_enabled(items)
            };
        }
        Some(from) => Shown::new(items).position(|(index, _)| index == from),
    };
    let shown = shown?;
    let mut walk = Shown::new(items)
        .enumerate()
        .filter(|(_, (_, item))| item.enabled)
        .map(|(nth, (index, _))| (nth, index));
    if forward {
        walk.find(|(nth, _)| *nth > shown).map(|(_, index)| index)
    } else {
        walk.take_while(|(nth, _)| *nth < shown)
            .map(|(_, index)| index)
            .last()
    }
}

/// Space between a row's content and its edge, on one side.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let half = size_px as i32 / 2;
    if half < 4 { 4 } else { half }
}

/// Where the n-th **shown** row sits.
fn row_rect(bounds: Rect, row_height: i32, nth: usize) -> Rect {
    let height = row_height.max(1);
    let nth = nth.min(i32::MAX as usize) as i64;
    let ceiling = i64::from(i32::MAX - height);
    let y = (i64::from(bounds.y) + i64::from(height) * nth).min(ceiling) as i32;
    Rect::new(bounds.x, y, bounds.width, height)
}

/// Draws a disclosure triangle, pointing down when open and along when shut.
///
/// Filled from horizontal spans rather than drawn as a glyph: the built-in font
/// covers ASCII and Latin-1, so `▾` would come out as the missing-character box
/// on a panel with no font file — which is the configuration this toolkit is for.
fn disclosure(canvas: &mut Canvas<'_>, box_of: Rect, open: bool, color: denise::Color) {
    // Sized off the row so it scales with the text, and odd so it has a point.
    let size = (box_of.height / 3).clamp(3, 9) | 1;
    let cx = box_of.x + box_of.width / 2;
    let cy = box_of.y + box_of.height / 2;
    if open {
        // Pointing down: a wide span at the top narrowing to a point.
        for step in 0..=size {
            let half = size - step;
            canvas.fill_rect(
                Rect::new(cx - half, cy - size / 2 + step, half * 2 + 1, 1),
                color,
            );
        }
    } else {
        // Pointing along: a tall span at the left narrowing to a point.
        for step in 0..=size {
            let half = size - step;
            canvas.fill_rect(
                Rect::new(cx - size / 2 + step, cy - half, 1, half * 2 + 1),
                color,
            );
        }
    }
}

impl<M: 'static> Widget<M> for Tree<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }

    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        // The height follows what is open, so this answer changes as branches
        // fold — which is the caller's cue to ask again.
        Measured::both(
            self.preferred_width(ctx.text),
            self.preferred_height(ctx.theme),
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() || self.items.is_empty() {
            return;
        }
        let (backdrop, _) = row_colors(ctx.theme, ctx.state, self.role, RowKind::Resting, true);
        canvas.fill_rect(bounds, backdrop);

        let row_height = self.row_height(ctx.theme);
        let pad = padding(self.style.size_px);
        let radius = ctx.theme.radius(Radius::Field);
        let hovered = hovered_row(ctx.state, self.hovered);

        for (nth, (index, item)) in Shown::new(&self.items).enumerate() {
            let row = row_rect(bounds, row_height, nth);
            if row.y >= bounds.bottom() {
                // Past the bottom edge; this widget does not scroll.
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

            let (triangle, rest) = self.parts(row, item);
            if has_children(&self.items, index) && !triangle.is_empty() {
                disclosure(canvas, triangle, item.open, content);
            }

            let trailing_width = item.trailing_width(ctx.text, self.style);
            let leading_width = if item.leading.is_empty() {
                0
            } else {
                ctx.text.measure_line(self.style, &item.leading)
            };
            let (leading, label, trailing) = columns(rest, pad, leading_width, trailing_width);
            for (box_of, text, align) in [
                (leading, &item.leading, Align::Start),
                (label, &item.text, Align::Start),
                (trailing, &item.trailing, Align::End),
            ] {
                if text.is_empty() || box_of.is_empty() {
                    continue;
                }
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
                let row = self
                    .hit(ctx.bounds, row_height, *position)
                    .map(|(index, _)| index)
                    .filter(|index| self.items[*index].enabled);
                if row == self.hovered {
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
                let Some((index, on_triangle)) = self.hit(ctx.bounds, row_height, *position) else {
                    return Handled::No;
                };
                // The triangle is its own target: pressing it opens the branch
                // and leaves the selection where it was, which is what lets
                // somebody look inside a branch without losing their place.
                if on_triangle {
                    return self.toggle(index, ctx);
                }
                if !self.items[index].enabled {
                    return Handled::No;
                }
                let intent = self.clicks.classify(index, ctx.now_ms, self.single_click);
                let handled = self.select(Some(index), ctx);
                if intent == Intent::Activate {
                    self.activate(index, ctx);
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
                KeyCode::ArrowRight => self.go_in(ctx),
                KeyCode::ArrowLeft => self.go_out(ctx),
                KeyCode::Home => {
                    let target = first_enabled(&self.items);
                    self.select(target, ctx)
                }
                KeyCode::End => {
                    let target = last_enabled(&self.items);
                    self.select(target, ctx)
                }
                KeyCode::Enter | KeyCode::NumpadEnter => match self.selected {
                    Some(row) if self.items.get(row).is_some_and(TreeItem::is_enabled) => {
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

    /// A tree with no row anybody can choose is not a tab stop.
    fn focusable(&self) -> bool {
        self.items.iter().any(TreeItem::is_enabled)
    }
}

impl<M> Describe for Tree<M> {
    const KIND: &'static str = "tree";
    const DOC: &'static str = "A hierarchy of rows that open and shut, indented by depth.";
    const GROUP: Group = Group::Data;
    const ICON: &'static denise_render::icon::Icon = &super::icons::TREE;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "item",
            PropertyKind::List,
            "The rows, as `item` child nodes, each at its own `depth`. Real data, like a list's.",
        ),
        Property::new(
            "selected",
            PropertyKind::Int { min: 0, max: 9999 },
            "Which row is selected, by its position in the file.",
        ),
        Property::new(
            "on-select",
            PropertyKind::Message(Payload::Index),
            "Sent with the row when the selection moves.",
        ),
        Property::new(
            "on-activate",
            PropertyKind::Message(Payload::Index),
            "Sent with the row on Enter or a double-click.",
        ),
        Property::new(
            "on-toggle",
            PropertyKind::Message(Payload::Index),
            "Sent with the row when a branch is opened or shut.",
        ),
        Property::new(
            "activate-on-click",
            PropertyKind::Bool,
            "Whether one tap both selects and activates. For a touch panel.",
        ),
        Property::new(
            "row-height",
            PropertyKind::Int { min: 16, max: 200 },
            "Height of every row in logical pixels, overriding the theme's field height.",
        )
        .in_pixels(),
        Property::new(
            "indent",
            PropertyKind::Int { min: 0, max: 100 },
            "How far one level is indented from the one above, in logical pixels.",
        )
        .in_pixels(),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "The colour of the selected row.",
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
            "selected" => Value::Int(i32::try_from(self.selected?).ok()?),
            "activate-on-click" => Value::Bool(self.single_click),
            "row-height" => Value::Int(self.row_height?),
            "indent" => Value::Int(self.indent),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, so a row that is hidden or disabled selects
            // nothing here exactly as it does everywhere else.
            "selected" => self.set_selected(Some(value.as_index()?)),
            // Built from the child nodes; an inspector edits them
            // where they live. See `PropertyKind::List`.
            "on-select" | "on-activate" | "on-toggle" | "item" => return Err(Mismatch::Supplied),
            "activate-on-click" => self.single_click = value.as_bool()?,
            "row-height" => self.row_height = Some(value.as_int()?.max(1)),
            "indent" => self.indent = value.as_int()?.max(0),
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

    /// Nettverk
    ///   Wi-Fi
    ///     Hjemme
    ///   Ethernet
    /// Skjerm
    ///   Lysstyrke
    fn items() -> Vec<TreeItem> {
        alloc::vec![
            TreeItem::new("Nettverk"),
            TreeItem::new("Wi-Fi").at_depth(1),
            TreeItem::new("Hjemme").at_depth(2),
            TreeItem::new("Ethernet").at_depth(1),
            TreeItem::new("Skjerm"),
            TreeItem::new("Lysstyrke").at_depth(1),
        ]
    }

    fn tree() -> Tree<usize> {
        Tree::new(items(), |row| row)
    }

    /// The rows that would be drawn, by index.
    fn shown(tree: &Tree<usize>) -> Vec<usize> {
        Shown::new(&tree.items).map(|(index, _)| index).collect()
    }

    #[test]
    fn the_hierarchy_is_the_depths_and_nothing_else() {
        let items = items();
        // A row has children exactly when the row after it is deeper. Nothing
        // stores this, so nothing can contradict it.
        assert!(has_children(&items, 0), "Nettverk holds Wi-Fi");
        assert!(has_children(&items, 1), "Wi-Fi holds Hjemme");
        assert!(!has_children(&items, 2), "Hjemme holds nothing");
        assert!(!has_children(&items, 3), "Ethernet holds nothing");
        assert!(has_children(&items, 4), "Skjerm holds Lysstyrke");
        assert!(!has_children(&items, 5), "the last row holds nothing");

        // A parent is the nearest row above with a smaller depth.
        assert_eq!(parent_of(&items, 0), None);
        assert_eq!(parent_of(&items, 1), Some(0));
        assert_eq!(parent_of(&items, 2), Some(1));
        assert_eq!(parent_of(&items, 3), Some(0), "past its deeper sibling");
        assert_eq!(parent_of(&items, 5), Some(4));
    }

    #[test]
    fn shutting_a_branch_hides_everything_under_it_however_deep() {
        let mut tree = tree();
        assert_eq!(shown(&tree), alloc::vec![0, 1, 2, 3, 4, 5]);

        // Shutting Wi-Fi hides only its own child.
        tree.set_open(1, false);
        assert_eq!(shown(&tree), alloc::vec![0, 1, 3, 4, 5]);

        // Shutting Nettverk hides Wi-Fi, its child, and Ethernet — the whole
        // subtree, not one level of it.
        tree.set_open(0, false);
        assert_eq!(shown(&tree), alloc::vec![0, 4, 5]);

        // And opening it again leaves Wi-Fi as it was found: shutting a branch
        // remembers what was inside rather than flattening it.
        tree.set_open(0, true);
        assert_eq!(shown(&tree), alloc::vec![0, 1, 3, 4, 5]);
    }

    #[test]
    fn a_row_with_no_children_is_never_shut_around() {
        // `open` on a leaf is meaningless and must not swallow its siblings.
        let mut items = items();
        items[2].open = false;
        let tree = Tree::new(items, |row| row);
        assert_eq!(shown(&tree), alloc::vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_keyboard_walks_the_rows_that_are_shown() {
        let mut tree = tree();
        tree.set_open(1, false);

        // Down from Wi-Fi skips its hidden child and lands on Ethernet.
        assert_eq!(step(&tree.items, Some(1), true), Some(3));
        // And back again.
        assert_eq!(step(&tree.items, Some(3), false), Some(1));

        // The ends do not wrap.
        assert_eq!(step(&tree.items, Some(5), true), None);
        assert_eq!(step(&tree.items, Some(0), false), None);

        // From nothing selected, the near end in the direction of travel.
        assert_eq!(step(&tree.items, None, true), Some(0));
        assert_eq!(step(&tree.items, None, false), Some(5));
    }

    #[test]
    fn disabled_rows_are_stepped_over_and_hidden_ones_are_not_reachable() {
        let mut items = items();
        items[1].enabled = false;
        items[3].enabled = false;
        let tree = Tree::new(items, |row| row);

        // Both children of Nettverk are disabled, so Down from it reaches its
        // grandchild rather than stopping.
        assert_eq!(step(&tree.items, Some(0), true), Some(2));
        assert_eq!(step(&tree.items, Some(2), true), Some(4));

        assert_eq!(first_enabled(&tree.items), Some(0));
        assert_eq!(last_enabled(&tree.items), Some(5));
    }

    #[test]
    fn a_selection_under_a_branch_that_shuts_moves_to_the_branch() {
        let mut tree = tree();
        tree.set_selected(Some(2));
        assert_eq!(tree.selected(), Some(2));

        // Shutting Wi-Fi through the setter leaves nothing selected rather than
        // a selection nobody can see.
        tree.set_open(1, false);
        assert_eq!(tree.selected(), None, "a hidden row stayed selected");
    }

    #[test]
    fn a_hidden_or_disabled_row_cannot_be_selected() {
        let mut tree = tree();
        tree.set_open(0, false);

        tree.set_selected(Some(2));
        assert_eq!(
            tree.selected(),
            None,
            "selected something under a shut branch"
        );

        tree.set_selected(Some(9));
        assert_eq!(tree.selected(), None, "selected a row that is not there");

        tree.set_row_enabled(4, false);
        tree.set_selected(Some(4));
        assert_eq!(tree.selected(), None, "selected a disabled row");
    }

    #[test]
    fn rows_are_a_fixed_height_stacked_from_the_top() {
        let bounds = Rect::new(10, 20, 300, 400);
        let mut previous = bounds.y;
        for nth in 0..6 {
            let row = row_rect(bounds, 36, nth);
            assert_eq!(row.y, previous);
            assert_eq!(row.height, 36);
            assert_eq!(row.x, bounds.x);
            assert_eq!(row.right(), bounds.right());
            previous = row.bottom();
        }
    }

    #[test]
    fn an_absurdly_deep_tree_neither_overflows_nor_panics() {
        // Arithmetic on somebody else's numbers: a panic inside a paint loop on
        // a kiosk is a black screen.
        let bounds = Rect::new(0, 0, 300, 400);
        let row = row_rect(bounds, 36, usize::MAX / 2);
        assert!(row.height > 0);
        assert!(row.bottom() >= row.y, "the rectangle inverted");

        let deep = Tree::<usize>::inert(alloc::vec![
            TreeItem::new("a").at_depth(0),
            TreeItem::new("b").at_depth(u16::MAX),
        ]);
        assert!(
            deep.indent_of(&deep.items[1]) > 0,
            "the indent saturated wrong"
        );
        let row = row_rect(bounds, 36, 1);
        let (triangle, content) = deep.parts(row, &deep.items[1]);
        assert!(
            triangle.width >= 0 && content.width >= 0,
            "a column inverted"
        );
        assert!(content.right() <= row.right(), "a column left the row");
    }

    #[test]
    fn the_triangle_is_its_own_target_and_the_rest_of_the_row_is_not() {
        let tree = tree();
        let bounds = Rect::new(0, 0, 300, 240);
        let row = row_rect(bounds, 40, 0);
        let (triangle, _) = tree.parts(row, &tree.items[0]);

        let on_triangle = Point::new(triangle.x + triangle.width / 2, row.y + row.height / 2);
        assert_eq!(tree.hit(bounds, 40, on_triangle), Some((0, true)));

        let on_label = Point::new(row.right() - 10, row.y + row.height / 2);
        assert_eq!(tree.hit(bounds, 40, on_label), Some((0, false)));

        // A leaf has no triangle, so the same place on its row is the row.
        let leaf_row = row_rect(bounds, 40, 2);
        let (leaf_triangle, _) = tree.parts(leaf_row, &tree.items[2]);
        let on_nothing = Point::new(
            leaf_triangle.x + leaf_triangle.width / 2,
            leaf_row.y + leaf_row.height / 2,
        );
        assert_eq!(tree.hit(bounds, 40, on_nothing), Some((2, false)));
    }

    #[test]
    fn a_deeper_rows_triangle_is_indented_with_it() {
        let tree = tree();
        let bounds = Rect::new(0, 0, 300, 240);
        let (top, _) = tree.parts(row_rect(bounds, 40, 0), &tree.items[0]);
        let (nested, _) = tree.parts(row_rect(bounds, 40, 1), &tree.items[1]);
        assert_eq!(nested.x - top.x, INDENT, "one level is one indent");

        let (deeper, _) = tree.parts(row_rect(bounds, 40, 2), &tree.items[2]);
        assert_eq!(deeper.x - top.x, INDENT * 2);
    }

    #[test]
    fn a_point_below_the_last_shown_row_is_not_the_last_row() {
        let mut tree = tree();
        tree.set_open(0, false);
        tree.set_open(4, false);
        let bounds = Rect::new(0, 0, 300, 400);
        // Two rows shown, so the third row's worth of space is nobody's.
        assert_eq!(tree.shown_rows(), 2);
        let below = Point::new(50, bounds.y + 40 * 2 + 5);
        assert_eq!(tree.hit(bounds, 40, below), None);
    }

    #[test]
    fn the_height_it_asks_for_follows_what_is_open() {
        let theme = &denise::theme::DARK;
        let mut tree = tree();
        tree = tree.with_row_height(20);
        assert_eq!(tree.preferred_height(theme), 120, "six rows");

        tree.set_open(0, false);
        assert_eq!(tree.preferred_height(theme), 60, "three rows");

        // Never zero: a widget with no height is a widget nobody can click on
        // to open again.
        tree.set_all_open(false);
        assert!(tree.preferred_height(theme) > 0);
    }

    #[test]
    fn an_empty_tree_is_inert_rather_than_broken() {
        let tree = Tree::<usize>::inert(Vec::<TreeItem>::new());
        assert_eq!(tree.shown_rows(), 0);
        assert_eq!(tree.selected(), None);
        assert!(
            !tree.focusable(),
            "an empty tree is a tab stop with nothing in it"
        );
        assert_eq!(
            tree.hit(Rect::new(0, 0, 100, 100), 20, Point::new(5, 5)),
            None
        );
        assert!(tree.preferred_height(&denise::theme::DARK) > 0);
    }

    #[test]
    fn replacing_the_rows_keeps_a_selection_that_still_makes_sense() {
        let mut tree = tree();
        tree.set_selected(Some(3));

        tree.set_items(items());
        assert_eq!(tree.selected(), Some(3), "the same row is still there");

        // And drops one that is not.
        tree.set_items(alloc::vec![TreeItem::new("only")]);
        assert_eq!(tree.selected(), None);
    }
}
