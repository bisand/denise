//! The closed half of a dropdown, and the four lines that open it.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role};
use denise_render::Canvas;
use denise_text::TextStyle;

use crate::widget::{Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair, muted};

/// A control showing one chosen option, which asks to be opened.
///
/// ```
/// # use denise_ui::Select;
/// enum Message { Open, Chose(usize) }
/// Select::new(["Auto", "Manuell", "Av"], Message::Open).with_placeholder("Velg modus");
/// ```
///
/// # It does not open its own list, and nothing here can
///
/// To open a list a widget would have to create nodes from inside `on_event`.
/// [`EventCtx`] can emit a message, ask for focus, ask for frames and ask to be
/// revealed — it cannot add widgets, and giving it that power would let any
/// widget restructure the tree from an event handler.
///
/// That is the same line already drawn three times: [`Tabs`](super::Tabs) owns
/// the selected index and not the pages, [`List`](super::List) owns the
/// selection and not the viewport, and
/// [`Ui::push_popup`](crate::Ui::push_popup) places a container the caller
/// fills. A select that owned its list would be the first widget to own nodes,
/// and the exception would be permanent.
///
/// So this widget emits an *open* message and the application opens the list —
/// which [`open_select`] does in one call:
///
/// ```
/// # use denise::{Size, theme};
/// # use denise_ui::{Select, Ui, widgets};
/// # #[derive(Clone, Debug)] enum Message { Open, Chose(usize) }
/// # fn demo(message: Message, select: denise_ui::NodeId) {
/// # let mut ui: Ui<Message> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// match message {
///     Message::Open => { widgets::open_select(&mut ui, select, Message::Chose); }
///     Message::Chose(index) => {
///         ui.close_popup();
///         ui.widget_mut::<Select<Message>>(select).unwrap().set_selected(Some(index));
///     }
/// }
/// # }
/// ```
///
/// Everything the open list needs is already there: the popup flips near a
/// screen edge, closes on Escape or a press outside — swallowing that press —
/// and returns focus here when it goes.
///
/// # Keyboard
///
/// `Enter`, `Space` and `ArrowDown` open it. Left and Right deliberately do
/// **not** cycle the value: a select whose value changes as somebody tabs past
/// it is the classic accidental-edit bug, and a closed control that quietly
/// edits itself is worse than one that needs a second keystroke.
#[derive(Clone, Debug)]
pub struct Select<M> {
    options: Vec<String>,
    selected: Option<usize>,
    placeholder: String,
    message: Option<M>,
    role: Role,
    style: TextStyle,
}

impl<M> Select<M> {
    /// A select with nothing chosen, emitting `message` when it wants opening.
    pub fn new(options: impl IntoIterator<Item = impl Into<String>>, message: M) -> Self {
        Self {
            options: options.into_iter().map(Into::into).collect(),
            selected: None,
            placeholder: String::from("—"),
            message: Some(message),
            role: Role::Base100,
            style: TextStyle::built_in(16),
        }
    }

    /// Sets the text shown when nothing is chosen.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the initially chosen option. Out of range chooses nothing.
    pub fn with_selected(mut self, index: Option<usize>) -> Self {
        self.set_selected(index);
        self
    }

    /// Sets the control's surface role.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The chosen index, if any.
    #[inline]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The chosen option's text, or `None` when nothing is chosen.
    #[inline]
    pub fn selected_option(&self) -> Option<&str> {
        self.options.get(self.selected?).map(String::as_str)
    }

    /// Chooses an option **without emitting anything**. Out of range chooses
    /// nothing, as [`List::set_selected`](super::List::set_selected) does and
    /// for the same reason: nothing chosen is a state this control can show.
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| *index < self.options.len());
    }

    /// The options, in order.
    #[inline]
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Replaces the options, dropping a selection that no longer exists.
    pub fn set_options(&mut self, options: impl IntoIterator<Item = impl Into<String>>) {
        self.options = options.into_iter().map(Into::into).collect();
        self.set_selected(self.selected);
    }

    /// Replaces the font and size.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// The font and size the text draws in.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// What the control currently reads.
    fn shown(&self) -> &str {
        self.selected_option().unwrap_or(&self.placeholder)
    }
}

/// Space between the text and the control's edge.
#[inline]
const fn padding(size_px: u16) -> i32 {
    let half = size_px as i32 / 2;
    if half < 4 { 4 } else { half }
}

/// The chevron's box: a square at the trailing edge, inset by the padding.
fn chevron_box(bounds: Rect, pad: i32) -> Rect {
    let side = (bounds.height / 3).clamp(1, bounds.width.max(1));
    Rect::new(
        bounds.right() - pad - side,
        bounds.y + (bounds.height - side / 2) / 2,
        side,
        side / 2,
    )
}

/// Draws a downward chevron inside `box_of`, as two strokes.
///
/// Two lines rather than a glyph: the built-in font has no arrow, and a control
/// whose affordance depended on which font was loaded would lose it on the
/// panel that ships with none.
fn draw_chevron(canvas: &mut Canvas<'_>, box_of: Rect, thickness: i32, color: denise::Color) {
    if box_of.is_empty() {
        return;
    }
    let tip = Point::new(box_of.x + box_of.width / 2, box_of.bottom());
    let left = Point::new(box_of.x, box_of.y);
    let right = Point::new(box_of.right(), box_of.y);
    for offset in 0..thickness.max(1) {
        let dy = offset;
        canvas.draw_line(
            Point::new(left.x, left.y + dy),
            Point::new(tip.x, tip.y + dy),
            color,
        );
        canvas.draw_line(
            Point::new(tip.x, tip.y + dy),
            Point::new(right.x, right.y + dy),
            color,
        );
    }
}

impl<M: Clone + 'static> Widget<M> for Select<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        let radius = ctx.theme.radius(Radius::Field);
        let (surface, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(bounds, radius, surface);
        canvas.stroke_rounded_rect(
            bounds,
            radius,
            ctx.theme.metrics.border,
            ctx.theme.color(Role::Base300),
        );
        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, bounds, radius, canvas);
        }

        let pad = padding(self.style.size_px);
        let chevron = chevron_box(bounds, pad);
        draw_chevron(canvas, chevron, ctx.theme.metrics.border, content);

        // A placeholder is de-emphasised, a chosen value is not — the same
        // `muted` every de-emphasised label here goes through, so a pair with
        // no contrast to spare is returned unchanged rather than made
        // unreadable.
        let colour = if self.selected.is_some() {
            content
        } else {
            muted(surface, content)
        };
        let text = Rect::from_edges(
            bounds.x + pad,
            bounds.y,
            (chevron.x - pad).max(bounds.x + pad),
            bounds.bottom(),
        );
        if !text.is_empty() {
            draw_aligned(
                canvas,
                ctx.text,
                self.style,
                text,
                (Align::Start, Align::Center),
                self.shown(),
                colour,
            );
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let opened = match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            })
            | Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => ctx.bounds.contains(*position),
            // Enter, Space and Down open. Left and Right are deliberately not
            // handled: a closed select that edited its own value as somebody
            // tabbed past it is the classic accidental-edit bug.
            Event::Input(InputEvent::Key {
                code: KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::ArrowDown,
                state: ElementState::Down,
                repeat: false,
                ..
            }) => ctx.state.contains(VisualState::FOCUSED),
            _ => return Handled::No,
        };
        if !opened || self.options.is_empty() {
            return Handled::No;
        }
        if let Some(message) = self.message.clone() {
            ctx.emit(message);
        }
        Handled::Yes
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// A select with no options is not a tab stop: there is nothing to open.
    fn focusable(&self) -> bool {
        !self.options.is_empty()
    }
}

impl<M> Describe for Select<M> {
    const KIND: &'static str = "select";

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "selected",
            PropertyKind::Int {
                min: 0,
                max: i32::MAX,
            },
            "The chosen option. Without one, nothing is chosen and the placeholder shows.",
        ),
        Property::new(
            "placeholder",
            PropertyKind::Text,
            "Shown while nothing is chosen.",
        ),
        Property::new(
            "on-change",
            // The exception to the payload table: a `Select` holds one message
            // and the application reads `selected()` when it arrives, because a
            // dropdown's choice outlives the event that made it.
            PropertyKind::Message(Payload::None),
            "Emitted when a choice is made; the application reads `selected` afterwards.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role of the control's own surface.",
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
            // Nothing chosen reports nothing, which is the state the format
            // spells by leaving `selected` out.
            "selected" => Value::Int(i32::try_from(self.selected?).unwrap_or(i32::MAX)),
            "placeholder" => Value::text(self.placeholder.as_str()),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            // Through the setter, so an option that is not there chooses
            // nothing rather than leaving a dangling index behind.
            "selected" => self.set_selected(Some(value.as_index()?)),
            "placeholder" => self.placeholder = value.as_text()?,
            "on-change" => return Err(Mismatch::Supplied),
            "role" => self.role = value.as_role()?,
            "size" => self.style.size_px = value.as_size()?,
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

/// Opens a [`Select`]'s option list as a popup below it.
///
/// The four lines an application would otherwise write, and the ones several
/// panels would each get subtly wrong: sizing the popup to the widest option,
/// matching its width to the control so the open list lines up with the closed
/// one, and seeding the list's selection from the select's.
///
/// The popup is an ordinary one — [`Ui::push_popup`](crate::Ui::push_popup) —
/// so it flips near a screen edge, closes on Escape or a press outside
/// (swallowing that press), and returns focus to the select. Its contents are
/// ordinary nodes: a caller who wants a different open list writes those four
/// lines instead of calling this.
///
/// message` is emitted when a row is **chosen** — by `Enter` or by a tap — and
/// not while the arrow keys move the highlight through the list. That is the
/// distinction [`List`](super::List) draws between selecting and activating,
/// and a dropdown wants only the second: a list that reported every row the
/// keyboard passed over would have an application applying three values on the
/// way to the fourth.
///
/// The application closes the popup and applies the choice; this does not,
/// because choosing is the application's business and a helper that closed the
/// popup would be deciding when a multi-select was finished.
///
/// Returns the popup's container, or `None` if `select` is not a live node
/// holding a `Select`.
pub fn open_select<M: Clone + 'static>(
    ui: &mut crate::Ui<M>,
    select: crate::NodeId,
    message: fn(usize) -> M,
) -> Option<crate::NodeId> {
    let widget = ui.widget::<Select<M>>(select)?;
    let options: Vec<String> = widget.options().to_vec();
    if options.is_empty() {
        return None;
    }
    let style = widget.style();
    let chosen = widget.selected();

    let anchor = ui.bounds(select)?;
    let row = ui.theme().metrics.size_field;
    let widest = options
        .iter()
        .map(|option| ui.text_mut().measure_line(style, option))
        .max()
        .unwrap_or(0);

    // As wide as the control, or as wide as the options need — whichever is
    // more. A list narrower than the thing it drops out of looks detached.
    let pad = padding(style.size_px);
    let width = anchor.width.max(widest + pad * 2);
    let height = row * options.len() as i32;

    let container = ui.push_popup(
        select,
        denise::Size::new(width as u32, height as u32),
        crate::Side::Below,
    )?;
    ui.add(
        container,
        super::Panel::default(),
        Rect::new(0, 0, width, height),
    )?;
    // Inert for selection, wired for activation: the arrows move the highlight
    // silently and only Enter or a tap reports a choice. `activate_on_click`
    // makes one tap do both, which is what a dropdown row is — a command, not
    // an option to be pondered.
    let list = super::List::inert(options)
        .on_activate(message)
        .with_row_height(row)
        .with_style(style)
        .activate_on_click()
        .with_selected(chosen);
    let list = ui.add(container, list, Rect::new(0, 0, width, height))?;
    // So the keyboard works the moment it opens, and Escape has somewhere to
    // return focus from.
    ui.focus(Some(list));
    Some(container)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select() -> Select<u8> {
        Select::new(["Auto", "Manuell", "Av"], 1u8)
    }

    /// Nothing chosen shows the placeholder; a choice shows the option.
    #[test]
    fn it_shows_the_placeholder_until_something_is_chosen() {
        let mut select = select().with_placeholder("Velg modus");
        assert_eq!(select.shown(), "Velg modus");
        assert_eq!(select.selected(), None);

        select.set_selected(Some(1));
        assert_eq!(select.shown(), "Manuell");
        assert_eq!(select.selected_option(), Some("Manuell"));
    }

    /// Out of range chooses nothing rather than the nearest option — nothing
    /// chosen is a state this control can show, so it is the honest answer.
    #[test]
    fn an_index_that_does_not_exist_chooses_nothing() {
        let mut select = select();
        select.set_selected(Some(9));
        assert_eq!(select.selected(), None);

        select.set_selected(Some(2));
        select.set_options(["Bare én"]);
        assert_eq!(select.selected(), None, "a shorter list drops it");
    }

    /// A select with no options is not a tab stop: there is nothing to open.
    #[test]
    fn an_empty_select_is_not_a_tab_stop() {
        let empty: Select<u8> = Select::new(Vec::<String>::new(), 1u8);
        assert!(!Widget::<u8>::focusable(&empty));
        assert!(Widget::<u8>::focusable(&select()));
    }

    /// The chevron stays inside the control at every size, and never inverts.
    #[test]
    fn the_chevron_stays_inside_the_control() {
        for bounds in [
            Rect::new(0, 0, 200, 36),
            Rect::new(10, 10, 40, 20),
            Rect::new(0, 0, 8, 8),
            Rect::new(0, 0, 1, 1),
        ] {
            let box_of = chevron_box(bounds, 8);
            assert!(box_of.width >= 0 && box_of.height >= 0, "{bounds:?}");
            assert!(
                box_of.right() <= bounds.right(),
                "{bounds:?}: chevron {box_of:?} escaped right"
            );
            assert!(
                box_of.y >= bounds.y && box_of.bottom() <= bounds.bottom(),
                "{bounds:?}: chevron {box_of:?} escaped vertically"
            );
        }
    }

    /// The text column stops before the chevron, so a long option is clipped
    /// rather than drawn through the affordance.
    #[test]
    fn the_text_column_stops_before_the_chevron() {
        let bounds = Rect::new(0, 0, 200, 36);
        let pad = padding(16);
        let chevron = chevron_box(bounds, pad);
        let text = Rect::from_edges(
            bounds.x + pad,
            bounds.y,
            (chevron.x - pad).max(bounds.x + pad),
            bounds.bottom(),
        );
        assert!(text.width > 0);
        assert!(
            text.right() <= chevron.x,
            "the text runs into the chevron: {text:?} {chevron:?}"
        );
    }

    /// The placeholder is de-emphasised and a value is not — and both stay
    /// readable in every theme, through the shared `muted`.
    #[test]
    fn the_placeholder_is_muted_but_still_readable() {
        use denise::Theme;
        use denise::theme::{AA_LARGE, contrast_x100};

        for theme in Theme::BUILT_IN {
            for state in [VisualState::NONE, VisualState::DISABLED] {
                let (surface, content) = interactive_pair(&theme, Role::Base100, state);
                let placeholder = muted(surface, content);
                let ratio = contrast_x100(surface, placeholder);
                assert!(
                    ratio >= AA_LARGE,
                    "{} {state:?}: placeholder is {ratio}, floor is {AA_LARGE}",
                    theme.name
                );
            }
            // Enabled, it is visibly quieter than a chosen value.
            let (surface, content) = interactive_pair(&theme, Role::Base100, VisualState::NONE);
            assert_ne!(muted(surface, content), content, "{}", theme.name);
        }
    }
}
