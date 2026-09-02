//! A section that folds to its header, and the controller that makes several
//! of them an accordion.

use alloc::string::String;
use alloc::vec::Vec;

use denise::{ElementState, InputEvent, KeyCode, Point, Radius, Rect, Role, Theme};
use denise_render::Pen;
use denise_text::TextStyle;

use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Widget,
};
use crate::widgets::describe::{
    Describe, DynDescribe, Group, Mismatch, Payload, Property, PropertyKind, ROLES, Value,
};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};
use crate::{NodeId, Ui};

/// How long [`set_open`] takes to fold or unfold, unless the caller says.
pub const FOLD_MS: u64 = 200;

/// A section that folds to its header.
///
/// The widget is the **header** — title, chevron, the toggle — and the node it
/// sits on hosts the body as ordinary children below the header strip, the way
/// [`Panel`](super::Panel) hosts children. Collapsing is the node's *height*
/// animating between the header alone and the full section; the body needs no
/// hiding, because the node's own clip crops it, mid-animation included.
///
/// ```
/// # use denise::{Rect, Size, theme};
/// # use denise_ui::{Ui, widgets::{self, Collapse, FOLD_MS}};
/// # #[derive(Clone, Debug)] enum Msg { Network(bool) }
/// # fn demo(message: Msg) -> Option<()> {
/// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// # let stack = ui.root();
/// # let rect = Rect::new(0, 0, 320, 48);
/// let section = ui.add(stack, Collapse::new("Nettverk", Msg::Network), rect)?;
/// // body children of `section`, placed below Collapse::header_height
/// // ...
/// match message {
///     Msg::Network(open) => widgets::set_open(&mut ui, section, open, FOLD_MS),
/// }
/// # Some(()) }
/// ```
///
/// The widget cannot animate its own node — `EventCtx` deliberately has no
/// tree access — so it emits `fn(bool) -> M` and the application answers with
/// [`set_open`], which flips the chevron and drives
/// [`Ui::animate_layout`](crate::Ui::animate_layout). Inside a
/// [`Ui::set_stack`](crate::Ui::set_stack) the siblings follow every frame,
/// which is the whole accordion mechanism; [`Accordion`] packages the
/// exclusivity.
///
/// # The expanded height is remembered, not configured
///
/// [`set_open`] notes the node's height at the moment of collapse, so opening
/// returns the section to wherever it really was — a section that grew a row
/// while open comes back at its grown height. The one case with nothing to
/// remember is a section *built* collapsed:
/// [`with_expanded_height`](Collapse::with_expanded_height) covers it.
#[derive(Clone, Debug)]
pub struct Collapse<M> {
    title: String,
    open: bool,
    /// Where opening returns to. Written by [`set_open`] on the way down, or
    /// by the builder for a section born collapsed.
    expanded: Option<i32>,
    message: Option<fn(bool) -> M>,
    role: Role,
    style: TextStyle,
}

impl<M> Collapse<M> {
    /// An open section titled `title`, reporting toggles through `message`.
    pub fn new(title: impl Into<String>, message: fn(bool) -> M) -> Self {
        Self {
            title: title.into(),
            open: true,
            expanded: None,
            message: Some(message),
            role: Role::Base200,
            style: TextStyle::built_in(16),
        }
    }

    /// A section that folds and reports nothing, for one the application never
    /// reads the state of.
    ///
    /// **It drives its own height**, which the one built with a message does
    /// not: a message *is* the application saying it will answer with
    /// [`set_open`], and one without has nobody else to. So a decorative
    /// section on a panel folds when pressed and nothing has to be wired to it —
    /// which is what a form file wants, since a form file has no application to
    /// name.
    ///
    /// [`Accordion`] is still the way to make a run of them exclusive; it drives
    /// them through `set_open` and works with these too.
    pub fn inert(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            open: true,
            expanded: None,
            message: None,
            role: Role::Base200,
            style: TextStyle::built_in(16),
        }
    }

    /// Starts the section collapsed. Pair with
    /// [`with_expanded_height`](Collapse::with_expanded_height), since a
    /// section that has never been open has no height to remember.
    pub fn closed(mut self) -> Self {
        self.open = false;
        self
    }

    /// Sets where the first opening goes, for a section built collapsed.
    pub fn with_expanded_height(mut self, height: i32) -> Self {
        self.expanded = Some(height.max(0));
        self
    }

    /// Sets the header's colour role.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the title's font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Whether the section is open (or opening).
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// The header strip's height: the theme's field height.
    ///
    /// The strip the widget draws and the closed height [`set_open`] folds
    /// to — one definition, so they cannot drift.
    pub fn header_height(&self, theme: &Theme) -> i32 {
        theme.metrics.size_field.max(1)
    }

    /// Flips the open state **without emitting or animating** — [`set_open`]
    /// calls this; so can an application restoring saved state before first
    /// paint.
    pub fn set_open_silent(&mut self, open: bool) {
        self.open = open;
    }

    /// The remembered expanded height, if any.
    #[inline]
    pub const fn expanded_height(&self) -> Option<i32> {
        self.expanded
    }

    /// Remembers where opening should return to.
    pub fn set_expanded_height(&mut self, height: i32) {
        self.expanded = Some(height.max(0));
    }
}

/// Opens or folds the section at `id`, animated over `duration_ms`.
///
/// The application's whole answer to a [`Collapse`] message: flips the
/// widget's chevron, remembers the expanded height on the way down, and
/// drives [`Ui::animate_layout`] on the node's height. Does nothing if `id`
/// does not hold a [`Collapse`].
///
/// Opening a section that was built collapsed and never given an expanded
/// height unfolds to the header alone — visibly wrong rather than silently
/// absent, so the missing `with_expanded_height` is found in review.
pub fn set_open<M: 'static>(ui: &mut Ui<M>, id: NodeId, open: bool, duration_ms: u64) {
    let Some(layout) = ui.layout(id) else {
        return;
    };
    let theme = *ui.theme();
    let Some(collapse) = ui.widget_mut::<Collapse<M>>(id) else {
        return;
    };
    let header = collapse.header_height(&theme);
    let target = if open {
        collapse.expanded.unwrap_or(header)
    } else {
        // The height at the moment of folding is where opening returns to.
        collapse.set_expanded_height(layout.height);
        header
    };
    collapse.set_open_silent(open);
    ui.animate_layout(
        id,
        Rect::new(layout.x, layout.y, layout.width, target),
        duration_ms,
    );
}

/// Exclusivity over a run of [`Collapse`] sections: opening one closes the
/// open one.
///
/// A controller the application owns, not a widget — a widget cannot own
/// other nodes, and which sections belong together is application policy.
/// Like [`set_open`], it emits nothing: it *is* the answer to the messages.
///
/// ```
/// # use denise::{Size, theme};
/// # use denise_ui::{Ui, widgets::Accordion};
/// # #[derive(Clone, Debug)] enum Msg { Section(usize, bool) }
/// # fn demo(message: Msg, network: denise_ui::NodeId, screen: denise_ui::NodeId,
/// #         about: denise_ui::NodeId) {
/// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
/// let mut accordion = Accordion::new([network, screen, about]);
/// // ...
/// match message {
///     Msg::Section(index, _) => accordion.toggle(&mut ui, index),
/// }
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Accordion {
    sections: Vec<NodeId>,
    open: Option<usize>,
    duration_ms: u64,
}

impl Accordion {
    /// An accordion over `sections`, all assumed open; the first `toggle`
    /// closes the rest. Call [`collapse_all`](Accordion::collapse_all) after
    /// building to start folded.
    pub fn new(sections: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            sections: sections.into_iter().collect(),
            open: None,
            duration_ms: FOLD_MS,
        }
    }

    /// Sets how long each fold takes.
    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    /// The open section's index, if one is.
    #[inline]
    pub const fn open(&self) -> Option<usize> {
        self.open
    }

    /// Folds every section, leaving nothing open.
    pub fn collapse_all<M: 'static>(&mut self, ui: &mut Ui<M>) {
        for &section in &self.sections {
            set_open(ui, section, false, self.duration_ms);
        }
        self.open = None;
    }

    /// Opens section `index`, folding whichever was open — or folds `index`
    /// itself if it was the open one, leaving the accordion closed.
    ///
    /// Out of range does nothing.
    pub fn toggle<M: 'static>(&mut self, ui: &mut Ui<M>, index: usize) {
        if index >= self.sections.len() {
            return;
        }
        if self.open == Some(index) {
            set_open(ui, self.sections[index], false, self.duration_ms);
            self.open = None;
            return;
        }
        if let Some(current) = self.open {
            set_open(ui, self.sections[current], false, self.duration_ms);
        }
        set_open(ui, self.sections[index], true, self.duration_ms);
        self.open = Some(index);
    }
}

/// The chevron: a small `>` when closed, rotated to `v` when open, drawn as
/// two strokes since the built-in font has no glyph for it.
fn chevron(canvas: &mut Pen<'_>, centre: Point, arm: i32, open: bool, color: denise::Color) {
    let a = arm.max(2);
    if open {
        // Pointing down: two arms meeting at the bottom.
        canvas.draw_line(
            Point::new(centre.x - a, centre.y - a / 2),
            Point::new(centre.x, centre.y + a / 2),
            color,
        );
        canvas.draw_line(
            Point::new(centre.x + a, centre.y - a / 2),
            Point::new(centre.x, centre.y + a / 2),
            color,
        );
    } else {
        // Pointing right: two arms meeting at the right.
        canvas.draw_line(
            Point::new(centre.x - a / 2, centre.y - a),
            Point::new(centre.x + a / 2, centre.y),
            color,
        );
        canvas.draw_line(
            Point::new(centre.x - a / 2, centre.y + a),
            Point::new(centre.x + a / 2, centre.y),
            color,
        );
    }
}

impl<M: 'static> Widget<M> for Collapse<M> {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn measure(&self, ctx: &mut MeasureCtx<'_>, _offered: Offer) -> Measured {
        // Its header, plus what it is holding open. Shut, it is the header —
        // which is what makes a column of these arrange correctly as they fold.
        let header = self.header_height(ctx.theme);
        let body = if self.is_open() {
            self.expanded_height().unwrap_or(0)
        } else {
            0
        };
        Measured::tall(header.saturating_add(body))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let bounds = ctx.bounds;
        if bounds.is_empty() {
            return;
        }
        let header = Rect::new(
            bounds.x,
            bounds.y,
            bounds.width,
            self.header_height(ctx.theme).min(bounds.height),
        );
        let radius = ctx.theme.radius(Radius::Field);
        let (fill, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(header, radius, fill);

        let pad = (self.style.size_px as i32 / 2).max(4);
        let arm = (header.height / 6).max(3);
        chevron(
            canvas,
            Point::new(header.x + pad + arm, header.y + header.height / 2),
            arm,
            self.open,
            content,
        );

        let title_box = Rect::new(
            header.x + pad * 2 + arm * 2,
            header.y,
            (header.width - pad * 3 - arm * 2).max(0),
            header.height,
        );
        if !title_box.is_empty() && !self.title.is_empty() {
            let mut clipped = canvas.with_clip(title_box);
            draw_aligned(
                &mut clipped,
                ctx.text,
                self.style,
                title_box,
                (Align::Start, Align::Center),
                &self.title,
                content,
            );
        }

        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, header, radius, canvas);
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let header = Rect::new(
            ctx.bounds.x,
            ctx.bounds.y,
            ctx.bounds.width,
            self.header_height(ctx.theme).min(ctx.bounds.height),
        );
        let toggle = match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            })
            | Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => header.contains(*position),
            Event::Input(InputEvent::Key {
                code: KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space,
                state: ElementState::Down,
                repeat,
                ..
            }) if ctx.state.contains(VisualState::FOCUSED) => {
                // Space held down must not fold and unfold per repeat, the
                // same guard Checkbox keeps.
                !repeat
            }
            _ => return Handled::No,
        };
        if !toggle {
            return Handled::No;
        }
        // The widget reports; the application animates. `open` flips here so
        // the chevron answers the press immediately, and `set_open` flips it
        // again to the same value — idempotent, not doubled.
        self.open = !self.open;
        match self.message {
            Some(message) => ctx.emit(message(self.open)),
            // Nobody else is going to. `set_open` does exactly this arithmetic
            // with `&mut Ui` in hand; here the current height comes from
            // `ctx.bounds`, which is the same number a moment earlier.
            None => {
                let header = self.header_height(ctx.theme);
                let target = if self.open {
                    self.expanded.unwrap_or(header)
                } else {
                    // The height at the moment of folding is where opening
                    // returns to — see `set_open`.
                    self.expanded = Some(ctx.bounds.height);
                    header
                };
                ctx.resize_height(target, FOLD_MS);
            }
        }
        Handled::Yes
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        // Every other widget here is focusable whether or not it carries a
        // message, because an inert one still does something when pressed. This
        // used to be the exception, and an inert section that folds itself is
        // no longer one.
        true
    }
}

impl<M> Describe for Collapse<M> {
    const KIND: &'static str = "collapse";
    const DOC: &'static str = "A section that folds away to its header and opens again.";
    const GROUP: Group = Group::Container;
    const ICON: &'static denise_render::icon::Icon = &super::icons::COLLAPSE;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "text",
            PropertyKind::Text,
            "The header's title. Named as `button` and `label` name theirs, because a form writes it the same way: as the node's first argument.",
        ),
        Property::new(
            "open",
            PropertyKind::Bool,
            "Whether the section is unfolded.",
        ),
        Property::new(
            "expanded-height",
            PropertyKind::Int { min: 0, max: 4096 },
            "The content's height when open; measured from the children without it.",
        )
        .in_pixels(),
        Property::new(
            "on-toggle",
            PropertyKind::Message(Payload::Bool),
            "Emitted with the new state when the header is pressed. The application answers with `set_open`, which is what actually folds the node.",
        ),
        Property::new(
            "role",
            PropertyKind::Enum(ROLES),
            "Colour role the header strip is filled with.",
        ),
        Property::new(
            "size",
            PropertyKind::Int { min: 6, max: 96 },
            "Title size in logical pixels.",
        )
        .in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "text" => Value::text(self.title.as_str()),
            "open" => Value::Bool(self.open),
            // A section that has never been folded has no remembered height, so
            // there is nothing to report and nothing for a file to write.
            "expanded-height" => Value::Int(self.expanded?),
            "role" => Value::role(self.role),
            "size" => Value::Int(i32::from(self.style.size_px)),
            // The message is the application's own type; see the `describe`
            // module documentation.
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "text" => self.title = value.as_text()?,
            // Silently: `set_open` animates a node's height, and a form being
            // loaded or a designer flipping a checkbox is stating what the
            // section *is*, not folding it in front of anyone. The node's own
            // height comes from the file, which is why the widget only has to
            // agree about the chevron.
            "open" => self.set_open_silent(value.as_bool()?),
            "expanded-height" => self.set_expanded_height(value.as_int()?),
            "role" => self.role = value.as_role()?,
            "size" => self.style.size_px = value.as_size()?,
            "on-toggle" => return Err(Mismatch::Supplied),
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::{ElementState, InputEvent, Modifiers, PointerButton, Size, theme};

    use crate::Ui;

    /// An inert section folds itself, because nothing else is going to.
    ///
    /// A `Collapse` with a message is telling the application it will answer
    /// with [`set_open`]; one without has nobody to tell. Before #118 the widget
    /// only flipped its chevron and the height stayed where it was, which is why
    /// there was no inert constructor to offer.
    #[test]
    fn an_inert_section_folds_and_opens_and_reports_nothing() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Never;

        let mut ui: Ui<Never> = Ui::new(Size::new(400, 300), theme::DARK);
        let root = ui.root();
        let id = ui
            .add(root, Collapse::inert("Avansert"), Rect::new(0, 0, 200, 120))
            .expect("a root takes children");

        let press = |ui: &mut Ui<Never>| {
            let at = Point::new(20, 8);
            ui.handle(&[
                InputEvent::PointerMoved { position: at },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Down,
                    position: at,
                    modifiers: Modifiers::NONE,
                },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Up,
                    position: at,
                    modifiers: Modifiers::NONE,
                },
            ]);
        };
        let settle = |ui: &mut Ui<Never>, from: u64| {
            for step in 0..=4 {
                ui.tick(from + step * FOLD_MS / 2);
            }
        };

        let open_height = ui.layout(id).expect("laid out").height;
        press(&mut ui);
        assert!(
            ui.drain_messages().next().is_none(),
            "an inert section emitted something"
        );
        settle(&mut ui, 0);

        let header = ui
            .widget::<Collapse<Never>>(id)
            .expect("a collapse")
            .header_height(&theme::DARK);
        assert_eq!(
            ui.layout(id).expect("laid out").height,
            header,
            "it did not fold to its header"
        );
        assert!(
            !ui.widget::<Collapse<Never>>(id)
                .expect("a collapse")
                .is_open()
        );

        // And back to exactly where it folded from: the height at the moment of
        // folding is where opening returns to, as `set_open` puts it.
        press(&mut ui);
        settle(&mut ui, 10 * FOLD_MS);
        assert_eq!(
            ui.layout(id).expect("laid out").height,
            open_height,
            "opening it again did not return to the height it folded from"
        );
        assert!(
            ui.widget::<Collapse<Never>>(id)
                .expect("a collapse")
                .is_open()
        );
    }

    /// The one with a message still leaves the height to the application.
    ///
    /// The other half of the rule, and the half that must not have changed: an
    /// accordion refuses folds, animates them at its own duration and closes the
    /// section beside the one that opened. A widget that folded itself anyway
    /// would fight it.
    #[test]
    fn a_section_with_a_message_still_waits_to_be_told() {
        let mut ui: Ui<bool> = Ui::new(Size::new(400, 300), theme::DARK);
        let root = ui.root();
        let id = ui
            .add(
                root,
                Collapse::new("Nettverk", |open| open),
                Rect::new(0, 0, 200, 120),
            )
            .expect("a root takes children");

        let at = Point::new(20, 8);
        ui.handle(&[
            InputEvent::PointerMoved { position: at },
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position: at,
                modifiers: Modifiers::NONE,
            },
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                position: at,
                modifiers: Modifiers::NONE,
            },
        ]);
        assert_eq!(ui.drain_messages().collect::<Vec<_>>(), vec![false]);
        for step in 0..=4 {
            ui.tick(step * FOLD_MS / 2);
        }
        assert_eq!(
            ui.layout(id).expect("laid out").height,
            120,
            "it folded itself instead of waiting for `set_open`"
        );
    }

    #[test]
    fn the_header_height_is_the_folded_height() {
        let c: Collapse<usize> = Collapse::new("Nettverk", |open| open as usize);
        assert_eq!(
            c.header_height(&theme::DARK),
            theme::DARK.metrics.size_field
        );
        assert!(c.is_open());
        assert!(
            !Collapse::<usize>::new("x", |o| o as usize)
                .closed()
                .is_open()
        );
    }

    #[test]
    fn the_expanded_height_floor_is_zero() {
        let c: Collapse<usize> = Collapse::new("x", |o| o as usize).with_expanded_height(-40);
        assert_eq!(c.expanded_height(), Some(0));
    }

    /// A section is a tab stop whether or not it carries a message.
    ///
    /// It was not, until #118: a `Collapse` with no message did nothing when
    /// pressed but flip its chevron, so there was no reason for the keyboard to
    /// stop on it. An inert one folds itself now, which makes it as interactive
    /// as every other inert widget here — `Checkbox`, `Toggle` and `Slider` are
    /// all focusable without a message for the same reason.
    #[test]
    fn a_section_is_a_tab_stop_with_or_without_a_listener() {
        let mut c: Collapse<usize> = Collapse::new("x", |o| o as usize);
        assert!(Widget::<usize>::focusable(&c));
        c.message = None;
        assert!(Widget::<usize>::focusable(&c));
        assert!(Widget::<usize>::focusable(&Collapse::<usize>::inert("x")));
    }
}
