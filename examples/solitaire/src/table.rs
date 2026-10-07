//! The widgets the toolkit does not have: a pile of cards, the cards being
//! carried, and a line of text that keeps its own time.
//!
//! # Why a pile is the widget, and the table is not
//!
//! The obvious design is one widget for the whole table, painting thirteen
//! piles and working out for itself which of them changed. Denise will not let
//! you write that, and it is worth being clear about why, because the refusal
//! is the toolkit's central idea and not an omission.
//!
//! **A widget does not own its damage — the tree does.** A widget is handed a
//! rectangle and paints in it; when it changes, the tree repaints that
//! rectangle. There is no call by which a widget says "only this corner of
//! me", and so no way for it to say that wrongly. Hover, press and focus are
//! tracked by the tree too, and each marks the *node*: a table that was one
//! node would be repainted whole by every press on it, before it had a chance
//! to have an opinion.
//!
//! So the unit of repainting is the node, and the answer to "repaint only the
//! piles a move touched" is to make a pile a node. Thirteen [`Pile`]s, one per
//! place cards lie at, and one [`Hand`] above them for cards off the table.
//! Then every rule the example needs falls out of rules the tree already has:
//!
//! - a move writes to the piles it changed, and
//!   [`Ui::widget_mut`](denise_ui::Ui::widget_mut) marks exactly those;
//! - carried cards are a node that is moved, and
//!   [`Ui::set_layout`](denise_ui::Ui::set_layout) damages where it was and
//!   where it is;
//! - a card flying home is the same node sent on its way with
//!   [`Ui::animate_layout`](denise_ui::Ui::animate_layout);
//! - the keyboard's ring is the tree's own focus, which repaints the pile it
//!   left and the pile it reached.
//!
//! Nothing in this file calls an invalidate. `--bench` prints what that buys.
//!
//! # What a widget here does not do
//!
//! It does not play. A pile knows what lies on it and where, and reports what
//! was pressed as a [`Message`]; whether that card may be picked up, and where
//! it may go, is `game.rs`'s to say and `app.rs`'s to ask. That is also why
//! most events below are answered [`Handled::No`] although a message was sent:
//! `Yes` would tell the tree this pile changed, and it has not — the
//! application will write back what the press did, to the piles it did it to.

use std::rc::Rc;

use denise::{
    Color, ElementState, InputEvent, KeyCode, Modifiers, Pen, PixelView, Point, PointerButton,
    Rect, Role,
};
use denise_ui::widgets::{Align, Label};
use denise_ui::{
    Animation, Event, EventCtx, Handled, PaintCtx, TextStyle, VisualState, Wake, Widget,
};

use crate::app::{Key, Message};
use crate::faces::Face;
use crate::game::Place;
use crate::layout::{Look, Shape};

/// A card's paper. A colour and not a role, on purpose: a card is a picture of
/// a thing, and it is white under a dark theme and a light one alike.
const PAPER: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// The line round a card.
const EDGE: Color = Color::rgba(0x10, 0x18, 0x20, 0x90);
/// Under cards in the hand.
const SHADOW: Color = Color::rgba(0, 0, 0, 0x50);
/// A back whose picture would not decode.
const PLAIN_BACK: Color = Color::rgb(0x2B, 0x5C, 0x8A);

/// The pictures a pile paints: one for each card lying on it, and the back.
#[derive(Clone, Debug, Default)]
pub struct Pictures {
    /// One per spot of the pile's [`Look`]. `None` for a card that lies face
    /// down, or whose picture will not decode and is drawn as blank paper.
    pub faces: Vec<Option<Rc<Face>>>,
    /// The back, for the face-down ones.
    pub back: Option<Rc<Face>>,
}

/// One place cards lie at: the stock, the waste, a foundation or a pile.
pub struct Pile {
    place: Place,
    shape: Shape,
    look: Look,
    pictures: Pictures,
    /// A press began here and has not ended. The tree keeps sending this node
    /// the pointer until it does, wherever the pointer goes; its own
    /// [`VisualState::PRESSED`] is cleared when the pointer leaves the node,
    /// which is why this is not that.
    held: bool,
}

impl Pile {
    /// An empty pile at `place`.
    pub fn new(place: Place, shape: Shape) -> Self {
        Self {
            place,
            shape,
            look: Look::default(),
            pictures: Pictures::default(),
            held: false,
        }
    }

    /// What it shows now. Read through [`Ui::widget`](denise_ui::Ui::widget),
    /// which marks nothing, to decide whether it needs writing to at all.
    pub fn look(&self) -> &Look {
        &self.look
    }

    /// The measurements it paints with.
    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Shows something else.
    pub fn show(&mut self, shape: Shape, look: Look, pictures: Pictures) {
        self.shape = shape;
        self.look = look;
        self.pictures = pictures;
    }

    /// Where the pile's first card lies, inside the node's bounds.
    fn origin(&self, bounds: Rect) -> Point {
        Point::new(bounds.x + self.shape.margin, bounds.y + self.shape.margin)
    }

    /// A ring round the top `count` cards, or round the slot of an empty pile.
    fn ring(
        &self,
        origin: Point,
        count: usize,
        thickness: i32,
        color: Color,
        canvas: &mut Pen<'_>,
    ) {
        let s = &self.shape;
        let spots = &self.look.spots;
        let card = |dx: i32, dy: i32| Rect::new(origin.x + dx, origin.y + dy, s.width, s.height);
        let first = spots.len().checked_sub(count.max(1)).map(|n| spots[n]);
        let around = match (first, spots.last()) {
            (Some(first), Some(last)) => card(first.dx, first.dy).union(&card(last.dx, last.dy)),
            _ => card(0, 0),
        };
        canvas.stroke_rounded_rect(
            around.inflate(s.margin),
            s.radius + s.margin,
            thickness,
            color,
        );
    }
}

/// How much of the card at spot `n` shows: all of the top one, and of the rest
/// the strip the next card leaves, plus the next card's rounded corner.
///
/// A pile is painted bottom card first, and without this every card under the
/// top one would be drawn whole and then covered. A pile of seven is then
/// seven pictures where one and six strips will do, and a pile of seven is the
/// ordinary case.
fn showing(look: &Look, shape: &Shape, n: usize, card: Rect) -> Rect {
    let Some(next) = look.spots.get(n + 1) else {
        return card;
    };
    let here = look.spots[n];
    if next.dy > here.dy {
        Rect::new(card.x, card.y, card.width, next.dy - here.dy + shape.radius)
    } else if next.dx > here.dx {
        Rect::new(
            card.x,
            card.y,
            next.dx - here.dx + shape.radius,
            card.height,
        )
    } else {
        // Exactly covered.
        Rect::new(card.x, card.y, 0, 0)
    }
}

/// Paints a card face up at `r`: its picture, or blank paper where there is
/// none, and the line round it.
fn face_up(canvas: &mut Pen<'_>, shape: &Shape, picture: Option<&Face>, r: Rect) {
    let view = picture.and_then(|f| PixelView::new(&f.pixels, f.size, f.size.width));
    match view {
        Some(view) => canvas.blit_rounded(&view, r, r, shape.radius),
        None => canvas.fill_rounded_rect(r, shape.radius, PAPER),
    }
    canvas.stroke_rounded_rect(r, shape.radius, shape.line, EDGE);
}

/// Paints a card face down at `r`: the back's picture inside a rim of paper.
fn face_down(canvas: &mut Pen<'_>, shape: &Shape, picture: Option<&Face>, r: Rect) {
    canvas.fill_rounded_rect(r, shape.radius, PAPER);
    let inner = r.inflate(-shape.rim());
    let view = picture.and_then(|f| PixelView::new(&f.pixels, f.size, f.size.width));
    match view {
        Some(view) => canvas.blit_rounded(&view, inner, inner, shape.radius / 2),
        None => canvas.fill_rounded_rect(inner, shape.radius / 2, PLAIN_BACK),
    }
    canvas.stroke_rounded_rect(r, shape.radius, shape.line, EDGE);
}

impl Widget<Message> for Pile {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        let s = &self.shape;
        let origin = self.origin(ctx.bounds);
        let slot = Rect::new(origin.x, origin.y, s.width, s.height);
        let hollow = ctx.theme.color(Role::Base300);

        if self.look.spots.is_empty() {
            canvas.stroke_rounded_rect(slot, s.radius, s.line * 2, hollow);
            match self.place {
                // A ring to press: the stock has run out and comes round again.
                Place::Stock if self.look.again => {
                    let centre = Point::new(slot.x + s.width / 2, slot.y + s.height / 2);
                    canvas.stroke_circle(centre, s.width / 5, s.line * 3, hollow);
                }
                // Where the aces go, said in the default face at a third of a
                // card's height.
                Place::Foundation(_) => {
                    let mark = TextStyle::built_in((s.height / 3).clamp(8, 400) as u16);
                    let extent = ctx.text.measure(mark, "A");
                    let at = Point::new(
                        slot.x + Align::Center.offset(s.width, extent.width as i32),
                        slot.y + Align::Center.offset(s.height, extent.height as i32),
                    );
                    ctx.text.draw(canvas, mark, at, "A", hollow);
                }
                _ => {}
            }
        }

        for (n, spot) in self.look.spots.iter().enumerate() {
            let card = slot.translate(spot.dx, spot.dy);
            // Asked of the canvas, which is already clipped to the damage: a
            // pointer sprite crossing one corner of a pile costs the cards
            // under that corner, not the pile.
            let showing = showing(&self.look, s, n, card);
            if canvas.visible(showing).is_none() {
                continue;
            }
            let mut strip = canvas.with_clip(showing);
            match spot.card {
                Some(_) => {
                    let picture = self.pictures.faces.get(n).and_then(Option::as_deref);
                    face_up(&mut strip, s, picture, card);
                }
                None => face_down(&mut strip, s, self.pictures.back.as_deref(), card),
            }
        }

        // The keyboard: a heavy ring round what it has picked up, a light one
        // where it merely is. The second is the tree's own focus, so moving it
        // costs this file nothing — the tree repaints the pile it left and the
        // pile it reached without being asked.
        let accent = ctx.theme.color(Role::Accent);
        let heavy = (s.margin * 2 / 3).max(1);
        let light = (s.margin / 3).max(1);
        if self.look.chosen > 0 {
            self.ring(origin, self.look.chosen, heavy, accent, canvas);
        } else if ctx.state.contains(VisualState::FOCUSED) {
            self.ring(origin, 1, light, accent, canvas);
        }
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, Message>) -> Handled {
        match event {
            // A finger is a pointer here: the tree has already decided which
            // node it landed on, and that is all the difference there was.
            Event::Input(
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Down,
                    position,
                    ..
                }
                | InputEvent::TouchDown { position, .. },
            ) => {
                let origin = self.origin(ctx.bounds);
                let within = Point::new(position.x - origin.x, position.y - origin.y);
                let slot = Rect::new(0, 0, self.shape.width, self.shape.height);
                let (count, dx, dy) = match self.look.hit(&self.shape, within) {
                    Some((count, spot)) => (count, spot.dx, spot.dy),
                    // An empty stock is still pressed, to turn the waste over.
                    None if slot.contains(within) => (0, 0, 0),
                    // The ring's margin, or the room the waste keeps for a fan.
                    None => return Handled::No,
                };
                self.held = true;
                ctx.emit(Message::Press {
                    place: self.place,
                    count,
                    card: Point::new(origin.x + dx, origin.y + dy),
                    at: *position,
                });
            }
            Event::Input(
                InputEvent::PointerMoved { position } | InputEvent::TouchMoved { position, .. },
            ) if self.held => ctx.emit(Message::Carry(*position)),
            Event::Input(InputEvent::TouchUp {
                cancelled: true, ..
            })
            | Event::PressCancelled
                if self.held =>
            {
                self.held = false;
                ctx.emit(Message::LetGo);
            }
            Event::Input(
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Up,
                    position,
                    ..
                }
                | InputEvent::TouchUp { position, .. },
            ) if self.held => {
                self.held = false;
                ctx.emit(Message::Drop(*position));
            }
            Event::Input(InputEvent::Key {
                code,
                state: ElementState::Down,
                modifiers,
                ..
            }) if !modifiers.contains(Modifiers::CTRL) => {
                let key = match code {
                    KeyCode::ArrowLeft => Key::Left,
                    KeyCode::ArrowRight => Key::Right,
                    KeyCode::ArrowUp => Key::Up,
                    KeyCode::ArrowDown => Key::Down,
                    KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
                    KeyCode::Space => Key::Space,
                    _ => return Handled::No,
                };
                ctx.emit(Message::Key(self.place, key));
            }
            _ => {}
        }
        // See the header: a message was sent, and nothing here has changed.
        Handled::No
    }

    fn accepts_pointer(&self) -> bool {
        true
    }

    /// Focusable, so the arrows have somewhere to be and Tab reaches the table.
    fn focusable(&self) -> bool {
        true
    }

    /// And yet a press does not move the focus here. The ring is the
    /// keyboard's: somebody playing with a mouse should not have one follow
    /// every card they touch, and each move of it is two piles repainted.
    fn preserves_focus(&self) -> bool {
        true
    }
}

/// Cards off the table: carried by the pointer, or flying to a foundation.
///
/// One node, above the piles, hidden when the hand is empty. It takes no
/// pointer — the press that picked the cards up is still held by the pile they
/// came from, and that pile reports where the pointer goes.
///
/// This widget never changes while it is being carried. Its *node* moves, and
/// that is the whole trick: the tree damages the rectangle a node leaves and
/// the one it arrives in, so dragging a card across a 1920×1080 table repaints
/// two card-sized rectangles a frame and not two megapixels.
#[derive(Default)]
pub struct Hand {
    shape: Shape,
    /// How far each card shows above the next.
    step: i32,
    cards: Vec<Option<Rc<Face>>>,
}

impl Hand {
    /// Takes these cards up, the first of them the lowest in the fan.
    pub fn hold(&mut self, shape: Shape, step: i32, cards: Vec<Option<Rc<Face>>>) {
        self.shape = shape;
        self.step = step;
        self.cards = cards;
    }
}

impl Widget<Message> for Hand {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        if self.cards.is_empty() {
            return;
        }
        let s = &self.shape;
        let b = ctx.bounds;
        let tail = (self.cards.len() as i32 - 1) * self.step;
        let shade = Rect::new(b.x + s.shadow.0, b.y + s.shadow.1, s.width, s.height + tail);
        canvas.fill_rounded_rect(shade, s.radius, SHADOW);
        for (k, picture) in self.cards.iter().enumerate() {
            let card = Rect::new(b.x, b.y + k as i32 * self.step, s.width, s.height);
            face_up(canvas, s, picture.as_deref(), card);
        }
    }
}

/// How the game stands, at the right of the bar: the clock and the moves.
///
/// A widget, and not a label the application writes to once a second, for the
/// reason the gallery's clock is one: only a widget can name its own deadline.
/// It answers [`Wake::At`] the moment the second turns, the tree reports that
/// through [`Ui::next_wake_ms`](denise_ui::Ui::next_wake_ms), and both backends
/// sleep until then with no timer of their own. Before the first move and
/// after the last it answers [`Wake::Never`], so a table nobody is playing at
/// wakes for nothing.
pub struct Status {
    /// The drawing is a label's; only the knowing-when is this widget's.
    face: Label,
    stands: Stands,
}

/// What a game's status is made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stands {
    /// Moves made, turns of the stock among them.
    pub moves: u32,
    /// Every card is home.
    pub won: bool,
    /// When the first move was made, on the tree's clock.
    pub began: Option<u64>,
    /// How many seconds the game took, once it is won.
    pub took: Option<u32>,
}

impl Stands {
    /// Whole seconds played by `now_ms`.
    pub fn seconds(&self, now_ms: u64) -> Option<u32> {
        self.took.or_else(|| {
            self.began
                .map(|began| u32::try_from(now_ms.saturating_sub(began) / 1000).unwrap_or(u32::MAX))
        })
    }

    /// What the bar says: nothing before the first move, then the time and
    /// the moves.
    pub fn words(&self, now_ms: u64) -> String {
        let moves = match self.moves {
            0 => return String::new(),
            1 => "1 move".to_owned(),
            n => format!("{n} moves"),
        };
        match (self.won, self.seconds(now_ms).map(clock)) {
            (true, Some(time)) => format!("Out in {time}  ·  {moves}"),
            (true, None) => format!("Out in {moves}"),
            (false, Some(time)) => format!("{time}  ·  {moves}"),
            (false, None) => moves,
        }
    }
}

/// Seconds as a clock shows them: `3:07`, and `1:02:03` past the hour.
pub fn clock(seconds: u32) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl Status {
    /// An empty status, written in `style`.
    pub fn new(style: TextStyle) -> Self {
        Self {
            face: Label::new("")
                .with_style(style)
                .with_align(Align::End, Align::Center),
            stands: Stands::default(),
        }
    }

    /// How the game stood when this was last told.
    pub fn stands(&self) -> Stands {
        self.stands
    }

    /// The words on the bar, for the tests to read.
    #[cfg(test)]
    pub fn text(&self) -> &str {
        self.face.text()
    }

    /// Tells it how the game stands now. The caller follows this with
    /// [`Ui::request_animation`](denise_ui::Ui::request_animation), which is
    /// how a clock that has just started gets its first deadline.
    pub fn set(&mut self, stands: Stands, now_ms: u64) {
        self.stands = stands;
        self.face.set_text(stands.words(now_ms));
        self.face.set_role(if stands.won {
            Role::Accent
        } else {
            Role::BaseContent
        });
    }

    fn tick(&mut self, now_ms: u64) -> Animation {
        let repaint = self.face.update(&self.stands.words(now_ms));
        let next = match (self.stands.began, self.stands.took) {
            // The start of the next second of *this game*, not a flat thousand
            // from now: woken a little late, a fixed interval would drift
            // until the clock on the bar skipped a second to catch up.
            (Some(began), None) => {
                let played = now_ms.saturating_sub(began);
                Wake::At(began.saturating_add((played / 1000 + 1) * 1000))
            }
            _ => Wake::Never,
        };
        Animation { repaint, next }
    }
}

impl Widget<Message> for Status {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        Widget::<Message>::paint(&self.face, ctx, canvas);
    }

    fn animate(&mut self, now_ms: u64) -> Animation {
        self.tick(now_ms)
    }

    /// With motion turned off a widget is asked to land and stop. A clock
    /// cannot: reduced motion is a request not to move things, not a request
    /// to stop telling the time.
    fn snap(&mut self, now_ms: u64) -> Animation {
        self.tick(now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::{Stands, Status, clock, showing};
    use crate::game::{Game, Place};
    use crate::layout::Layout;
    use denise::{Rect, Size};
    use denise_ui::{TextStyle, Wake};

    #[test]
    fn a_clock_reads_minutes_and_seconds_and_hours_when_there_are_any() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(187), "3:07");
        assert_eq!(clock(3723), "1:02:03");
    }

    #[test]
    fn the_bar_says_nothing_until_a_move_is_made_and_then_counts() {
        let mut stands = Stands::default();
        assert_eq!(stands.words(5_000), "");
        stands.began = Some(1_000);
        assert_eq!(stands.words(5_000), "", "a clock with no move is nothing");
        stands.moves = 1;
        assert_eq!(stands.words(1_999), "0:00  ·  1 move");
        stands.moves = 2;
        assert_eq!(stands.words(188_000), "3:07  ·  2 moves");
        // Won: the time it took, whatever the clock says since.
        stands.won = true;
        stands.took = Some(61);
        assert_eq!(stands.words(999_000), "Out in 1:01  ·  2 moves");
    }

    #[test]
    fn the_status_wakes_on_the_second_and_not_at_all_when_no_clock_runs() {
        let mut status = Status::new(TextStyle::built_in(16));
        assert_eq!(status.tick(10_000).next, Wake::Never);
        let running = Stands {
            moves: 1,
            began: Some(10_250),
            ..Stands::default()
        };
        status.set(running, 10_250);
        assert_eq!(status.text(), "0:00  ·  1 move");
        // Nothing to repaint until the second turns, and woken exactly then.
        let first = status.tick(10_300);
        assert!(!first.repaint);
        assert_eq!(first.next, Wake::At(11_250));
        // Woken late, it still aims at the game's own next second.
        let late = status.tick(11_400);
        assert!(late.repaint);
        assert_eq!(status.text(), "0:01  ·  1 move");
        assert_eq!(late.next, Wake::At(12_250));
        // Won: the clock stops, and so do the wakes.
        status.set(
            Stands {
                won: true,
                took: Some(1),
                ..running
            },
            11_500,
        );
        assert_eq!(status.tick(60_000).next, Wake::Never);
        assert_eq!(status.text(), "Out in 0:01  ·  1 move");
    }

    #[test]
    fn a_covered_card_is_painted_only_as_far_as_it_shows() {
        let layout = Layout::new(Size::new(1200, 800), 1.0, false);
        let s = layout.shape;
        let mut game = Game::new(4, 3);
        game.deal();
        let card = |dx: i32, dy: i32| Rect::new(dx, dy, s.width, s.height);

        // A pile: strips as tall as the step, and the next card's corner.
        let pile = layout.look(&game, Place::Tableau(6), 0, 0, 0);
        let first = showing(&pile, &s, 0, card(0, 0));
        assert_eq!(first, Rect::new(0, 0, s.width, pile.spots[1].dy + s.radius));
        let top = pile.spots[6];
        assert_eq!(showing(&pile, &s, 6, card(0, top.dy)), card(0, top.dy));

        // The waste fans sideways.
        let waste = layout.look(&game, Place::Waste, 0, 0, 0);
        assert_eq!(
            showing(&waste, &s, 0, card(0, 0)),
            Rect::new(0, 0, layout.fan + s.radius, s.height)
        );
        // Together the strips are far less than the cards they stand for.
        let painted: i64 = (0..7)
            .map(|n| {
                let r = showing(&pile, &s, n, card(0, pile.spots[n].dy));
                i64::from(r.width) * i64::from(r.height)
            })
            .sum();
        assert!(painted * 3 < 7 * i64::from(s.width) * i64::from(s.height));
    }
}
