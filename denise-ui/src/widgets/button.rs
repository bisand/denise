//! A pressable, focusable, message-emitting rectangle.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Radius, Role};
use denise_render::Canvas;
use denise_text::TextStyle;

use crate::widget::{Event, EventCtx, Handled, PaintCtx, VisualState, Widget};
use crate::widgets::style::{Align, draw_aligned, focus_ring, interactive_pair};

/// A button that emits a message when it is activated.
///
/// Activation is a release *inside* the button, or Enter/Space while it holds
/// focus. A press that is dragged off and released elsewhere is cancelled, which
/// is what makes a touchscreen usable — a finger that lands on the wrong control
/// can be slid away rather than committing.
#[derive(Clone, Debug)]
pub struct Button<M> {
    label: String,
    message: Option<M>,
    role: Role,
    radius: Radius,
    style: TextStyle,
    no_focus: bool,
}

impl<M> Button<M> {
    /// A primary button carrying `message`.
    pub fn new(label: impl Into<String>, message: M) -> Self {
        Self {
            label: label.into(),
            message: Some(message),
            role: Role::Primary,
            radius: Radius::Field,
            style: TextStyle::built_in(16),
            no_focus: false,
        }
    }

    /// A button that emits nothing. Useful as a disabled affordance, or when the
    /// application only cares about focus.
    pub fn inert(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            message: None,
            role: Role::Primary,
            radius: Radius::Field,
            style: TextStyle::built_in(16),
            no_focus: false,
        }
    }

    /// Presses without touching focus: the button takes none, and costs none.
    ///
    /// An ordinary button takes focus when pressed, which is right for a button
    /// somebody tabs to and wrong for a key on an on-screen keyboard — that key
    /// is pressed *while* a field is being typed into, and the field has to keep
    /// the caret. Making the key merely unfocusable is not enough either, since
    /// pressing an unfocusable node is what drops focus and commits a field.
    ///
    /// So this asks for neither. The button still presses, still paints pressed,
    /// still emits its message; Tab skips it, and the focus ring never moves.
    ///
    /// ```
    /// # use denise::{Rect, Size, theme};
    /// # use denise_ui::{Ui, widgets::{Button, TextInput}};
    /// # #[derive(Clone, Debug)] enum Msg { Key(char) }
    /// # let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
    /// # let root = ui.root();
    /// # let field = ui.add(root, TextInput::new(), Rect::new(0, 0, 200, 40)).unwrap();
    /// # ui.focus(Some(field));
    /// ui.add(root, Button::new("q", Msg::Key('q')).no_focus(), Rect::new(0, 100, 40, 40));
    /// assert_eq!(ui.focused(), Some(field));
    /// ```
    pub fn no_focus(mut self) -> Self {
        self.no_focus = true;
        self
    }

    /// Sets the colour role. The content colour comes from the theme's pairing, so
    /// the label stays readable whichever role and theme are chosen.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Sets the corner rounding token.
    pub fn with_radius(mut self, radius: Radius) -> Self {
        self.radius = radius;
        self
    }

    /// Sets the font and size.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the size, keeping the font.
    pub fn with_size(mut self, size_px: u16) -> Self {
        self.style.size_px = size_px;
        self
    }

    /// The font and size the label draws in.
    #[inline]
    pub const fn style(&self) -> TextStyle {
        self.style
    }

    /// The current label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Replaces the label.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Replaces the message emitted on activation.
    pub fn set_message(&mut self, message: Option<M>) {
        self.message = message;
    }

    /// Replaces the colour role.
    ///
    /// What a list of buttons uses to show which one is selected, since a role
    /// survives a theme change and a colour does not.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Replaces the font and size.
    ///
    /// For an application that registers a font after building its tree, which is
    /// the ordinary case: the tree has to exist before anyone knows whether the
    /// font file was there.
    pub fn set_style(&mut self, style: TextStyle) {
        self.style = style;
    }

    /// Width this button needs for its label plus comfortable padding.
    ///
    /// Takes the engine because with a proportional font the answer is not the
    /// character count times anything, and guessing is how a button ends up one
    /// letter too narrow in the language it was not tested in.
    pub fn preferred_width(&self, engine: &mut denise_text::TextEngine) -> i32 {
        let text = engine.measure_line(self.style, &self.label);
        text + i32::from(self.style.size_px) * 3 / 2
    }
}

impl<M: Clone + 'static> Widget<M> for Button<M> {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let radius = ctx.theme.radius(self.radius);
        let (background, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(ctx.bounds, radius, background);
        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, ctx.bounds, radius, canvas);
        }
        draw_aligned(
            canvas,
            ctx.text,
            self.style,
            ctx.bounds,
            (Align::Center, Align::Center),
            &self.label,
            content,
        );
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, M>) -> Handled {
        let activated = match event {
            Event::Input(InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            }) => ctx.bounds.contains(*position),
            Event::Input(InputEvent::TouchUp {
                position,
                cancelled: false,
                ..
            }) => ctx.bounds.contains(*position),
            Event::Input(InputEvent::Key {
                code: KeyCode::Enter | KeyCode::Space | KeyCode::NumpadEnter,
                state: ElementState::Down,
                repeat: false,
                ..
            }) => ctx.state.contains(VisualState::FOCUSED),
            _ => return Handled::No,
        };
        if !activated {
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

    fn focusable(&self) -> bool {
        !self.no_focus
    }

    fn preserves_focus(&self) -> bool {
        self.no_focus
    }
}
