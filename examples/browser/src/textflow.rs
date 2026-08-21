//! The widget HTML text becomes — and the reason the toolkit did not need a
//! rich-text widget of its own.
//!
//! A [`TextFlow`] owns a [`FlowLayout`] the layout pass already measured, so
//! `paint` measures nothing: it walks lines and fragments calling
//! [`TextEngine::draw_line`](denise_text::TextEngine::draw_line) at precomputed baselines. That is the whole
//! division of labour — layout decides once, paint repeats it — and it is
//! why scrolling a page costs blitting, not shaping.
//!
//! Links are fragments whose run carries an index. Their rectangles were
//! collected at construction; hovering one thickens its underline, releasing
//! on one emits [`Message::Navigate`]. Wheel events are refused, so the
//! scroll falls through to the viewport the tree already runs.

use denise::{Color, Point, Rect};
use denise_render::Canvas;
use denise_text::TextStyle;
use denise_ui::{Event, EventCtx, Handled, PaintCtx, Widget};

use crate::app::Message;
use crate::layout::FlowLayout;

pub struct TextFlow {
    flow: FlowLayout,
    /// Widget-local rectangles of every link fragment, with the link index.
    links: Vec<(Rect, usize)>,
    hover: Option<usize>,
    pressed: Option<usize>,
}

impl TextFlow {
    pub fn new(flow: FlowLayout) -> Self {
        let mut links = Vec::new();
        for line in &flow.lines {
            for frag in &line.fragments {
                if let Some(link) = flow.runs[frag.run].link {
                    links.push((Rect::new(frag.x, line.y, frag.width, line.height), link));
                }
            }
        }
        Self {
            flow,
            links,
            hover: None,
            pressed: None,
        }
    }

    fn link_at(&self, bounds: Rect, position: Point) -> Option<usize> {
        let local = Point::new(position.x - bounds.x, position.y - bounds.y);
        self.links
            .iter()
            .find(|(rect, _)| rect.contains(local))
            .map(|&(_, link)| link)
    }
}

impl Widget<Message> for TextFlow {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        for line in &self.flow.lines {
            let baseline = ctx.bounds.y + line.y + line.baseline;
            for frag in &line.fragments {
                let run = &self.flow.runs[frag.run];
                let x = ctx.bounds.x + frag.x;
                ctx.text.draw_line(
                    canvas,
                    run.style,
                    Point::new(x, baseline),
                    &run.text[frag.range.clone()],
                    run.color,
                );
                if run.underline {
                    let hovered = run.link.is_some() && run.link == self.hover;
                    let thickness = if hovered { 2 } else { 1 };
                    canvas.fill_rect(Rect::new(x, baseline + 2, frag.width, thickness), run.color);
                }
            }
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, Message>) -> Handled {
        use denise::{ElementState, InputEvent, PointerButton};
        let Event::Input(input) = event else {
            return Handled::No;
        };
        match input {
            InputEvent::PointerMoved { position } => {
                let hover = self.link_at(ctx.bounds, *position);
                if hover != self.hover {
                    self.hover = hover;
                    ctx.invalidate();
                }
                Handled::No
            }
            InputEvent::PointerLeft => {
                if self.hover.take().is_some() {
                    ctx.invalidate();
                }
                self.pressed = None;
                Handled::No
            }
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state,
                position,
                ..
            } => {
                let link = self.link_at(ctx.bounds, *position);
                match state {
                    ElementState::Down => {
                        self.pressed = link;
                        if link.is_some() {
                            Handled::Yes
                        } else {
                            Handled::No
                        }
                    }
                    ElementState::Up => {
                        // A link follows on release over the same link it was
                        // pressed on — the same forgiveness a button gives.
                        let follow = link.is_some() && link == self.pressed;
                        self.pressed = None;
                        if follow {
                            ctx.emit(Message::Navigate(link.expect("checked")));
                            Handled::Yes
                        } else {
                            Handled::No
                        }
                    }
                }
            }
            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }
}

/// A rectangle of one colour: the page background's widget. `Panel` speaks
/// theme roles, and an author's colour is precisely not one.
pub struct Filler(pub Color);

impl Widget<Message> for Filler {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        canvas.fill_rect(ctx.bounds, self.0);
    }
}

/// A list marker. One line of one style, drawn top-aligned beside the item
/// it counts.
pub struct BulletMark {
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
}

impl Widget<Message> for BulletMark {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Canvas<'_>) {
        let ascent = ctx.text.metrics(self.style).ascent;
        ctx.text.draw_line(
            canvas,
            self.style,
            Point::new(ctx.bounds.x, ctx.bounds.y + ascent),
            &self.text,
            self.color,
        );
    }
}
