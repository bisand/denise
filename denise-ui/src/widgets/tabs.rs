//! A row of labels where one is selected.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Rect, Role, Theme};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, interactive_pair, muted};

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
#[derive(Clone, Debug)]
pub struct Tabs<M> {
    labels: Vec<String>,
    selected: usize,
    message: Option<fn(usize) -> M>,
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
}

impl<M> Tabs<M> {
    /// A strip with the first tab selected.
    pub fn new(
        labels: impl IntoIterator<Item = impl Into<String>>,
        message: fn(usize) -> M,
    ) -> Self {
        Self {
            labels: labels.into_iter().map(Into::into).collect(),
            selected: 0,
            message: Some(message),
            role: Role::Primary,
            style: TextStyle::built_in(16),
            over_pages: false,
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

    /// A strip that emits nothing.
    pub fn inert(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            labels: labels.into_iter().map(Into::into).collect(),
            selected: 0,
            message: None,
            role: Role::Primary,
            style: TextStyle::built_in(16),
            over_pages: false,
        }
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
    pub fn set_labels(&mut self, labels: impl IntoIterator<Item = impl Into<String>>) {
        self.labels = labels.into_iter().map(Into::into).collect();
        self.selected = self.clamp(self.selected);
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
        let pad = padding(self.style.size_px);
        self.labels
            .iter()
            .map(|label| engine.measure_line(self.style, label) + pad * 2)
            .collect()
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

/// Space each side of a label.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let value = size_px as i32;
    if value < 8 { 8 } else { value }
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

/// Which tab contains `point`, if any.
fn hit(bounds: Rect, tabs: &[Rect], point: Point) -> Option<usize> {
    if !bounds.contains(point) {
        return None;
    }
    tabs.iter().position(|tab| tab.contains(point))
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

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        // A strip over pages is as tall as the page it shows, and what this
        // widget draws is the band along its top. A strip on its own fills its
        // node, which is what `tabs h=40` has always meant.
        let bounds = self.band(ctx.bounds, ctx.theme);
        if bounds.is_empty() || self.labels.is_empty() {
            return;
        }
        let widths = self.widths(ctx.text);
        let tabs = place(bounds, &widths);

        // A rule under the whole strip, with the selected tab's segment drawn
        // over it. Cheaper than a box per tab, and it reads as a strip rather
        // than as a row of unrelated buttons.
        let thickness = (bounds.height / 10).max(2);
        let rule = Rect::new(
            bounds.x,
            bounds.bottom() - thickness,
            bounds.width,
            thickness,
        );
        canvas.fill_rect(rule, ctx.theme.color(Role::Base300));

        // Neither label colour depends on `self.role`: a role is only guaranteed
        // against *its own* content, not against the surface a label sits on.
        let (_, content, resting) = label_colors(ctx.theme, ctx.state);
        let underline = if ctx.state.contains(VisualState::DISABLED) {
            resting
        } else {
            ctx.theme.color(self.role)
        };

        for (index, tab) in tabs.iter().enumerate() {
            let chosen = index == self.selected;
            if chosen {
                canvas.fill_rect(Rect::new(tab.x, rule.y, tab.width, thickness), underline);
            }
            // The label sits above the rule, not centred in the whole height, or
            // a tall strip puts its text on top of its own underline.
            let text = Rect::new(tab.x, tab.y, tab.width, tab.height - thickness);
            draw_aligned(
                canvas,
                ctx.text,
                self.style,
                text,
                (Align::Center, Align::Center),
                &self.labels[index],
                if chosen { content } else { resting },
            );
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        if self.labels.is_empty() {
            return Handled::No;
        }
        let chosen = match event {
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
                let widths = self.widths(ctx.text);
                let band = self.band(ctx.bounds, ctx.theme);
                hit(band, &place(band, &widths), *position)
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

        let Some(chosen) = chosen else {
            return Handled::No;
        };
        if chosen == self.selected {
            // Nothing changed, so nothing is reported — but the event was still
            // this widget's to handle.
            return Handled::Yes;
        }
        self.selected = chosen;
        if let Some(message) = self.message {
            ctx.emit(message(chosen));
        }
        Handled::Yes
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
