//! A row of labels where one is selected.

use alloc::string::String;
use alloc::vec::Vec;

use denise::Pen;
use denise::theme::{AA, contrast_x100, derive_content};
use denise::{Color, ElementState, InputEvent, KeyCode, Point, PointerButton, Rect, Role, Theme};
use denise_text::{TextEngine, TextStyle};

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{
    Align, ClickPair, Intent, draw_aligned, hovered_row, interactive_pair, muted,
};

/// A tab strip: a row of labels, one of them selected, with a rule underneath.
///
/// # The strip, and the pages under it
///
/// On its own the widget fills its node and selects nothing: an application
/// listens to its message and shows and hides pages it built itself. A form
/// file can also nest a page under each `tab`, in which case the node hosts
/// them and the strip draws in a band along its top — see [`Tabs::over_pages`]
/// and [`Tabs::strip_height`]. Either way **this widget still owns only which
/// tab is selected**; what changes is who owns the pages.
///
/// ```
/// # use denise_ui::Tabs;
/// enum Message { Page(usize) }
/// Tabs::new(["Oversikt", "Alarmer", "Innstillinger"], Message::Page);
/// ```
///
/// # What it owns, and what it does not
///
/// **This widget owns which tab is selected, and nothing else.** Showing and
/// hiding the pages is [`Ui::set_visible`](crate::Ui::set_visible) on nodes the
/// application already owns.
///
/// That is a deliberate line rather than an omission. A tab strip that owned its
/// pages would have to own their *layout* — where each page goes, how big it is,
/// what happens when the strip moves — and that is a layout engine. Owning one
/// index is the part nobody else can do better.
///
/// # A strip people work in
///
/// A strip of documents is handled more than a strip of sections: tabs are
/// closed, dragged into order, renamed and coloured. Made with
/// [`Tabs::with_events`], the strip reports each of those as a [`TabEvent`],
/// and the line above still holds — it reports, and the application decides:
///
/// - **Closing** is asked for, never done. The close button ([`with_close_buttons`])
///   and a middle click report [`TabEvent::Close`]; the tab stays until the
///   application removes it, because a document with unsaved changes asks first.
/// - **Dragging** is the one thing the strip does on its own, because the tab
///   has to move under the pointer while it is held and only the strip can draw
///   that. It reports [`TabEvent::Moved`] once, when the tab is let go, and the
///   application moves its own list the same way.
/// - **Renaming** is a double click, reported as [`TabEvent::Activated`]. The
///   strip has no text field; [`tab_rect`] says where to put one.
/// - **A right click** reports [`TabEvent::Menu`] with the point, for
///   [`open_menu_at`](super::open_menu_at).
/// - **A colour** tints a tab and marks its top edge ([`Tabs::set_color`]). The
///   label stays readable on the tint whatever colour is chosen.
///
/// A strip made with [`Tabs::new`] does none of this and behaves as it always
/// has.
///
/// ```
/// # use denise_ui::{TabEvent, Tabs};
/// enum Message { Tab(TabEvent) }
/// Tabs::with_events(["notes.txt", "server.log"], Message::Tab).with_close_buttons(true);
/// ```
///
/// [`with_close_buttons`]: Tabs::with_close_buttons
///
/// # Keyboard
///
/// The strip is **one tab stop**, like [`RadioGroup`](super::RadioGroup) and for
/// the same reason: `Tab` should move from the strip into the page, not through
/// three tabs first. Left and Right move the selection and wrap; `Home` and `End`
/// go to the ends.
///
/// Up and Down are deliberately *not* handled. A tab strip is horizontal, and
/// the vertical keys almost always belong to whatever is in the page below it —
/// which is the opposite of `RadioGroup`, where a vertical list takes all four.
///
/// # A row wider than the strip
///
/// The row slides left as far as it takes to show the selected tab, so the tab
/// being looked at is never the one clipped off the end.
#[derive(Clone, Debug)]
pub struct Tabs<M> {
    labels: Vec<String>,
    /// Each tab's colour, or `None` for the panel's own. As long as `labels`.
    colors: Vec<Option<Color>>,
    selected: usize,
    report: Report<M>,
    /// Whether each tab carries a button that asks for it to be closed.
    closable: bool,
    role: Role,
    style: TextStyle,
    /// Whether the node this sits on is hosting a page below the strip.
    ///
    /// A strip on its own fills its node, which is what a `tabs` node has
    /// always been and what a form that sets `h=40` is asking for. A strip over
    /// pages draws in a band of [`Tabs::strip_height`] along the top and leaves
    /// the rest to the page, the way `Collapse` leaves everything below its
    /// header to the body. The *builder* knows which, because it can see
    /// whether any `tab` node carries children; the widget cannot.
    over_pages: bool,
    /// The tab under the pointer, while the pointer is over the strip.
    hovered: Option<usize>,
    /// A press on a tab, until it is let go.
    press: Option<Press>,
    clicks: ClickPair,
}

/// What a person did to a strip made with [`Tabs::with_events`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabEvent {
    /// A tab was chosen — clicked, or reached with the arrow keys.
    Selected(usize),
    /// A tab's close button was clicked, or the tab was middle-clicked. The
    /// strip removes nothing.
    Close(usize),
    /// A tab was dragged from `from` and let go at `to`, and the strip has
    /// already moved it there. Indices after the move: the tab that was at
    /// `from` is at `to`, and the ones between moved one place to make room.
    Moved {
        /// Where the tab was.
        from: usize,
        /// Where it is now.
        to: usize,
    },
    /// A tab was double-clicked: the gesture that renames one.
    Activated(usize),
    /// A tab was right-clicked at `at`, in surface pixels: where its context
    /// menu opens.
    Menu {
        /// The tab under the pointer.
        index: usize,
        /// Where the pointer was.
        at: Point,
    },
}

/// Who hears about it.
#[derive(Debug)]
enum Report<M> {
    Nothing,
    Index(fn(usize) -> M),
    Events(fn(TabEvent) -> M),
}

// By hand, because the derives would ask for `M: Copy`, and a function pointer
// is copied whatever it returns.
impl<M> Clone for Report<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for Report<M> {}

/// A press on a tab.
#[derive(Clone, Copy, Debug)]
struct Press {
    /// Where the tab is now: where it was pressed, until a drag moves it.
    index: usize,
    /// Where it was when it was pressed.
    from: usize,
    button: PointerButton,
    /// Whether the press landed on the tab's close button.
    on_close: bool,
    start: Point,
    /// How far into the tab the pointer took hold, so a dragged tab stays
    /// under the pointer at the place it was picked up.
    grab: i32,
    /// The pointer's x once the press has moved far enough to be a drag.
    dragging: Option<i32>,
}

impl<M> Tabs<M> {
    /// A strip with the first tab selected, reporting the index of each tab
    /// chosen.
    pub fn new(
        labels: impl IntoIterator<Item = impl Into<String>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self::reporting(labels, Report::Index(message))
    }

    /// A strip that reports everything done to it — choosing, closing,
    /// dragging, renaming and right-clicking a tab — as a [`TabEvent`].
    pub fn with_events(
        labels: impl IntoIterator<Item = impl Into<String>>,
        message: fn(TabEvent) -> M,
    ) -> Self {
        Self::reporting(labels, Report::Events(message))
    }

    /// A strip that emits nothing.
    pub fn inert(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::reporting(labels, Report::Nothing)
    }

    fn reporting(labels: impl IntoIterator<Item = impl Into<String>>, report: Report<M>) -> Self {
        let labels: Vec<String> = labels.into_iter().map(Into::into).collect();
        Self {
            colors: alloc::vec![None; labels.len()],
            labels,
            selected: 0,
            report,
            closable: false,
            role: Role::Primary,
            style: TextStyle::built_in(16),
            over_pages: false,
            hovered: None,
            press: None,
            clicks: ClickPair::default(),
        }
    }

    /// A strip that sits above pages hosted on its own node.
    ///
    /// Set by `denise-forms` when a `tab` in the file carries children. It
    /// changes only where the strip is drawn — the band along the top rather
    /// than the whole node — so that the page below it is visible.
    #[must_use]
    pub fn over_pages(mut self) -> Self {
        self.over_pages = true;
        self
    }

    /// Whether this strip is drawn in a band rather than filling its node.
    #[inline]
    pub const fn is_over_pages(&self) -> bool {
        self.over_pages
    }

    /// The strip's own height: the theme's field height.
    ///
    /// The band this widget draws in, and the offset a form places a tab's page
    /// at — one definition, so the two cannot drift. The same shape as
    /// [`Collapse::header_height`](super::Collapse::header_height), and for the
    /// same reason: the widget is the strip, and the node it sits on may be
    /// much taller because it is hosting a page below.
    ///
    /// A node no taller than this is a strip and nothing else, which is what a
    /// `tabs` node was before a `tab` could hold anything.
    pub fn strip_height(&self, theme: &Theme) -> i32 {
        theme.metrics.size_field.max(1)
    }

    /// The part of `bounds` this strip draws in and answers clicks in.
    fn band(&self, bounds: Rect, theme: &Theme) -> Rect {
        if !self.over_pages {
            return bounds;
        }
        let height = bounds.height.min(self.strip_height(theme));
        Rect::new(bounds.x, bounds.y, bounds.width, height)
    }

    /// Sets the initially selected tab. Out of range selects the last one.
    pub fn with_selected(mut self, index: usize) -> Self {
        self.selected = self.clamp(index);
        self
    }

    /// Sets the colour of the selected tab's underline.
    ///
    /// Only the underline — a bar, not text. An earlier version drew the selected
    /// *label* in this colour, and `Secondary` on the light theme is 2.34:1
    /// against the panel, which is a label nobody can read. The rule the labels
    /// follow now cannot depend on which role a caller passes.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the labels' font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Gives every tab a button at its trailing end that asks for it to be
    /// closed, shown on the selected tab and the one under the pointer.
    ///
    /// Only a strip made with [`Tabs::with_events`] has anything to report it
    /// with; on any other the button is drawn and does nothing.
    #[must_use]
    pub fn with_close_buttons(mut self, on: bool) -> Self {
        self.closable = on;
        self
    }

    /// Sets each tab's colour, in order. See [`Tabs::set_colors`].
    #[must_use]
    pub fn with_colors(mut self, colors: impl IntoIterator<Item = Option<Color>>) -> Self {
        self.set_colors(colors);
        self
    }

    /// The selected index. Always in range while the strip has tabs.
    #[inline]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// The selected tab's label, or `None` for an empty strip.
    #[inline]
    pub fn selected_label(&self) -> Option<&str> {
        self.labels.get(self.selected).map(String::as_str)
    }

    /// Selects a tab **without emitting anything**. Out of range is clamped.
    pub fn set_selected(&mut self, index: usize) {
        self.selected = self.clamp(index);
    }

    /// The labels, in order.
    #[inline]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Replaces the labels, keeping the selection in range.
    ///
    /// Colours stay with the places they were set on; a tab past the old end
    /// has none.
    pub fn set_labels(&mut self, labels: impl IntoIterator<Item = impl Into<String>>) {
        self.labels = labels.into_iter().map(Into::into).collect();
        self.colors.resize(self.labels.len(), None);
        self.selected = self.clamp(self.selected);
        self.forget_pointer();
    }

    /// Renames tab `index`. Out of range is ignored.
    pub fn set_label(&mut self, index: usize, label: impl Into<String>) {
        if let Some(slot) = self.labels.get_mut(index) {
            *slot = label.into();
        }
    }

    /// Each tab's colour, in order: `None` for a tab drawn on the panel.
    #[inline]
    pub fn colors(&self) -> &[Option<Color>] {
        &self.colors
    }

    /// Sets each tab's colour, in order. A list shorter than the tabs leaves
    /// the rest uncoloured, and a longer one is cut to them.
    pub fn set_colors(&mut self, colors: impl IntoIterator<Item = Option<Color>>) {
        self.colors = colors.into_iter().collect();
        self.colors.resize(self.labels.len(), None);
    }

    /// Colours tab `index`, or takes its colour away. Out of range is ignored.
    pub fn set_color(&mut self, index: usize, color: Option<Color>) {
        if let Some(slot) = self.colors.get_mut(index) {
            *slot = color;
        }
    }

    /// Moves tab `from` to `to` **without emitting anything**, its label and
    /// colour with it. The selection follows the tab it was on.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let count = self.labels.len();
        if from >= count || to >= count || from == to {
            return;
        }
        let label = self.labels.remove(from);
        self.labels.insert(to, label);
        let color = self.colors.remove(from);
        self.colors.insert(to, color);
        self.selected = moved_index(self.selected, from, to);
        self.hovered = None;
        self.clicks.forget();
    }

    /// Whether the tabs carry close buttons.
    #[inline]
    pub const fn has_close_buttons(&self) -> bool {
        self.closable
    }

    /// Shows the close buttons, or stops. See [`Tabs::with_close_buttons`].
    pub fn set_close_buttons(&mut self, on: bool) {
        self.closable = on;
    }

    /// Replaces the colour role.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the labels' font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Width the whole strip needs for every tab at its natural width.
    ///
    /// A strip given less than this clips its last tabs rather than shrinking
    /// them — a tab whose label is cut in half says less than one that is not
    /// there, and squeezing them all would make the strip reflow every time a
    /// label changed.
    pub fn preferred_width(&self, engine: &mut TextEngine) -> i32 {
        self.widths(engine).iter().sum()
    }

    /// Each tab's width, in order.
    fn widths(&self, engine: &mut TextEngine) -> Vec<i32> {
        widths(&self.labels, self.style, self.closable, engine)
    }

    /// Where each tab is drawn in `band`.
    fn layout(&self, band: Rect, engine: &mut TextEngine) -> Vec<Rect> {
        lay_out(
            &self.labels,
            self.style,
            self.closable,
            self.selected,
            band,
            engine,
        )
    }

    #[inline]
    fn clamp(&self, index: usize) -> usize {
        index.min(self.labels.len().saturating_sub(1))
    }

    /// Moves the selection by one, wrapping.
    fn step(&self, forward: bool) -> usize {
        let count = self.labels.len();
        if count == 0 {
            return 0;
        }
        if forward {
            (self.selected + 1) % count
        } else {
            (self.selected + count - 1) % count
        }
    }

    /// Drops what the pointer was doing: the tabs it knew about moved.
    fn forget_pointer(&mut self) {
        self.hovered = None;
        self.press = None;
        self.clicks.forget();
    }

    fn reports_events(&self) -> bool {
        matches!(self.report, Report::Events(_))
    }

    /// Tells the application, in whichever form it asked to be told. A strip
    /// that reports indices hears only about selection.
    fn emit(&self, ctx: &mut EventCtx<'_, M>, event: TabEvent) {
        match (self.report, event) {
            (Report::Events(message), event) => ctx.emit(message(event)),
            (Report::Index(message), TabEvent::Selected(index)) => ctx.emit(message(index)),
            _ => {}
        }
    }

    fn select(&mut self, index: usize, ctx: &mut EventCtx<'_, M>) -> Handled {
        if index == self.selected {
            // Nothing changed, so nothing is reported — but the event was still
            // this widget's to handle.
            return Handled::Yes;
        }
        self.selected = index;
        self.emit(ctx, TabEvent::Selected(index));
        Handled::Yes
    }

    /// The close button of a tab drawn at `tab` in `band`: a square at its
    /// trailing end, level with the label.
    fn close_rect(&self, tab: Rect, band: Rect) -> Rect {
        close_rect(self.style.size_px, tab, band)
    }

    fn pressed(
        &mut self,
        button: PointerButton,
        position: Point,
        band: Rect,
        ctx: &mut EventCtx<'_, M>,
    ) -> Handled {
        if !self.reports_events() {
            return Handled::No;
        }
        let tabs = self.layout(band, ctx.text);
        let Some(index) = hit(band, &tabs, position) else {
            self.press = None;
            return Handled::No;
        };
        match button {
            PointerButton::Right => {
                self.press = None;
                self.emit(
                    ctx,
                    TabEvent::Menu {
                        index,
                        at: position,
                    },
                );
                Handled::Yes
            }
            PointerButton::Left | PointerButton::Middle => {
                let on_close = button == PointerButton::Left
                    && self.closable
                    && self.close_rect(tabs[index], band).contains(position);
                self.press = Some(Press {
                    index,
                    from: index,
                    button,
                    on_close,
                    start: position,
                    grab: position.x - tabs[index].x,
                    dragging: None,
                });
                Handled::Yes
            }
            PointerButton::Other(_) => Handled::No,
        }
    }

    fn pointer_moved(&mut self, position: Point, band: Rect, ctx: &mut EventCtx<'_, M>) -> Handled {
        let tabs = self.layout(band, ctx.text);
        let over = hit(band, &tabs, position);
        // The hovered tab only changes the picture when it shows a close button.
        let mut changed = false;
        if over != self.hovered {
            self.hovered = over;
            changed = self.closable;
        }
        let Some(mut press) = self.press else {
            return if changed { Handled::Yes } else { Handled::No };
        };
        if press.button != PointerButton::Left || press.on_close {
            return if changed { Handled::Yes } else { Handled::No };
        }
        if press.dragging.is_none()
            && (position.x - press.start.x).abs() < drag_threshold(self.style.size_px)
        {
            return if changed { Handled::Yes } else { Handled::No };
        }
        press.dragging = Some(position.x);
        let Some(tab) = tabs.get(press.index) else {
            self.press = None;
            return Handled::Yes;
        };
        let centre = position.x - press.grab + tab.width / 2;
        if let Some(to) = drag_target(&tabs, press.index, centre) {
            self.move_tab(press.index, to);
            press.index = to;
        }
        self.press = Some(press);
        Handled::Yes
    }

    fn released(
        &mut self,
        button: PointerButton,
        position: Point,
        band: Rect,
        ctx: &mut EventCtx<'_, M>,
    ) -> Handled {
        let tabs = self.layout(band, ctx.text);
        if !self.reports_events() {
            return match hit(band, &tabs, position) {
                Some(index) => self.select(index, ctx),
                None => Handled::No,
            };
        }
        let Some(press) = self.press.take() else {
            return Handled::No;
        };
        if press.button != button {
            return Handled::No;
        }
        if press.dragging.is_some() {
            if press.index != press.from {
                self.emit(
                    ctx,
                    TabEvent::Moved {
                        from: press.from,
                        to: press.index,
                    },
                );
            }
            self.clicks.forget();
            return self.select(press.index, ctx);
        }
        // Let go somewhere other than where it was pressed: nothing, the way a
        // button's press dragged off is nothing.
        if hit(band, &tabs, position) != Some(press.index) {
            return Handled::Yes;
        }
        let index = press.index;
        if button == PointerButton::Middle {
            self.emit(ctx, TabEvent::Close(index));
            return Handled::Yes;
        }
        if press.on_close {
            if self.close_rect(tabs[index], band).contains(position) {
                self.emit(ctx, TabEvent::Close(index));
            }
            return Handled::Yes;
        }
        self.select(index, ctx);
        if self.clicks.classify(index, ctx.now_ms, false) == Intent::Activate {
            self.emit(ctx, TabEvent::Activated(index));
        }
        Handled::Yes
    }
}

/// Where tab `index` of the strip at `id` is drawn, in surface pixels: where an
/// application puts the field that renames it, or anchors something to it.
///
/// A free function for the reason [`title_layout`](super::title_layout) is one:
/// measuring a label needs the text engine, which belongs to the tree the strip
/// is borrowed from. `None` when `id` is not a strip or `index` is past its last
/// tab.
pub fn tab_rect<M: 'static>(
    ui: &mut crate::Ui<M>,
    id: crate::NodeId,
    index: usize,
) -> Option<Rect> {
    let bounds = ui.bounds(id)?;
    let theme = *ui.theme();
    let (band, labels, style, closable, selected) = {
        let strip = ui.widget::<Tabs<M>>(id)?;
        (
            strip.band(bounds, &theme),
            strip.labels.clone(),
            strip.style,
            strip.closable,
            strip.selected,
        )
    };
    lay_out(&labels, style, closable, selected, band, ui.text_mut())
        .get(index)
        .copied()
}

/// Each tab's width: its label, padding either side, and room for a close
/// button when there is one.
fn widths(
    labels: &[String],
    style: TextStyle,
    closable: bool,
    engine: &mut TextEngine,
) -> Vec<i32> {
    let pad = padding(style.size_px);
    let close = if closable {
        close_size(style.size_px)
    } else {
        0
    };
    labels
        .iter()
        .map(|label| engine.measure_line(style, label) + pad * 2 + close)
        .collect()
}

/// Where each tab is drawn in `band`: end to end from the leading edge, slid
/// left as far as it takes to show the selected tab.
fn lay_out(
    labels: &[String],
    style: TextStyle,
    closable: bool,
    selected: usize,
    band: Rect,
    engine: &mut TextEngine,
) -> Vec<Rect> {
    let mut tabs = place(band, &widths(labels, style, closable, engine));
    let shift = reveal_shift(band, &tabs, selected);
    for tab in &mut tabs {
        tab.x -= shift;
    }
    tabs
}

/// The panel behind the labels, the selected label's colour, and the others'.
///
/// One function so the paint path and the contrast test cannot disagree about
/// what is actually drawn.
///
/// **A disabled strip does not mute**, and it does not have to say so here:
/// `interactive_pair` derives its disabled content by mixing until it *just*
/// clears the contrast floor, and [`muted`] hands back anything that cannot
/// afford the shift. A disabled strip is already recessed as a whole, and the
/// selection still reads from the underline.
fn label_colors(
    theme: &denise::Theme,
    state: VisualState,
) -> (denise::Color, denise::Color, denise::Color) {
    let (surface, content) = interactive_pair(theme, Role::Base100, state);
    (surface, content, muted(surface, content))
}

/// How far a tab's colour is mixed into the panel behind it, out of 255.
///
/// Enough to tell a red tab from a blue one at a glance; not so much that the
/// strip turns into a row of buttons. The full colour is in the bar along the
/// tab's top edge.
const TINT: u8 = 64;

/// The panel under a tab coloured `color`.
fn tinted(surface: Color, color: Color) -> Color {
    surface.mix(color, TINT)
}

/// The selected and resting label colours on a tint: the strip's own while they
/// are readable there, and otherwise derived from the tint itself.
///
/// A colour is the caller's and can be anything, so unlike a role it comes with
/// no promise about what reads on it — this is where the promise is made.
fn labels_on(tint: Color, content: Color) -> (Color, Color) {
    let selected = if contrast_x100(tint, content) >= AA {
        content
    } else {
        derive_content(tint, AA)
    };
    (selected, muted(tint, selected))
}

/// Space each side of a label.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let value = size_px as i32;
    if value < 8 { 8 } else { value }
}

/// The side of a tab's close button.
#[inline]
const fn close_size(size_px: u16) -> i32 {
    let value = size_px as i32;
    if value < 12 { 12 } else { value }
}

/// The rule under the strip, and the bar along a coloured tab's top.
#[inline]
const fn rule_thickness(band: Rect) -> i32 {
    let value = band.height / 10;
    if value < 2 { 2 } else { value }
}

/// How far a press moves before it is a drag: far enough that a click with an
/// unsteady hand is still a click.
#[inline]
const fn drag_threshold(size_px: u16) -> i32 {
    let value = size_px as i32 / 3;
    if value < 4 { 4 } else { value }
}

/// The close button of a tab drawn at `tab` in `band`.
fn close_rect(size_px: u16, tab: Rect, band: Rect) -> Rect {
    let size = close_size(size_px);
    let pad = padding(size_px);
    let y = tab.y + (tab.height - rule_thickness(band) - size) / 2;
    Rect::new(tab.right() - pad / 2 - size, y, size, size)
}

/// Where each tab sits, laid left to right from the leading edge.
///
/// Tabs that fall past the right edge are still placed — the canvas clips them,
/// and a rectangle that says where a tab *would* be keeps hit testing and
/// drawing agreeing about it.
fn place(bounds: Rect, widths: &[i32]) -> Vec<Rect> {
    let mut x = bounds.x;
    widths
        .iter()
        .map(|width| {
            let rect = Rect::new(x, bounds.y, *width, bounds.height);
            x += width;
            rect
        })
        .collect()
}

/// How far left a row of `tabs` slides so that tab `selected` is entirely in
/// `band`. A tab wider than the band shows its leading edge.
fn reveal_shift(band: Rect, tabs: &[Rect], selected: usize) -> i32 {
    let Some(tab) = tabs.get(selected) else {
        return 0;
    };
    let overflow = tab.right() - band.right();
    if overflow <= 0 {
        return 0;
    }
    overflow.min(tab.x - band.x).max(0)
}

/// Which tab contains `point`, if any.
fn hit(bounds: Rect, tabs: &[Rect], point: Point) -> Option<usize> {
    if !bounds.contains(point) {
        return None;
    }
    tabs.iter().position(|tab| tab.contains(point))
}

/// Where the tab at `index`, dragged so its centre is at `centre`, belongs:
/// past every neighbour whose centre it has crossed.
///
/// Centres rather than edges, so tabs of different widths do not trade back
/// and forth: once a tab has passed its neighbour's centre, the neighbour's new
/// centre is further behind it by the dragged tab's whole width.
fn drag_target(tabs: &[Rect], index: usize, centre: i32) -> Option<usize> {
    let middle = |tab: &Rect| tab.x + tab.width / 2;
    let mut to = index;
    while to + 1 < tabs.len() && centre > middle(&tabs[to + 1]) {
        to += 1;
    }
    if to == index {
        while to > 0 && centre < middle(&tabs[to - 1]) {
            to -= 1;
        }
    }
    (to != index).then_some(to)
}

/// Where the tab that was at `index` is after the tab at `from` moved to `to`.
const fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

/// An × filling the middle of `rect`.
fn draw_cross(canvas: &mut Pen<'_>, rect: Rect, color: Color) {
    let inset = rect.width / 4;
    let (x0, y0) = ((rect.x + inset) * 256, (rect.y + inset) * 256);
    let (x1, y1) = ((rect.right() - inset) * 256, (rect.bottom() - inset) * 256);
    // Half a stroke, measured square to the diagonal: a stroke is about a
    // seventh of the button, and never thinner than a pixel.
    let half = (rect.width * 256 / 14).max(128) * 181 / 256;
    canvas.fill_polygon_fx(
        &[
            (x0 + half, y0 - half),
            (x1 + half, y1 - half),
            (x1 - half, y1 + half),
            (x0 - half, y0 + half),
        ],
        color,
    );
    canvas.fill_polygon_fx(
        &[
            (x1 + half, y0 + half),
            (x0 + half, y1 + half),
            (x0 - half, y1 - half),
            (x1 - half, y0 - half),
        ],
        color,
    );
}

impl<M: 'static> Widget<M> for Tabs<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        Measured::both(
            self.preferred_width(ctx.text),
            ctx.theme.metrics.size_field.max(1),
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        // A strip over pages is as tall as the page it shows, and what this
        // widget draws is the band along its top. A strip on its own fills its
        // node, which is what `tabs h=40` has always meant.
        let bounds = self.band(ctx.bounds, ctx.theme);
        if bounds.is_empty() || self.labels.is_empty() {
            return;
        }
        let mut tabs = self.layout(bounds, ctx.text);

        // A rule under the whole strip, with the selected tab's segment drawn
        // over it. Cheaper than a box per tab, and it reads as a strip rather
        // than as a row of unrelated buttons.
        let thickness = rule_thickness(bounds);
        let rule = Rect::new(
            bounds.x,
            bounds.bottom() - thickness,
            bounds.width,
            thickness,
        );
        canvas.fill_rect(rule, ctx.theme.color(Role::Base300));

        // Neither label colour depends on `self.role`: a role is only guaranteed
        // against *its own* content, not against the surface a label sits on.
        let (surface, content, resting) = label_colors(ctx.theme, ctx.state);
        let underline = if ctx.state.contains(VisualState::DISABLED) {
            resting
        } else {
            ctx.theme.color(self.role)
        };
        let hovered = hovered_row(ctx.state, self.hovered);
        let close = if self.closable {
            close_size(self.style.size_px)
        } else {
            0
        };

        // A tab being dragged is drawn where the pointer holds it, over the
        // others, and so last.
        let dragged = self
            .press
            .and_then(|press| press.dragging.map(|x| (press.index, x - press.grab)));
        if let Some((index, x)) = dragged
            && let Some(tab) = tabs.get_mut(index)
        {
            tab.x = x.clamp(bounds.x, (bounds.right() - tab.width).max(bounds.x));
        }
        let order = (0..tabs.len())
            .filter(|&index| Some(index) != dragged.map(|(d, _)| d))
            .chain(dragged.map(|(d, _)| d));

        for index in order {
            let tab = tabs[index];
            let chosen = index == self.selected;
            let face = Rect::new(tab.x, tab.y, tab.width, tab.height - thickness);
            let (on, off) = match self.colors.get(index).copied().flatten() {
                Some(color) => {
                    let tint = tinted(surface, color);
                    canvas.fill_rect(face, tint);
                    canvas.fill_rect(Rect::new(tab.x, tab.y, tab.width, thickness), color);
                    labels_on(tint, content)
                }
                None => {
                    if dragged.is_some_and(|(d, _)| d == index) {
                        // Lifted off the strip: opaque, so the tabs it passes
                        // over do not show through its label.
                        canvas.fill_rect(face, surface);
                    }
                    (content, resting)
                }
            };
            if chosen {
                canvas.fill_rect(Rect::new(tab.x, rule.y, tab.width, thickness), underline);
            }
            // The label sits above the rule, not centred in the whole height, or
            // a tall strip puts its text on top of its own underline.
            let text = Rect::new(tab.x, tab.y, tab.width - close, tab.height - thickness);
            draw_aligned(
                canvas,
                ctx.text,
                self.style,
                text,
                (Align::Center, Align::Center),
                &self.labels[index],
                if chosen { on } else { off },
            );
            if self.closable && (chosen || hovered == Some(index)) {
                draw_cross(
                    canvas,
                    self.close_rect(tab, bounds),
                    if chosen { on } else { off },
                );
            }
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        if self.labels.is_empty() {
            return Handled::No;
        }
        let band = self.band(ctx.bounds, ctx.theme);
        let chosen = match event {
            Event::Input(InputEvent::PointerMoved { position }) => {
                return self.pointer_moved(*position, band, ctx);
            }
            Event::Input(InputEvent::PointerButton {
                button,
                state: ElementState::Down,
                position,
                ..
            }) => return self.pressed(*button, *position, band, ctx),
            Event::Input(InputEvent::PointerButton {
                button,
                state: ElementState::Up,
                position,
                ..
            }) => return self.released(*button, *position, band, ctx),
            Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => {
                let tabs = self.layout(band, ctx.text);
                hit(band, &tabs, *position)
            }
            // Left and Right only. A tab strip is horizontal, and Up and Down
            // almost always belong to whatever is in the page below it.
            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => match code {
                KeyCode::ArrowLeft => Some(self.step(false)),
                KeyCode::ArrowRight => Some(self.step(true)),
                KeyCode::Home => Some(0),
                KeyCode::End => Some(self.labels.len() - 1),
                _ => return Handled::No,
            },
            _ => return Handled::No,
        };

        match chosen {
            Some(chosen) => self.select(chosen, ctx),
            None => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// An empty strip is not a tab stop: there is nothing for a key to do.
    fn focusable(&self) -> bool {
        !self.labels.is_empty()
    }
}

impl<M> Describe for Tabs<M> {
    const KIND: &'static str = "tabs";
    const DOC: &'static str = "A row of labels where one is selected, for switching what is below.";
    const GROUP: Group = Group::Container;
    const ICON: &'static denise::icon::Icon = &super::icons::TABS;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "tab",
            PropertyKind::List,
            "The section names, as `tab` child nodes. Real data: a form's sections are the form's. A `tab` that carries children carries that section's page with it.",
        ),
        Property::new(
            "selected",
            PropertyKind::Int {
                min: 0,
                max: i32::MAX,
            },
            "Index of the selected tab. A strip with tabs always has one, so this is never unset.",
        ),
        Property::new(
            "on-change",
            PropertyKind::Message(Payload::Index),
            "Emitted with the newly selected tab's index.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role of the selected tab's underline, and only that.",
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
            "selected" => Value::Int(i32::try_from(self.selected).unwrap_or(i32::MAX)),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, which clamps into the labels: a tab strip has
            // no way to show nothing selected.
            "selected" => self.set_selected(value.as_index()?),
            // The engine builds these from the child nodes, and an
            // inspector edits them where they live. See
            // `PropertyKind::List`.
            "on-change" | "tab" => return Err(Mismatch::Supplied),
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
    use denise::Theme;
    use denise::theme;

    fn tabs() -> Tabs<usize> {
        Tabs::new(["Oversikt", "Alarmer", "Innstillinger"], |index| index)
    }

    /// Tabs are laid end to end from the leading edge, each its own width — not
    /// an equal share of the strip, which is what would make a short label sit
    /// in a wide empty box.
    #[test]
    fn tabs_are_laid_end_to_end_at_their_own_widths() {
        let bounds = Rect::new(10, 20, 300, 40);
        let placed = place(bounds, &[60, 90, 40]);

        assert_eq!(placed[0].x, bounds.x);
        for pair in placed.windows(2) {
            assert_eq!(pair[1].x, pair[0].right(), "a gap or an overlap");
        }
        assert_eq!(placed.last().expect("a tab").right(), bounds.x + 190);
        for tab in &placed {
            assert_eq!(tab.y, bounds.y);
            assert_eq!(tab.height, bounds.height);
        }
    }

    /// A strip narrower than its tabs still places them all. The canvas clips
    /// what runs off the end, and hit testing and drawing agree about where a
    /// tab is even when it is not visible.
    #[test]
    fn tabs_wider_than_the_strip_are_still_placed() {
        let bounds = Rect::new(0, 0, 100, 40);
        let placed = place(bounds, &[60, 90, 40]);
        assert_eq!(placed.len(), 3);
        assert!(
            placed[2].x > bounds.right(),
            "the last tab should be past the edge"
        );
    }

    /// Every point in the strip belongs to the tab that contains it, and points
    /// outside belong to none — including the gap past the last tab, which is
    /// strip but not tab.
    #[test]
    fn a_point_lands_in_the_tab_that_contains_it() {
        let bounds = Rect::new(10, 20, 300, 40);
        let widths = [60, 90, 40];
        let placed = place(bounds, &widths);

        assert_eq!(hit(bounds, &placed, Point::new(11, 30)), Some(0));
        assert_eq!(hit(bounds, &placed, Point::new(69, 30)), Some(0));
        assert_eq!(hit(bounds, &placed, Point::new(70, 30)), Some(1));
        assert_eq!(hit(bounds, &placed, Point::new(199, 30)), Some(2));
        assert_eq!(
            hit(bounds, &placed, Point::new(200, 30)),
            None,
            "the right edge is exclusive: 160..200 ends at 199"
        );
        assert_eq!(
            hit(bounds, &placed, Point::new(280, 30)),
            None,
            "and past the last tab is strip, not tab"
        );
        assert_eq!(hit(bounds, &placed, Point::new(5, 30)), None, "left of it");
        assert_eq!(hit(bounds, &placed, Point::new(100, 5)), None, "above it");
    }

    /// Wrapping in both directions, and the ends.
    #[test]
    fn the_selection_wraps_and_the_ends_are_reachable() {
        let mut tabs = tabs();
        assert_eq!(tabs.step(true), 1);
        tabs.set_selected(2);
        assert_eq!(tabs.step(true), 0, "past the end comes back to the start");
        tabs.set_selected(0);
        assert_eq!(tabs.step(false), 2, "and before the start goes to the end");
    }

    /// A one-tab strip steps to itself rather than dividing by nothing.
    #[test]
    fn a_single_tab_strip_steps_to_itself() {
        let tabs: Tabs<usize> = Tabs::new(["Bare én"], |index| index);
        assert_eq!(tabs.step(true), 0);
        assert_eq!(tabs.step(false), 0);
    }

    /// An empty strip is inert and is not a tab stop.
    #[test]
    fn an_empty_strip_is_inert_rather_than_broken() {
        let mut tabs: Tabs<usize> = Tabs::inert(Vec::<String>::new());
        assert_eq!(tabs.selected(), 0);
        assert_eq!(tabs.selected_label(), None);
        assert_eq!(tabs.step(true), 0);
        assert!(!Widget::<usize>::focusable(&tabs));
        tabs.set_selected(9);
        assert_eq!(tabs.selected(), 0);
        assert!(place(Rect::new(0, 0, 100, 40), &[]).is_empty());
    }

    /// The selection is always a tab that exists, including after the labels
    /// change under it.
    #[test]
    fn the_selection_survives_the_labels_changing() {
        let mut tabs = tabs();
        tabs.set_selected(2);
        assert_eq!(tabs.selected_label(), Some("Innstillinger"));
        tabs.set_labels(["Bare én"]);
        assert_eq!(tabs.selected(), 0);
        assert_eq!(tabs.selected_label(), Some("Bare én"));
    }

    /// The preferred width is every tab at its natural size, which is what a
    /// caller needs to know before it can size the strip.
    #[test]
    fn the_preferred_width_is_the_sum_of_the_tabs() {
        let mut engine = TextEngine::new();
        let tabs = tabs();
        let widths = tabs.widths(&mut engine);
        assert_eq!(widths.len(), 3);
        assert_eq!(
            tabs.preferred_width(&mut engine),
            widths.iter().sum::<i32>()
        );
        assert!(
            widths[2] > widths[1],
            "a longer label should make a wider tab"
        );
    }

    /// A close button is room at the end of every tab, whether or not it is
    /// showing: a tab that grew when the pointer reached it would push the row
    /// along under the pointer.
    #[test]
    fn close_buttons_widen_every_tab_by_the_same_amount() {
        let mut engine = TextEngine::new();
        let plain = tabs().widths(&mut engine);
        let closable = tabs().with_close_buttons(true).widths(&mut engine);
        for (plain, closable) in plain.iter().zip(&closable) {
            assert_eq!(closable - plain, close_size(16));
        }
    }

    /// The close button sits inside its tab, after the label's padding starts,
    /// and above the rule.
    #[test]
    fn the_close_button_sits_inside_the_end_of_its_tab() {
        let band = Rect::new(0, 0, 400, 40);
        let tab = Rect::new(100, 0, 120, 40);
        let close = close_rect(16, tab, band);
        assert!(close.x > tab.x && close.right() < tab.right());
        assert!(close.bottom() <= band.bottom() - rule_thickness(band));
        assert!(close.y >= tab.y);
    }

    /// A row wider than the strip slides so the selected tab is in it — and not
    /// at all while it already is.
    #[test]
    fn the_selected_tab_is_slid_into_view() {
        let band = Rect::new(0, 0, 100, 40);
        let placed = place(band, &[60, 90, 40]);
        assert_eq!(reveal_shift(band, &placed, 0), 0, "already visible");
        let shift = reveal_shift(band, &placed, 2);
        assert_eq!(
            placed[2].right() - shift,
            band.right(),
            "its end at the edge"
        );

        // Wider than the strip on its own: its leading edge shows.
        let placed = place(band, &[60, 300]);
        assert_eq!(placed[1].x - reveal_shift(band, &placed, 1), band.x);
    }

    /// A dragged tab passes a neighbour once its centre crosses the
    /// neighbour's, and does not trade straight back — even when the two are
    /// different widths, which is where trading on edges flickers.
    #[test]
    fn a_dragged_tab_passes_its_neighbours_at_their_centres_and_stays_passed() {
        let band = Rect::new(0, 0, 400, 40);
        for widths in [[40, 120, 60], [120, 40, 60]] {
            let placed = place(band, &widths);
            let neighbour = placed[1].x + placed[1].width / 2;
            assert_eq!(drag_target(&placed, 0, neighbour), None, "on the centre");
            assert_eq!(drag_target(&placed, 0, neighbour + 1), Some(1));

            // After the move, the same centre asks for no move back.
            let moved = place(band, &[widths[1], widths[0], widths[2]]);
            assert_eq!(drag_target(&moved, 1, neighbour + 1), None, "{widths:?}");
        }
        let placed = place(band, &[40, 40, 40, 40]);
        assert_eq!(drag_target(&placed, 0, 150), Some(3), "several at once");
        assert_eq!(drag_target(&placed, 3, 10), Some(0), "and back");
    }

    /// The selection follows the tab it was on through a move.
    #[test]
    fn moving_a_tab_takes_its_colour_and_the_selection_with_it() {
        let red = Color::rgb(220, 50, 50);
        let mut tabs = tabs().with_colors([Some(red)]);
        tabs.set_selected(1);
        tabs.move_tab(0, 2);
        assert_eq!(tabs.labels(), ["Alarmer", "Innstillinger", "Oversikt"]);
        assert_eq!(tabs.colors(), [None, None, Some(red)]);
        assert_eq!(tabs.selected_label(), Some("Alarmer"));

        for (from, to) in [(0, 2), (2, 0), (1, 1), (0, 9)] {
            let mut tabs = tabs.clone();
            let before = tabs.selected_label().map(String::from);
            tabs.move_tab(from, to);
            assert_eq!(tabs.selected_label().map(String::from), before);
        }
    }

    /// Colours stay one to a tab whatever the labels do.
    #[test]
    fn there_is_one_colour_per_tab() {
        let blue = Color::rgb(50, 90, 220);
        let mut tabs = tabs().with_colors([Some(blue); 5]);
        assert_eq!(tabs.colors().len(), 3, "cut to the tabs");
        tabs.set_labels(["En", "To", "Tre", "Fire"]);
        assert_eq!(tabs.colors(), [Some(blue), Some(blue), Some(blue), None]);
        tabs.set_color(9, Some(blue));
        tabs.set_color(3, Some(blue));
        assert_eq!(tabs.colors()[3], Some(blue));
    }

    /// Both label colours have to be readable on the panel, in every theme, in
    /// every state. This is what rejected drawing the selected label in the role
    /// colour: `Secondary` on the light theme is 2.34:1 against `Base100`, which
    /// is a tab nobody can read, and it fails in exactly one of the three themes.
    #[test]
    fn both_label_colours_are_readable_on_the_panel_in_every_theme() {
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for state in [
                VisualState::NONE,
                VisualState::HOVERED,
                VisualState::FOCUSED,
                VisualState::DISABLED,
            ] {
                let (surface, selected, resting) = label_colors(&theme, state);
                for (which, colour) in [("selected", selected), ("unselected", resting)] {
                    let ratio = contrast_x100(surface, colour);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {state:?} {which}: label on the panel is {ratio}, floor \
                         is {AA_LARGE}",
                        theme.name
                    );
                }
            }
        }
    }

    /// A colour is the caller's, so it promises nothing about what reads on it.
    /// The labels on a coloured tab are readable anyway, for colours from both
    /// ends and the treacherous middle, in every theme.
    #[test]
    fn labels_are_readable_on_a_tab_of_any_colour() {
        use denise::theme::AA_LARGE;

        let colours = [
            Color::rgb(229, 72, 77),
            Color::rgb(247, 144, 9),
            Color::rgb(245, 208, 0),
            Color::rgb(48, 164, 108),
            Color::rgb(18, 165, 148),
            Color::rgb(62, 99, 221),
            Color::rgb(142, 78, 198),
            Color::rgb(214, 64, 159),
            Color::rgb(128, 128, 128),
            Color::WHITE,
            Color::BLACK,
        ];
        for theme in Theme::BUILT_IN {
            let (surface, content, _) = label_colors(&theme, VisualState::NONE);
            for colour in colours {
                let tint = tinted(surface, colour);
                let (selected, resting) = labels_on(tint, content);
                for (which, label) in [("selected", selected), ("resting", resting)] {
                    let ratio = contrast_x100(tint, label);
                    assert!(
                        ratio >= AA_LARGE,
                        "{} {colour:?} {which}: {ratio}",
                        theme.name
                    );
                }
            }
        }
    }

    /// The mute has to be visible, or the selected tab is marked only by its
    /// underline and the labels all look the same.
    #[test]
    fn the_muted_label_is_actually_different_from_the_selected_one() {
        for theme in Theme::BUILT_IN {
            let (_, selected, resting) = label_colors(&theme, VisualState::NONE);
            assert_ne!(resting, selected, "{}", theme.name);
        }
    }

    /// The exception the rule needs: a disabled strip does not mute, because the
    /// colour it would mute was already derived to sit exactly on the floor.
    #[test]
    fn a_disabled_strip_does_not_mute_a_colour_that_has_nothing_left_to_give() {
        for theme in Theme::BUILT_IN {
            let (_, selected, resting) = label_colors(&theme, VisualState::DISABLED);
            assert_eq!(
                resting, selected,
                "{}: a disabled label was muted below its own floor",
                theme.name
            );
        }
    }

    /// Padding never collapses, however small the font.
    #[test]
    fn padding_survives_an_absurdly_small_font() {
        assert!(padding(0) >= 8);
        assert!(padding(6) >= 8);
        assert_eq!(padding(16), 16);
        let _ = theme::DARK;
    }
}
