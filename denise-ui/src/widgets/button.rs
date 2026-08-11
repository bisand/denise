//! A pressable, focusable, message-emitting rectangle.

use alloc::string::String;

use denise::{ElementState, InputEvent, KeyCode, Radius, Role};
use denise_render::Canvas;
use denise_render::font;

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
    scale: i32,
}

impl<M> Button<M> {
    /// A primary button carrying `message`.
    pub fn new(label: impl Into<String>, message: M) -> Self {
        Self {
            label: label.into(),
            message: Some(message),
            role: Role::Primary,
            radius: Radius::Field,
            scale: 2,
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
            scale: 2,
        }
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

    /// Sets the integer glyph scale.
    pub fn with_scale(mut self, scale: i32) -> Self {
        self.scale = scale.max(1);
        self
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

    /// Width this button needs for its label plus comfortable padding.
    pub fn preferred_width(&self) -> i32 {
        font::BUILT_IN.line_width(&self.label, self.scale) + font::ADVANCE * self.scale * 2
    }
}

impl<M: Clone + 'static> Widget<M> for Button<M> {
    fn paint(&self, ctx: &PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let radius = ctx.theme.radius(self.radius);
        let (background, content) = interactive_pair(ctx.theme, self.role, ctx.state);
        canvas.fill_rounded_rect(ctx.bounds, radius, background);
        if ctx.state.contains(VisualState::FOCUSED) {
            focus_ring(ctx.theme, ctx.bounds, radius, canvas);
        }
        draw_aligned(
            canvas,
            ctx.bounds,
            Align::Center,
            Align::Center,
            self.scale,
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
        true
    }
}
