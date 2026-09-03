//! Columns of cells under a pinned header.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use denise::Pen;
use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role, Theme};
use denise_text::TextStyle;

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{
    Align, ClickPair, Intent, RowKind, draw_aligned, focus_ring, hovered_row, interactive_pair,
    row_colors,
};

/// The scroll thumb's width, and the gap between it and the rows.
const THUMB: i32 = 3;

/// One column: a title, a width, and how its cells align.
///
/// ```
/// # use denise_ui::widgets::Column;
/// Column::new("Navn", 190);
/// Column::flex("Rolle");
/// Column::new("Alder", 60).align_end();  // numbers align on their ends
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    title: String,
    /// `None` shares whatever the fixed columns leave.
    width: Option<i32>,
    align: Align,
}

impl Column {
    /// A column of `width` pixels.
    pub fn new(title: impl Into<String>, width: i32) -> Self {
        Self {
            title: title.into(),
            width: Some(width.max(0)),
            align: Align::Start,
        }
    }

    /// A column that takes an equal share of whatever the fixed ones leave.
    pub fn flex(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: None,
            align: Align::Start,
        }
    }

    /// Aligns this column's cells — and its title — at the trailing edge.
    /// For numbers, which compare by their ends.
    pub fn align_end(mut self) -> Self {
        self.align = Align::End;
        self
    }

    /// Centres this column's cells and title.
    pub fn align_center(mut self) -> Self {
        self.align = Align::Center;
        self
    }

    /// The title drawn in the header.
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }
}

impl From<&str> for Column {
    /// A bare string is a flex column, so the common case needs no builder.
    fn from(title: &str) -> Self {
        Self::flex(title)
    }
}

/// Rows of cells under a pinned header, at most one row selected.
///
/// ```
/// # use denise_ui::widgets::{Column, Table};
/// # struct Record { name: String, role: String }
/// # let records: Vec<Record> = Vec::new();
/// enum Message { Pick(usize), Open(usize) }
/// Table::new([Column::new("Navn", 190), Column::flex("Rolle")], Message::Pick)
///     .with_rows(records.iter().map(|r| [r.name.clone(), r.role.clone()]))
///     .on_activate(Message::Open);
/// ```
///
/// # This widget scrolls itself, and that is the whole design
///
/// [`List`](super::List) deliberately owns no scrolling: it cooperates with a
/// [`set_scrollable`](crate::Ui::set_scrollable) viewport. A table cannot,
/// for one structural reason — the **header**. A header inside the viewport
/// scrolls away with the rows; a header outside it is a second widget whose
/// column layout has to be kept in agreement with the first, which is exactly
/// the drift a table widget exists to prevent.
///
/// So the table windows its *data* instead: it owns the index of the first
/// visible row, draws the header pinned and then only the rows that fit, and
/// scrolling changes which records are drawn rather than moving anything.
/// The wheel scrolls it, PageUp and PageDown page it, and a selection moved
/// by keyboard drags the window along with it.
///
/// This is also what makes row count a non-question: **paint cost follows the
/// rectangle, not the data.** Ten thousand rows cost what nine cost, because
/// the rows outside the window are never even iterated.
///
/// # One column definition places everything
///
/// The header title and every cell in a column come from the same [`Column`],
/// so they cannot drift apart — the alignment bug a hand-built grid gets
/// wrong the first time somebody edits one of its two copies.
///
/// # Selection and activation are [`List`](super::List)'s contract
///
/// An [`Option`] selection, separate select and activate messages, `Enter` or
/// a double-click to activate, [`activate_on_click`](Table::activate_on_click)
/// for touch, no wrap at the ends, and **one node, so one tab stop**. The
/// double-click window reads [`Ui::tick`](crate::Ui::tick)'s clock, and the
/// pairing rules are literally shared code.
///
/// # What this deliberately is not
///
/// Cells are text. A grid of live widgets — editors in cells, buttons in
/// cells — is a different thing, and `examples/table-editor` shows the
/// pattern this toolkit prefers: select a row, edit it in a form beside the
/// table. Sorting belongs to the application, which owns the data.
#[derive(Clone, Debug)]
pub struct Table<M> {
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
    selected: Option<usize>,
    hovered: Option<usize>,
    /// Index of the first row inside the window.
    scroll: usize,
    row_height: Option<i32>,
    selection: Option<fn(usize) -> M>,
    activation: Option<fn(usize) -> M>,
    single_click: bool,
    clicks: ClickPair,
    role: Role,
    style: TextStyle,
}

impl<M> Table<M> {
    /// A table with no rows yet, reporting selection through `message`.
    pub fn new(
        columns: impl IntoIterator<Item = impl Into<Column>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
            selected: None,
            hovered: None,
            scroll: 0,
            row_height: None,
            selection: Some(message),
            activation: None,
            single_click: false,
            clicks: ClickPair::default(),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// A table that emits nothing — for a selection the application reads
    /// rather than reacts to, exactly as [`List::inert`](super::List::inert).
    /// Clicks and keys still move the selection; nothing is reported.
    pub fn inert(columns: impl IntoIterator<Item = impl Into<Column>>) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
            selected: None,
            hovered: None,
            scroll: 0,
            row_height: None,
            selection: None,
            activation: None,
            single_click: false,
            clicks: ClickPair::default(),
            role: Role::Primary,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the rows, builder-style.
    pub fn with_rows(
        mut self,
        rows: impl IntoIterator<Item = impl IntoIterator<Item = impl Into<String>>>,
    ) -> Self {
        self.set_rows(rows);
        self
    }

    /// Sets the message sent when a row is activated by `Enter` or a
    /// double-click.
    pub fn on_activate(mut self, message: fn(usize) -> M) -> Self {
        self.activation = Some(message);
        self
    }

    /// Makes a single click or tap activate as well as select — the touch
    /// answer, as on [`List`](super::List).
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

    /// Sets the cells' font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The selected row, if any.
    #[inline]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects a row **without emitting anything**. Out of range selects
    /// nothing — same honesty as [`List`](super::List).
    ///
    /// The window does not move: this is the application writing state, and
    /// yanking the view to wherever the write landed is a decision the
    /// application should make itself, with [`set_scroll`](Table::set_scroll).
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| *index < self.rows.len());
    }

    /// The index of the first row inside the window.
    #[inline]
    pub const fn scroll(&self) -> usize {
        self.scroll
    }

    /// Scrolls so `index` is the first visible row, as far as the data allows.
    ///
    /// Clamping happens against the row count here and against the rectangle
    /// at paint time, so a scroll set before the widget knows its bounds still
    /// comes out right.
    pub fn set_scroll(&mut self, index: usize) {
        self.scroll = index.min(self.rows.len().saturating_sub(1));
    }

    /// How many rows there are.
    #[inline]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The columns, in order.
    #[inline]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// One cell's text; empty for a row or column that is not there — a row
    /// shorter than the column list simply has empty cells at the end.
    pub fn cell(&self, row: usize, column: usize) -> &str {
        self.rows
            .get(row)
            .and_then(|cells| cells.get(column))
            .map_or("", String::as_str)
    }

    /// Replaces every row, dropping a selection that no longer exists and
    /// clamping the window back into range.
    pub fn set_rows(
        &mut self,
        rows: impl IntoIterator<Item = impl IntoIterator<Item = impl Into<String>>>,
    ) {
        self.rows = rows
            .into_iter()
            .map(|cells| cells.into_iter().map(Into::into).collect())
            .collect();
        self.set_selected(self.selected);
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(1));
        // A remembered click points at a row that may now be something else.
        self.clicks.forget();
    }

    /// Appends one row, reporting its index.
    pub fn push_row(&mut self, cells: impl IntoIterator<Item = impl Into<String>>) -> usize {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self.rows.len() - 1
    }

    /// Rewrites one cell, reporting whether the text actually changed.
    ///
    /// For the panel that writes its readings every cycle: an unchanged cell
    /// must not cost a repaint, and [`widget_mut`](crate::Ui::widget_mut)
    /// cannot know that on its own. Out of range changes nothing.
    pub fn update_cell(&mut self, row: usize, column: usize, text: &str) -> bool {
        let Some(cell) = self
            .rows
            .get_mut(row)
            .and_then(|cells| cells.get_mut(column))
        else {
            return false;
        };
        if cell == text {
            return false;
        }
        *cell = text.to_string();
        true
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the cells' font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Height every row — and the header — is drawn at.
    pub fn row_height(&self, theme: &Theme) -> i32 {
        self.row_height.unwrap_or(theme.metrics.size_field).max(1)
    }

    /// How many data rows a rectangle of `height` shows under its header.
    pub fn visible_rows(&self, theme: &Theme, height: i32) -> usize {
        let row = self.row_height(theme);
        ((height - row).max(0) / row) as usize
    }

    /// Height this table needs to show `rows` rows and its header.
    ///
    /// Offered, never called by the tree — the application does its own
    /// arithmetic and passes a rectangle.
    pub fn preferred_height(&self, theme: &Theme, rows: usize) -> i32 {
        let row = i64::from(self.row_height(theme));
        (row * (rows.min(i32::MAX as usize) as i64 + 1)).min(i64::from(i32::MAX)) as i32
    }

    /// The furthest the window can scroll with `fits` rows on screen.
    fn max_scroll(&self, fits: usize) -> usize {
        self.rows.len().saturating_sub(fits.max(1))
    }

    /// Moves the window so `index` is inside it — `table-editor`'s rule.
    fn ensure_visible(&mut self, index: usize, fits: usize) {
        let fits = fits.max(1);
        if index < self.scroll {
            self.scroll = index;
        } else if index >= self.scroll + fits {
            self.scroll = index + 1 - fits;
        }
    }

    /// Which row is under `point`, if any. Points in the header, in the empty
    /// space after the last row, or outside the bounds belong to no row.
    fn row_at(&self, bounds: Rect, row_height: i32, point: Point) -> Option<usize> {
        if self.rows.is_empty() || !bounds.contains(point) {
            return None;
        }
        let inside = point.y - bounds.y;
        if inside < row_height {
            return None; // the header
        }
        let slot = ((inside - row_height) / row_height.max(1)) as usize;
        let index = self.scroll.checked_add(slot)?;
        (index < self.rows.len()).then_some(index)
    }

    /// Moves the selection, dragging the window along, and reports it.
    fn select(&mut self, target: Option<usize>, fits: usize, ctx: &mut EventCtx<'_, M>) -> Handled {
        let Some(target) = target else {
            // At the end already; the key was still ours — see `List::select`.
            return Handled::Yes;
        };
        self.ensure_visible(target, fits);
        if self.selected == Some(target) {
            return Handled::Yes;
        }
        self.selected = Some(target);
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

    /// One step in the direction of travel, or `None` at the end. No wrap,
    /// for [`List`](super::List)'s reason. From nothing, lands on the near end.
    fn step(&self, forward: bool) -> Option<usize> {
        let count = self.rows.len();
        if count == 0 {
            return None;
        }
        match self.selected {
            None => Some(if forward { 0 } else { count - 1 }),
            Some(index) if forward => (index + 1 < count).then_some(index + 1),
            Some(index) => index.checked_sub(1),
        }
    }
}

/// Space between a cell's content and its column's edge.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let half = size_px as i32 / 2;
    if half < 4 { 4 } else { half }
}

/// Where each column sits inside a row of `width`: `(x offset, width)` pairs.
///
/// Fixed columns take what they asked for; flex columns share what is left,
/// with the leftmost flex columns absorbing the remainder pixel by pixel so
/// the row is filled exactly. When the fixed columns alone overflow the row,
/// the later ones are squeezed to nothing rather than drawn outside — each
/// cell is clipped to its own box, so the failure is text cut short.
fn column_spans(width: i32, columns: &[Column], pad: i32) -> Vec<(i32, i32)> {
    let gaps = pad * (columns.len().max(1) as i32 - 1) + pad * 2;
    let fixed: i64 = columns.iter().filter_map(|c| c.width.map(i64::from)).sum();
    let flexes = columns.iter().filter(|c| c.width.is_none()).count() as i64;
    let leftover = (i64::from(width) - i64::from(gaps) - fixed).max(0);
    let (share, mut spare) = if flexes > 0 {
        ((leftover / flexes) as i32, (leftover % flexes) as i32)
    } else {
        (0, 0)
    };

    let mut spans = Vec::with_capacity(columns.len());
    let mut x = pad;
    for column in columns {
        let w = match column.width {
            Some(fixed) => fixed,
            None => {
                let extra = i32::from(spare > 0);
                spare -= extra;
                share + extra
            }
        };
        // Never past the right edge: the last columns squeeze to zero instead.
        let w = w.min((width - pad - x).max(0));
        spans.push((x, w));
        x += w + pad;
    }
    spans
}

impl<M: 'static> Widget<M> for Table<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        // The header plus the rows it holds. No width: the columns divide
        // whatever they are given, which is what `flex` on a column means.
        Measured::tall(self.preferred_height(ctx.theme, self.rows.len()))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() || self.columns.is_empty() {
            return;
        }
        let row_height = self.row_height(ctx.theme);
        let pad = padding(self.style.size_px);
        let spans = column_spans(bounds.width, &self.columns, pad);
        let radius = ctx.theme.radius(Radius::Field);

        // The backdrop, exactly as `List` paints its own: a widget drawing
        // text onto whatever happened to be behind it can promise nothing.
        let (backdrop, _) = row_colors(ctx.theme, ctx.state, self.role, RowKind::Resting, true);
        canvas.fill_rect(bounds, backdrop);

        // The header: pinned, on its own recessed strip, titles placed by the
        // same spans as the cells below them.
        let header = Rect::new(bounds.x, bounds.y, bounds.width, row_height);
        let (strip, title_color) = interactive_pair(ctx.theme, Role::Base200, ctx.state);
        canvas.fill_rect(header, strip);
        for (column, &(x, w)) in self.columns.iter().zip(&spans) {
            let cell = Rect::new(bounds.x + x, header.y, w, header.height);
            if cell.is_empty() || column.title.is_empty() {
                continue;
            }
            let mut clipped = canvas.with_clip(cell);
            draw_aligned(
                &mut clipped,
                ctx.text,
                self.style,
                cell,
                (column.align, Align::Center),
                &column.title,
                title_color,
            );
        }

        // The rows that fit, and only those: rows outside the window are not
        // iterated, which is the whole virtualisation story.
        let fits = self.visible_rows(ctx.theme, bounds.height);
        let scroll = self.scroll.min(self.max_scroll(fits));
        let hovered = hovered_row(ctx.state, self.hovered);
        for slot in 0..fits {
            let index = scroll + slot;
            let Some(cells) = self.rows.get(index) else {
                break;
            };
            let row = Rect::new(
                bounds.x,
                bounds.y + row_height * (slot as i32 + 1),
                bounds.width,
                row_height,
            );
            let kind = if self.selected == Some(index) {
                RowKind::Selected
            } else if hovered == Some(index) {
                RowKind::Hovered
            } else {
                RowKind::Resting
            };
            let (fill, content) = row_colors(ctx.theme, ctx.state, self.role, kind, true);
            if kind != RowKind::Resting {
                canvas.fill_rounded_rect(row, radius, fill);
            }
            if kind == RowKind::Selected && ctx.state.contains(VisualState::FOCUSED) {
                focus_ring(ctx.theme, row, radius, canvas);
            }
            for (c, (column, &(x, w))) in self.columns.iter().zip(&spans).enumerate() {
                let cell = Rect::new(bounds.x + x, row.y, w, row.height);
                let text = cells.get(c).map_or("", String::as_str);
                if cell.is_empty() || text.is_empty() {
                    continue;
                }
                let mut clipped = canvas.with_clip(cell);
                draw_aligned(
                    &mut clipped,
                    ctx.text,
                    self.style,
                    cell,
                    (column.align, Align::Center),
                    text,
                    content,
                );
            }
        }

        // The thumb: where the window sits in the data, when there is more of
        // it than the window shows. Rounded division so a window at the end
        // reads as at the end.
        if self.rows.len() > fits && fits > 0 {
            let region = Rect::new(
                bounds.right() - THUMB,
                bounds.y + row_height,
                THUMB,
                bounds.height - row_height,
            );
            let len = self.rows.len() as i64;
            let h = (i64::from(region.height) * fits as i64 / len).max(8) as i32;
            let travel = i64::from(region.height - h);
            let top = if self.max_scroll(fits) > 0 {
                (travel * scroll as i64 / self.max_scroll(fits) as i64) as i32
            } else {
                0
            };
            let (thumb, _) = interactive_pair(ctx.theme, Role::Base300, ctx.state);
            canvas.fill_rounded_rect(
                Rect::new(region.x, region.y + top, THUMB, h),
                THUMB / 2,
                thumb,
            );
        }

        if ctx.state.contains(VisualState::FOCUSED) && self.selected.is_none() {
            focus_ring(ctx.theme, bounds, radius, canvas);
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let row_height = self.row_height(ctx.theme);
        let fits = self.visible_rows(ctx.theme, ctx.bounds.height);

        match event {
            Event::Input(InputEvent::PointerMoved { position }) => {
                let row = self.row_at(ctx.bounds, row_height, *position);
                if row == self.hovered {
                    return Handled::No;
                }
                self.hovered = row;
                Handled::Yes
            }
            Event::Input(InputEvent::PointerScroll { delta_y, .. }) => {
                if self.max_scroll(fits) == 0 {
                    // Nothing to scroll: declined, so a viewport this table
                    // sits inside can have the wheel instead.
                    return Handled::No;
                }
                // Positive delta scrolls down the content — the convention the
                // tree's own viewports follow. A delta smaller than a row
                // still moves one, so a gentle wheel is never ignored.
                let magnitude = ((delta_y.abs() as i32) / row_height).max(1) as usize;
                let scroll = if *delta_y > 0.0 {
                    self.scroll.saturating_add(magnitude)
                } else {
                    self.scroll.saturating_sub(magnitude)
                };
                let scroll = scroll.min(self.max_scroll(fits));
                if scroll == self.scroll {
                    return Handled::Yes;
                }
                self.scroll = scroll;
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
                let Some(row) = self.row_at(ctx.bounds, row_height, *position) else {
                    return Handled::No;
                };
                let intent = self.clicks.classify(row, ctx.now_ms, self.single_click);
                // Selection first, always — the second click of a pair finds
                // the row selected and emits nothing, so a pair is one
                // selection and one activation.
                let handled = self.select(Some(row), fits, ctx);
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
                KeyCode::ArrowDown => self.select(self.step(true), fits, ctx),
                KeyCode::ArrowUp => self.select(self.step(false), fits, ctx),
                KeyCode::Home => self.select((!self.rows.is_empty()).then_some(0), fits, ctx),
                KeyCode::End => self.select(self.rows.len().checked_sub(1), fits, ctx),
                KeyCode::PageDown => {
                    let target = self
                        .selected
                        .map_or(0, |index| index + fits.max(1))
                        .min(self.rows.len().saturating_sub(1));
                    self.select((!self.rows.is_empty()).then_some(target), fits, ctx)
                }
                KeyCode::PageUp => {
                    let target = self
                        .selected
                        .map_or(0, |index| index.saturating_sub(fits.max(1)));
                    self.select((!self.rows.is_empty()).then_some(target), fits, ctx)
                }
                KeyCode::Enter | KeyCode::NumpadEnter => match self.selected {
                    Some(row) => self.activate(row, ctx),
                    None => Handled::No,
                },
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// A table with no rows is not a tab stop: focusing one strands the
    /// keyboard on a widget no key does anything to. An inert one still is,
    /// as with [`List`](super::List) — its selection moves, it just says
    /// nothing about it.
    fn focusable(&self) -> bool {
        !self.rows.is_empty()
    }
}

impl<M> Describe for Table<M> {
    const KIND: &'static str = "table";
    const DOC: &'static str = "Columns of cells under a header that stays put.";
    const GROUP: Group = Group::Data;
    const ICON: &'static denise::icon::Icon = &super::icons::TABLE;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "column",
            PropertyKind::List,
            "The columns, as `column` child nodes. Real data: a table's columns are its shape, not its contents.",
        ),
        Property::new(
            "row",
            PropertyKind::Placeholder,
            "Rows to show on a canvas, as `row` child nodes inside `design`. The application supplies the real records, so these never reach a kiosk.",
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
            "A single press activates as well as selects.",
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
            // all, so a property left at its default need not be written out.
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
            // Through the setter, which drops a row that is not there rather
            // than remembering an index nothing can draw.
            "selected" => self.set_selected(Some(value.as_index()?)),
            // Built from the child nodes; an inspector edits them
            // where they live. See `PropertyKind::List`, and
            // `PropertyKind::Placeholder` for `row`, which the application
            // replaces at run time.
            "on-select" | "on-activate" | "column" | "row" => return Err(Mismatch::Supplied),
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
    use denise::theme;

    fn columns() -> Vec<Column> {
        alloc::vec![
            Column::new("Navn", 190),
            Column::flex("Rolle"),
            Column::new("Alder", 60).align_end(),
        ]
    }

    fn rows(n: usize) -> Vec<[String; 3]> {
        (0..n)
            .map(|i| {
                [
                    alloc::format!("Person {i}"),
                    alloc::format!("Rolle {i}"),
                    alloc::format!("{}", 20 + i),
                ]
            })
            .collect()
    }

    fn table(n: usize) -> Table<usize> {
        Table::new(columns(), |index| index).with_rows(rows(n))
    }

    // — the window —

    /// The window follows a keyboard selection in both directions, and only
    /// when it has to — `table-editor`'s rule, verbatim.
    #[test]
    fn the_window_follows_the_selection_only_when_it_leaves() {
        let mut t = table(100);
        t.ensure_visible(5, 10);
        assert_eq!(t.scroll(), 0, "a selection inside the window moves nothing");
        t.ensure_visible(10, 10);
        assert_eq!(t.scroll(), 1, "one past the bottom scrolls by one");
        t.ensure_visible(50, 10);
        assert_eq!(t.scroll(), 41, "a jump lands the target on the last slot");
        t.ensure_visible(3, 10);
        assert_eq!(t.scroll(), 3, "leaving upward puts the target on top");
    }

    /// The window never shows blank space below the last row when there are
    /// rows enough to fill it.
    #[test]
    fn the_scroll_clamps_to_the_last_full_window() {
        let t = table(25);
        assert_eq!(t.max_scroll(10), 15);
        assert_eq!(t.max_scroll(30), 0, "a window bigger than the data");
        assert_eq!(table(0).max_scroll(10), 0);

        let mut t = table(25);
        t.set_scroll(999);
        assert_eq!(t.scroll(), 24, "set_scroll clamps against the data");
    }

    /// Ten thousand rows and nine cost the same paint: only the window is
    /// iterated. This is the structural half of the claim — the loop runs
    /// `fits` times, so the row count cannot appear in the cost.
    #[test]
    fn a_huge_table_is_addressed_without_iterating_it() {
        let t = table(10_000);
        // Everything the paint path asks of the data is indexed, not scanned.
        assert_eq!(t.cell(9_999, 0), "Person 9999");
        assert_eq!(t.max_scroll(9), 9_991);
        assert_eq!(
            t.visible_rows(&theme::DARK, 400),
            (400 / t.row_height(&theme::DARK) - 1) as usize
        );
    }

    /// The header takes the first row's worth of height; what is left is rows.
    #[test]
    fn the_header_costs_one_row_of_height() {
        let t = table(5).with_row_height(30);
        assert_eq!(t.visible_rows(&theme::DARK, 300), 9);
        assert_eq!(
            t.visible_rows(&theme::DARK, 30),
            0,
            "room only for the header"
        );
        assert_eq!(t.visible_rows(&theme::DARK, 0), 0);
        assert_eq!(t.preferred_height(&theme::DARK, 5), 180);
    }

    // — hits —

    /// The header is nobody's row, the empty space below the last row is
    /// nobody's row, and a hit accounts for the window.
    #[test]
    fn hits_land_on_the_data_row_not_the_screen_row() {
        let mut t = table(50).with_row_height(20);
        t.set_scroll(30);
        let bounds = Rect::new(10, 10, 300, 100);
        assert_eq!(
            t.row_at(bounds, 20, Point::new(50, 15)),
            None,
            "the header is not a row"
        );
        assert_eq!(
            t.row_at(bounds, 20, Point::new(50, 35)),
            Some(30),
            "the first slot is the scrolled-to row"
        );
        assert_eq!(t.row_at(bounds, 20, Point::new(50, 95)), Some(33));
        assert_eq!(t.row_at(bounds, 20, Point::new(400, 35)), None, "outside");

        let mut short = table(2).with_row_height(20);
        short.set_scroll(0);
        assert_eq!(
            short.row_at(bounds, 20, Point::new(50, 75)),
            None,
            "below the last row is nobody's"
        );
    }

    // — columns —

    /// Fixed columns get what they asked for; flex columns share the rest and
    /// the row is filled exactly, remainder and all.
    #[test]
    fn flex_columns_share_the_leftover_exactly() {
        let cols = alloc::vec![Column::new("a", 100), Column::flex("b"), Column::flex("c"),];
        let spans = column_spans(400, &cols, 10);
        assert_eq!(spans[0], (10, 100));
        // Leftover: 400 - 4 gaps of 10 - 100 fixed = 260, shared 130/130.
        assert_eq!(spans[1].1 + spans[2].1, 260);
        assert!(
            (spans[1].1 - spans[2].1).abs() <= 1,
            "shares differ by more than the remainder"
        );
        // The row is filled exactly: last column ends one pad from the edge.
        assert_eq!(spans[2].0 + spans[2].1, 400 - 10);
    }

    /// Fixed widths that overflow the row squeeze the later columns to zero
    /// rather than drawing outside it.
    #[test]
    fn overflowing_columns_squeeze_rather_than_escape() {
        let cols = alloc::vec![
            Column::new("a", 300),
            Column::new("b", 300),
            Column::flex("c"),
        ];
        for width in [0, 50, 320, 640] {
            let spans = column_spans(width, &cols, 8);
            for (i, &(x, w)) in spans.iter().enumerate() {
                assert!(w >= 0, "width {width}: column {i} inverted");
                // A zero-width column draws nothing, so only a column with ink
                // can escape.
                assert!(
                    w == 0 || x + w <= width,
                    "width {width}: column {i} escaped ({x}+{w})"
                );
            }
        }
    }

    /// A bare string is a flex column, so the common case needs no builder.
    #[test]
    fn a_bare_string_is_a_flex_column() {
        let t: Table<usize> = Table::new(["Navn", "Rolle"], |i| i);
        assert_eq!(t.columns().len(), 2);
        assert_eq!(t.columns()[0].title(), "Navn");
        assert!(t.columns()[0].width.is_none());
    }

    // — the data —

    /// A short row reads as empty cells, not a panic — rows and columns come
    /// from different places and will disagree.
    #[test]
    fn a_short_row_has_empty_cells_at_the_end() {
        let mut t: Table<usize> = Table::new(columns(), |i| i);
        t.push_row(["bare", "to"]);
        assert_eq!(t.cell(0, 0), "bare");
        assert_eq!(t.cell(0, 2), "", "the missing cell is empty");
        assert_eq!(t.cell(5, 0), "", "a missing row is empty too");
    }

    /// Rewriting an unchanged cell reports nothing to repaint.
    #[test]
    fn writing_the_same_cell_reports_no_change() {
        let mut t = table(3);
        assert!(t.update_cell(1, 2, "99"));
        assert!(!t.update_cell(1, 2, "99"));
        assert_eq!(t.cell(1, 2), "99");
        assert!(!t.update_cell(50, 0, "x"), "out of range changes nothing");
    }

    /// Replacing the rows drops what no longer exists — selection, window and
    /// the remembered half of a click pair.
    #[test]
    fn replacing_the_rows_drops_what_no_longer_exists() {
        let mut t = table(50);
        t.set_selected(Some(40));
        t.set_scroll(35);
        t.set_rows(rows(5));
        assert_eq!(t.selected(), None, "the selection pointed past the end");
        assert_eq!(t.scroll(), 4, "the window clamped back into the data");

        let mut kept = table(50);
        kept.set_selected(Some(3));
        kept.set_rows(rows(10));
        assert_eq!(kept.selected(), Some(3), "a selection that exists survives");
    }

    /// Stepping does not wrap, and from nothing lands on the near end.
    #[test]
    fn stepping_stops_at_the_ends() {
        let mut t = table(3);
        assert_eq!(t.step(true), Some(0), "first press lands on the near end");
        t.set_selected(Some(2));
        assert_eq!(t.step(true), None, "no wrap at the bottom");
        t.set_selected(Some(0));
        assert_eq!(t.step(false), None, "no wrap at the top");
        assert_eq!(table(0).step(true), None, "an empty table has no step");
    }

    /// An empty table is not a tab stop; an inert one with rows still is,
    /// following `List`: its selection moves, it just emits nothing.
    #[test]
    fn only_a_table_with_rows_is_a_tab_stop() {
        let empty_inert: Table<usize> = Table::inert(columns());
        assert!(!Widget::<usize>::focusable(&empty_inert));
        let empty: Table<usize> = Table::new(columns(), |i| i);
        assert!(!Widget::<usize>::focusable(&empty));
        assert!(Widget::<usize>::focusable(&table(3)));
    }
}
