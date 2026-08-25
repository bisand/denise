//! Events in order: a time, a disc, a connector, a label.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use denise::{Point, Rect, Role, Theme};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{Describe, DynDescribe, Mismatch, Property, PropertyKind, Value};
use crate::widgets::style::{Align, draw_aligned, interactive_pair, muted};

/// One event on a [`Timeline`].
///
/// ```
/// # use denise_ui::widgets::TimelineItem;
/// # use denise::theme::Role;
/// TimelineItem::new("Pumpe startet").with_time("12:01").with_role(Role::Success);
/// TimelineItem::new("Ventil åpnes").pending();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineItem {
    text: String,
    time: String,
    role: Role,
    reached: bool,
}

impl TimelineItem {
    /// A reached event carrying `text`, disc in [`Role::Primary`].
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            time: String::new(),
            role: Role::Primary,
            reached: true,
        }
    }

    /// Puts a time — or any short marker — in the column before the disc.
    pub fn with_time(mut self, time: impl Into<String>) -> Self {
        self.time = time.into();
        self
    }

    /// Sets the disc's colour role.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Marks the event as not yet happened: a hollow disc, de-emphasised text.
    pub fn pending(mut self) -> Self {
        self.reached = false;
        self
    }

    /// The label.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The time column's text, empty when there is none.
    #[inline]
    pub fn time(&self) -> &str {
        &self.time
    }

    /// Whether the event has happened.
    #[inline]
    pub const fn is_reached(&self) -> bool {
        self.reached
    }

    /// Marks the event reached or pending.
    pub fn set_reached(&mut self, reached: bool) {
        self.reached = reached;
    }

    /// Replaces the label, reporting whether it changed — for the log that
    /// rewrites its rows every cycle.
    pub fn update(&mut self, text: &str) -> bool {
        let changed = self.text != text;
        if changed {
            self.text = text.to_string();
        }
        changed
    }
}

impl From<&str> for TimelineItem {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

/// A vertical sequence of events: time, disc, connector, label.
///
/// ```text
/// 12:01  ●  Pumpe startet
///        │
/// 12:04  ●  Trykk nådd
///        │
///        ○  Ventil åpnes
/// ```
///
/// A display widget, like [`Label`](super::Label) and
/// [`Image`](super::Image): not interactive, not focusable, invisible to the
/// pointer — a timeline inside a clickable row must not swallow the click.
///
/// # The discs form a straight line, and that is the widget
///
/// The time column is as wide as the widest time in the list, so every disc
/// sits at the same x whatever its row's time says — the alignment several
/// panels would each get subtly wrong, and the same answer
/// [`List`](super::List) gives for its leading column. The connector runs
/// between discs, never past the first or last, so the line reads as the
/// span of the events rather than a stripe down the rectangle.
///
/// # Rows are `List`'s geometry
///
/// A fixed height stacked from the top; what does not fit is not drawn. A
/// long timeline sits inside a
/// [`set_scrollable`](crate::Ui::set_scrollable) viewport, exactly as a long
/// list does — it has no header, so none of [`Table`](super::Table)'s reasons
/// to window its own data apply. [`preferred_height`](Timeline::preferred_height)
/// is offered and never called by the tree.
#[derive(Clone, Debug)]
pub struct Timeline {
    items: Vec<TimelineItem>,
    row_height: Option<i32>,
    style: TextStyle,
}

impl Timeline {
    /// A timeline of `items`, in the order they happened.
    pub fn new(items: impl IntoIterator<Item = impl Into<TimelineItem>>) -> Self {
        Self {
            items: items.into_iter().map(Into::into).collect(),
            row_height: None,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the height of every row, overriding the theme's field height.
    pub fn with_row_height(mut self, height: i32) -> Self {
        self.row_height = Some(height.max(1));
        self
    }

    /// Sets the font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The events, in order.
    #[inline]
    pub fn items(&self) -> &[TimelineItem] {
        &self.items
    }

    /// One event, for updating in place through
    /// [`Ui::widget_mut`](crate::Ui::widget_mut).
    pub fn item_mut(&mut self, index: usize) -> Option<&mut TimelineItem> {
        self.items.get_mut(index)
    }

    /// Replaces every event.
    pub fn set_items(&mut self, items: impl IntoIterator<Item = impl Into<TimelineItem>>) {
        self.items = items.into_iter().map(Into::into).collect();
    }

    /// Appends one event, reporting its index.
    pub fn push(&mut self, item: impl Into<TimelineItem>) -> usize {
        self.items.push(item.into());
        self.items.len() - 1
    }

    /// Height every row is drawn at.
    pub fn row_height(&self, theme: &Theme) -> i32 {
        self.row_height.unwrap_or(theme.metrics.size_field).max(1)
    }

    /// Height this timeline needs to show every event.
    pub fn preferred_height(&self, theme: &Theme) -> i32 {
        let rows = self.items.len().max(1) as i64;
        (i64::from(self.row_height(theme)) * rows).min(i64::from(i32::MAX)) as i32
    }

    /// Width of the time column: the widest time in the list, so the discs
    /// line up whatever each row's time says.
    fn time_width(&self, engine: &mut TextEngine) -> i32 {
        self.items
            .iter()
            .map(|item| engine.measure_line(self.style, &item.time))
            .max()
            .unwrap_or(0)
    }
}

/// Space between columns, scaled with the text.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let half = size_px as i32 / 2;
    if half < 4 { 4 } else { half }
}

/// The disc's radius for a row of `height`: a fifth, floored at 3 so a hollow
/// disc still has a visible hole at small sizes.
#[inline]
fn disc_radius(row_height: i32) -> i32 {
    (row_height / 5).max(3)
}

impl<M: 'static> Widget<M> for Timeline {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() || self.items.is_empty() {
            return;
        }
        let row_height = self.row_height(ctx.theme);
        let pad = padding(self.style.size_px);
        let time_width = self.time_width(ctx.text);
        let radius = disc_radius(row_height);

        // Columns: time, disc, label. The disc column's centre is fixed by the
        // widest time, which is what makes the line of discs straight.
        let time_x = bounds.x;
        let disc_x = bounds.x + time_width + if time_width > 0 { pad } else { 0 } + radius;
        let text_x = disc_x + radius + pad;

        // Text on the surface behind the widget: the timeline paints no
        // backdrop of its own, so it takes Base100's guaranteed pairing, like
        // RadialProgress's label.
        let (surface, content) = interactive_pair(ctx.theme, Role::Base100, ctx.state);
        let line_color = ctx.theme.color(Role::Base300);

        for (index, item) in self.items.iter().enumerate() {
            let row = Rect::new(
                bounds.x,
                bounds.y + row_height * index as i32,
                bounds.width,
                row_height,
            );
            if row.y >= bounds.bottom() {
                break;
            }
            let centre = Point::new(disc_x, row.y + row_height / 2);

            // The connector to the *next* disc, so the line never runs past
            // the last one — the segment belongs to the gap it crosses.
            if index + 1 < self.items.len() && row.bottom() < bounds.bottom() {
                canvas.fill_rect(
                    Rect::new(
                        centre.x - 1,
                        centre.y + radius + 2,
                        2,
                        row_height - 2 * radius - 4,
                    ),
                    line_color,
                );
            }

            // The disc: filled when the event happened, hollow when it is
            // still to come. Both in the item's role, so a pending step in
            // Warning still warns — but the *fill* is what says "done".
            let (disc, _) = interactive_pair(ctx.theme, item.role, ctx.state);
            if item.reached {
                canvas.fill_circle(centre, radius, disc);
            } else {
                canvas.stroke_circle(centre, radius, (radius / 3).max(1), disc);
            }

            // Pending rows are de-emphasised — as far as the pair can afford,
            // which `muted` decides.
            let text_color = if item.reached {
                content
            } else {
                muted(surface, content)
            };

            if !item.time.is_empty() && time_width > 0 {
                let time_box = Rect::new(time_x, row.y, time_width, row_height);
                let mut clipped = canvas.with_clip(time_box);
                draw_aligned(
                    &mut clipped,
                    ctx.text,
                    self.style,
                    time_box,
                    (Align::End, Align::Center),
                    &item.time,
                    text_color,
                );
            }
            if !item.text.is_empty() {
                let text_box =
                    Rect::from_edges(text_x, row.y, bounds.right().max(text_x), row.bottom());
                if !text_box.is_empty() {
                    let mut clipped = canvas.with_clip(text_box);
                    draw_aligned(
                        &mut clipped,
                        ctx.text,
                        self.style,
                        text_box,
                        (Align::Start, Align::Center),
                        &item.text,
                        text_color,
                    );
                }
            }
        }
    }
}

impl Describe for Timeline {
    const KIND: &'static str = "timeline";

    /// A timeline has no colour role of its own: the role belongs to each
    /// event, so a run of them can be `success` up to the one that is still
    /// `warning`.
    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "row-height",
            PropertyKind::Int { min: 16, max: 200 },
            "Height of every event row in logical pixels, overriding the theme's field height.",
        )
        .in_pixels(),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Text size in logical pixels.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            // A timeline taking the theme's row height has nothing to report,
            // so nothing is written back out for it.
            "row-height" => Value::Int(self.row_height?),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "row-height" => self.row_height = Some(value.as_int()?.max(1)),
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

    fn items() -> Vec<TimelineItem> {
        alloc::vec![
            TimelineItem::new("Pumpe startet")
                .with_time("12:01")
                .with_role(Role::Success),
            TimelineItem::new("Trykk nådd").with_time("12:04"),
            TimelineItem::new("Ventil åpnes").pending(),
        ]
    }

    #[test]
    fn a_bare_string_is_a_reached_event() {
        let t = Timeline::new(["Start"]);
        assert_eq!(t.items()[0].text(), "Start");
        assert!(t.items()[0].is_reached());
        assert_eq!(t.items()[0].time(), "");
    }

    /// The height contract is `List`'s: rows times a fixed height.
    #[test]
    fn the_preferred_height_is_rows_times_the_row() {
        let t = Timeline::new(items());
        assert_eq!(
            t.preferred_height(&theme::DARK),
            theme::DARK.metrics.size_field * 3
        );
        assert_eq!(
            Timeline::new(items())
                .with_row_height(40)
                .preferred_height(&theme::DARK),
            120
        );
    }

    /// The disc column is placed by the widest time, so every disc sits at the
    /// same x — the whole point of the widget.
    #[test]
    fn the_time_column_is_the_widest_time() {
        let mut engine = denise_text::TextEngine::new();
        let style = TextStyle::built_in(16);
        let t = Timeline::new(items()).with_style(style);
        let widest = ["12:01", "12:04", ""]
            .iter()
            .map(|s| engine.measure_line(style, s))
            .max()
            .unwrap();
        assert_eq!(t.time_width(&mut engine), widest);

        // No times at all: no column, and nothing reserves space for one.
        let bare = Timeline::new(["a", "b"]);
        assert_eq!(bare.time_width(&mut engine), 0);
    }

    /// An event can be updated in place, and an unchanged write says so.
    #[test]
    fn updating_an_event_reports_whether_it_changed() {
        let mut t = Timeline::new(items());
        assert!(t.item_mut(1).expect("row").update("Trykk tapt"));
        assert!(!t.item_mut(1).expect("row").update("Trykk tapt"));
        t.item_mut(2).expect("row").set_reached(true);
        assert!(t.items()[2].is_reached());
        assert!(t.item_mut(99).is_none());
    }

    /// A timeline is display: not a tab stop, invisible to the pointer.
    #[test]
    fn a_timeline_takes_no_input() {
        let t = Timeline::new(items());
        assert!(!Widget::<usize>::focusable(&t));
        assert!(!Widget::<usize>::accepts_pointer(&t));
    }

    /// A hollow disc must be told apart from a filled one by more than faith:
    /// the stroke thickness leaves a hole of at least a pixel at every size.
    #[test]
    fn a_hollow_disc_keeps_its_hole_at_every_row_height() {
        for row_height in [1, 8, 15, 20, 34, 60, 200] {
            let radius = disc_radius(row_height);
            let thickness = (radius / 3).max(1);
            assert!(radius >= 3, "row {row_height}: disc too small to be hollow");
            assert!(
                thickness < radius,
                "row {row_height}: the stroke fills the disc"
            );
        }
    }

    /// Pending text mutes only as far as the pair can afford — `muted`'s
    /// contract, asserted here against every theme so a pending step is
    /// always still readable.
    #[test]
    fn pending_text_is_readable_in_every_theme() {
        use crate::widget::VisualState;
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            let (surface, content) = interactive_pair(&theme, Role::Base100, VisualState::NONE);
            let pending = muted(surface, content);
            let ratio = contrast_x100(surface, pending);
            assert!(
                ratio >= AA_LARGE,
                "{}: pending text is {ratio}, floor is {AA_LARGE}",
                theme.name
            );
        }
    }
}
