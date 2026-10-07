//! The game's tree, and everything that happens in it.
//!
//! Platform-independent, like `table-editor`'s and the gallery's: this file
//! never learns whether it is in a window or on the scanout buffer of a
//! Raspberry Pi. It is handed events and a clock, and it keeps a tree.
//!
//! # The shape of it
//!
//! A bar of ordinary widgets — [`Button`]s, a [`Panel`], and a status line —
//! over thirteen [`Pile`]s and one [`Hand`], which `table.rs` explains. A
//! question ("Deal again?") is a scene pushed over all of it, the way
//! `table-editor` asks before deleting a record.
//!
//! # One place decides what is painted
//!
//! Everything that can happen ends in [`App::refresh`]. It works out what each
//! of the thirteen piles *should* show from the game as it now stands, compares
//! that with what each pile's widget already shows, and writes to the ones that
//! differ. Nothing else touches a pile.
//!
//! That is the whole of this example's invalidation, and it is worth noticing
//! what is absent from it. No handler below knows which piles its move
//! touched: turning the stock does not say "the stock and the waste", undo
//! does not remember what it undid, and auto-finish does not track the card it
//! sent. Each changes the game and returns. The comparison finds the piles,
//! [`Ui::widget_mut`] marks them, and a move that touched two piles repaints
//! two piles — because the tree was told the truth about two widgets, not
//! because anybody computed a rectangle.
//!
//! The price is thirteen small comparisons per event, of vectors a few cards
//! long. On a Pi 3 that is microseconds; a pile repainted needlessly is
//! milliseconds.
//!
//! # Whose keys are whose
//!
//! The arrows, Enter and Space go to whichever pile has the focus, through the
//! tree, like any key to any widget. The letters — `N`, `U`, `B`, `T`, `H` —
//! and the chords are the application's and are read here, the way `hello`
//! reads Escape: they mean the same thing wherever the focus is, including
//! nowhere. The bar's buttons take no focus at all
//! ([`Button::no_focus`]), so pressing "Undo" with the mouse does not carry
//! the keyboard off to a button and leave Space meaning "Undo again".

use denise::{ElementState, InputEvent, KeyCode, Modifiers, Point, Rect, Role, Size, theme};
use denise_ui::widgets::{Button, Label, Panel};
use denise_ui::{GlyphSource, NodeId, Ui};

use crate::cards::Rng;
use crate::faces::{BACKS, Faces};
use crate::game::{Game, PILES, Place};
use crate::layout::{Layout, Look, PLACES, index, places};
use crate::table::{Hand, Pictures, Pile, Stands, Status};

/// A key a pile takes while it has the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// To the pile on the left.
    Left,
    /// To the pile on the right.
    Right,
    /// To the row above; on a pile whose cards are picked up, one card more.
    Up,
    /// To the row below; on a pile whose cards are picked up, one fewer.
    Down,
    /// Pick up, or put down.
    Enter,
    /// Turn the stock.
    Space,
}

/// What the widgets send back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// The bar: deal again.
    New,
    /// The bar: take the last move back.
    Undo,
    /// The bar: turn one card from the stock, or three. Deals again.
    Turn,
    /// The bar: the next back.
    Back,
    /// The bar: leave. Only on a display with no window to close.
    Quit,
    /// A pile was pressed.
    Press {
        /// Which.
        place: Place,
        /// How many cards lie from the one pressed to the top; 0 for a card
        /// face down, or an empty pile.
        count: usize,
        /// Where the pressed card's top-left corner is.
        card: Point,
        /// Where the pointer is.
        at: Point,
    },
    /// The pointer moved with the press still held.
    Carry(Point),
    /// The press ended here.
    Drop(Point),
    /// The press ended nowhere: the system took the finger away, or a dialog
    /// came up under it.
    LetGo,
    /// A key for the pile that has the focus.
    Key(Place, Key),
    /// A question was answered: `true` is the answer that loses the game.
    Answer(bool),
}

/// What an application is started with.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The deals come from this, one after another: the same seed is the
    /// same evening of solitaire.
    pub seed: u64,
    /// How many cards the stock turns at a time, one or three.
    pub turn: usize,
    /// Offer a way out on the bar. A window has a close button; a display
    /// with no desktop has only what the application draws.
    pub quit: bool,
}

/// What either backend hands the application to write with.
pub type Font = Option<(String, Box<dyn GlyphSource>)>;

/// How long a card takes to fly to its foundation.
const FLIGHT_MS: u64 = 140;
/// Two presses on a card within this are a double click.
const DOUBLE_MS: u64 = 400;
/// How dark the table goes behind a question.
const VEIL: u8 = 150;

/// Cards carried by the pointer.
struct Drag {
    place: Place,
    count: usize,
    /// From the first card's corner to the pointer.
    grip: (i32, i32),
}

/// A card on its way to where the game already has it.
struct Flight {
    /// The foundation it is flying to, which does not show it until it lands.
    place: Place,
    /// Where the hand's node comes to rest.
    to: Rect,
}

/// What is being asked over the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// Whether to give this game up for one that turns so many cards.
    Deal(usize),
    /// Whether to leave a game under way.
    Leave,
}

struct Dialog {
    ask: Ask,
    panel: NodeId,
    /// Where the keyboard was, to put it back when the question is answered.
    focus: Option<NodeId>,
}

/// The nodes that get written to after startup.
struct Nodes {
    bar: NodeId,
    new: NodeId,
    undo: NodeId,
    turn: NodeId,
    back: NodeId,
    quit: Option<NodeId>,
    status: NodeId,
    /// One per place, in [`places`]' order.
    piles: [NodeId; PLACES],
    hand: NodeId,
}

pub struct App {
    pub ui: Ui<Message>,
    /// Set when the game is to end. Both backends read it after every update.
    pub exit: bool,
    game: Game,
    seeds: Rng,
    faces: Faces,
    layout: Layout,
    /// Physical pixels per logical one, as the surface was when this was built.
    scale: f32,
    offers_quit: bool,
    nodes: Nodes,
    /// Which of [`BACKS`] the cards wear.
    back: usize,
    drag: Option<Drag>,
    flight: Option<Flight>,
    /// The cards the keyboard has picked up, which stay where they lie.
    chosen: Option<(Place, usize)>,
    /// The last press on a card and when, for telling a double click.
    pressed: Option<(Place, usize, u64)>,
    /// When the first move of this game was made.
    began: Option<u64>,
    /// How long it took, once it is won.
    took: Option<u32>,
    dialog: Option<Dialog>,
    /// The last key that went down was held with Ctrl, Alt or the logo key, so
    /// whatever text follows it is a chord's and not a letter's.
    chord: bool,
    /// The clock, as of the last [`App::update`].
    now: u64,
    started: std::time::Instant,
}

/// The top row, by the column each place stands in. The third is empty.
const ROW: [Option<Place>; PILES] = [
    Some(Place::Stock),
    Some(Place::Waste),
    None,
    Some(Place::Foundation(0)),
    Some(Place::Foundation(1)),
    Some(Place::Foundation(2)),
    Some(Place::Foundation(3)),
];

/// The column a place stands in, and whether it is in the top row.
fn column(place: Place) -> (usize, bool) {
    match place {
        Place::Stock => (0, true),
        Place::Waste => (1, true),
        Place::Foundation(i) => (3 + i.min(3), true),
        Place::Tableau(i) => (i.min(PILES - 1), false),
    }
}

/// The place the keyboard reaches from `at` with an arrow, if there is one.
fn step(at: Place, key: Key) -> Option<Place> {
    let (col, top) = column(at);
    let along = |by: isize| -> Option<usize> {
        let mut c = col;
        loop {
            c = c.checked_add_signed(by).filter(|c| *c < PILES)?;
            if !top || ROW[c].is_some() {
                return Some(c);
            }
        }
    };
    let (col, top) = match key {
        Key::Left => (along(-1)?, top),
        Key::Right => (along(1)?, top),
        // Up from under the gap in the top row lands on the waste beside it.
        Key::Up if !top => (if col == 2 { 1 } else { col }, true),
        Key::Down if top => (col, false),
        _ => return None,
    };
    if top {
        ROW[col]
    } else {
        Some(Place::Tableau(col))
    }
}

impl App {
    /// Builds the tree once, for a surface of `size` physical pixels at
    /// `scale`, and deals.
    pub fn new(size: Size, scale: f32, font: Font, options: Options) -> Self {
        let mut seeds = Rng::new(options.seed);
        let game = Game::new(seeds.number(), options.turn);
        Self::with_game(size, scale, font, options, seeds, game)
    }

    /// The same, with a game somebody else set out: how the tests get a table
    /// five cards from its end without playing a hundred moves to reach it.
    fn with_game(
        size: Size,
        scale: f32,
        font: Font,
        options: Options,
        seeds: Rng,
        game: Game,
    ) -> Self {
        let px = |v: f32| (v * scale + 0.5) as u16;
        let layout = Layout::new(size, scale, options.quit);

        let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale));
        // One face for everything: registered as the default, so no widget
        // below has to be told which font it is in. Without one the built-in
        // bitmap face draws the same tree.
        if let Some((name, source)) = font {
            let id = ui.add_font(source);
            ui.set_default_font(id);
            eprintln!("font    {name}");
        }
        let root = ui.root();
        let text = px(15.0);

        let bar = ui
            .add(root, Panel::filled(Role::Base200), Self::bar_rect(&layout))
            .expect("bar");
        // No focus: see the header. Each still presses, paints pressed and
        // emits; the keyboard simply stays on the table.
        let mut button = |label: &str, message: Message, rect: Rect| {
            let widget = Button::new(label, message)
                .with_role(Role::Neutral)
                .with_size(text)
                .no_focus();
            ui.add(root, widget, rect).expect("button")
        };
        let new = button("New game", Message::New, layout.bar.new);
        let undo = button("Undo", Message::Undo, layout.bar.undo);
        let turn = button("Turn one", Message::Turn, layout.bar.turn);
        let back = button("Back", Message::Back, layout.bar.back);
        let quit = layout
            .bar
            .quit
            .map(|rect| button("Quit", Message::Quit, rect));
        let status = ui
            .add(
                root,
                Status::new(denise_ui::TextStyle::built_in(text)),
                layout.bar.status,
            )
            .expect("status");

        // The piles start empty and at their slots; `refresh` below gives each
        // its cards and its true extent, by the same path every later change
        // takes. There is no second way to fill a pile.
        let mut piles = [root; PLACES];
        for (node, place) in piles.iter_mut().zip(places()) {
            let empty = layout.extent(place, &Look::default());
            *node = ui
                .add(root, Pile::new(place, layout.shape), empty)
                .expect("pile");
        }
        // Above every pile, and nowhere until there is something to carry.
        let hand = ui.add(root, Hand::default(), Rect::ZERO).expect("hand");
        ui.set_z(hand, 1);
        ui.set_visible(hand, false);

        let mut app = Self {
            ui,
            exit: false,
            game,
            seeds,
            faces: Faces::default(),
            layout,
            scale,
            offers_quit: options.quit,
            nodes: Nodes {
                bar,
                new,
                undo,
                turn,
                back,
                quit,
                status,
                piles,
                hand,
            },
            back: 0,
            drag: None,
            flight: None,
            chosen: None,
            pressed: None,
            began: None,
            took: None,
            dialog: None,
            chord: false,
            now: 0,
            started: std::time::Instant::now(),
        };
        app.refresh();
        app
    }

    /// The bar's panel, pushed past the surface's top and sides so that the
    /// theme's rounded corners are off it and the bar reads as an edge.
    fn bar_rect(layout: &Layout) -> Rect {
        let b = layout.bar.rect;
        let over = b.height;
        Rect::new(-over, -over, b.width + 2 * over, b.height + over)
    }

    /// Milliseconds since the application started: the clock both backends
    /// hand to [`App::update`], so the tree's ticks come from one place.
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// A point on the top card of one of the seven piles: where the bench
    /// presses to pick a card up, since nothing outside this file knows where
    /// anything is.
    pub fn top_card(&self, pile: usize) -> Point {
        let card = self.layout.top(&self.game, Place::Tableau(pile), 1);
        Point::new(card.x + card.width / 2, card.y + card.height / 2)
    }

    /// One turn of the loop: events in, the tree and the game brought up to
    /// `now`. What is left to do afterwards is paint, if
    /// [`Ui::needs_paint`] says so.
    pub fn update(&mut self, events: &[InputEvent], now: u64) {
        self.now = now;
        // How things stood before the tree saw these events. A key that
        // answers a question must not also be read as a key for the table
        // that the answer has just uncovered.
        let asked = self.dialog.is_some();
        let focused = self.ui.focused();

        self.ui.handle(events);
        // Ticked before the messages are read, so that a flight started by
        // one of them starts on this frame's clock and not the last one's.
        self.ui.tick(now);

        if let Some(size) = events.iter().rev().find_map(|event| match event {
            InputEvent::SurfaceResized { size, .. } => Some(*size),
            _ => None,
        }) {
            self.arrange(size);
        }

        // Collected before acting: draining borrows the tree, and acting on a
        // message needs it back.
        let messages: Vec<Message> = self.ui.drain_messages().collect();
        // A mouse reports far more often than a frame is drawn. Every report
        // moved the cards for a frame nobody saw, and each would add two
        // rectangles of damage — past sixteen the tracker gives up and takes
        // their bounding box, which is the whole path of the drag. Only the
        // last position before something else happens is where the cards are.
        let mut carried = None;
        for message in messages {
            if let Message::Carry(at) = message {
                carried = Some(at);
                continue;
            }
            if let Some(at) = carried.take() {
                self.carry(at);
            }
            self.on_message(message);
        }
        if let Some(at) = carried {
            self.carry(at);
        }

        self.shortcuts(events, asked, focused);
        self.settle();
    }

    /// Acts on one message. Changes the game; [`App::settle`] shows it.
    pub fn on_message(&mut self, message: Message) {
        match message {
            Message::New => self.ask_new(self.game.turn()),
            Message::Turn => self.ask_new(if self.game.turn() == 3 { 1 } else { 3 }),
            Message::Undo => self.undo(),
            Message::Back => self.wear(1),
            Message::Quit => self.leave(),
            Message::Press {
                place,
                count,
                card,
                at,
            } => self.press(place, count, card, at),
            Message::Carry(at) => self.carry(at),
            Message::Drop(at) => self.drop_at(Some(at)),
            Message::LetGo => self.drop_at(None),
            // By where the focus is now, not where it was when the key went
            // down: two arrows in one batch are two steps, and the second
            // starts where the first one ended.
            Message::Key(place, key) => {
                let at = self.focused_place().unwrap_or(place);
                self.key(at, key);
            }
            Message::Answer(yes) => self.answer(yes),
        }
    }

    // ------------------------------------------------------------ the pointer

    fn press(&mut self, place: Place, count: usize, card: Point, at: Point) {
        // The pointer has taken over: the keyboard lets go of what it held and
        // its ring goes, until an arrow brings it back.
        self.ui.focus(None);
        self.chosen = None;
        let twice = matches!(
            self.pressed,
            Some((p, c, when)) if p == place && c == count && self.now.saturating_sub(when) < DOUBLE_MS
        );
        self.pressed = Some((place, count, self.now));

        if place == Place::Stock {
            self.land();
            self.game.deal();
            return;
        }
        if self.game.held(place, count).is_none() {
            return;
        }
        if twice && count == 1 && self.send_home(place) {
            self.pressed = None;
            return;
        }
        self.land();
        let size = self.layout.shape.card();
        let cards: Vec<_> = self
            .game
            .held(place, count)
            .unwrap_or_default()
            .iter()
            .map(|card| self.faces.card(*card, size))
            .collect();
        self.hold(cards, card);
        self.drag = Some(Drag {
            place,
            count,
            grip: (at.x - card.x, at.y - card.y),
        });
    }

    fn carry(&mut self, at: Point) {
        let Some(drag) = &self.drag else {
            return;
        };
        let corner = Point::new(at.x - drag.grip.0, at.y - drag.grip.1);
        // The whole of dragging a card. The tree damages where the node was
        // and where it is; the piles underneath are repainted inside those two
        // rectangles and nowhere else.
        self.ui
            .set_layout(self.nodes.hand, self.layout.hand(corner, drag.count));
    }

    /// The press ended: over `at`, or nowhere.
    fn drop_at(&mut self, at: Option<Point>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.empty_hand();
        let Some(at) = at else {
            return;
        };
        // Judged by where the first card in the hand is, near its top, and
        // not by where the pointer is on it: what the card covers is what the
        // eye aimed.
        let s = self.layout.shape;
        let over = Point::new(
            at.x - drag.grip.0 + s.width / 2,
            at.y - drag.grip.1 + s.height / 4,
        );
        if let Some(to) = self.layout.target(over) {
            self.game.play(drag.place, drag.count, to);
        }
    }

    /// Puts cards in the hand, with the first of them at `corner`.
    fn hold(&mut self, cards: Vec<Option<std::rc::Rc<crate::faces::Face>>>, corner: Point) {
        let hand = self.nodes.hand;
        let rect = self.layout.hand(corner, cards.len());
        if let Some(widget) = self.ui.widget_mut::<Hand>(hand) {
            widget.hold(self.layout.shape, self.layout.up, cards);
        }
        self.ui.set_layout(hand, rect);
        self.ui.set_visible(hand, true);
    }

    /// Empties the hand. Parked at no size as well as hidden, so that the
    /// next cards it takes up are not announced by a repaint of wherever the
    /// last ones were put down.
    fn empty_hand(&mut self) {
        self.ui.set_visible(self.nodes.hand, false);
        self.ui.set_layout(self.nodes.hand, Rect::ZERO);
    }

    // ----------------------------------------------------------- the keyboard

    /// The pile the keyboard is at, if it is at one.
    fn focused_place(&self) -> Option<Place> {
        let focused = self.ui.focused()?;
        places().find(|place| self.nodes.piles[index(*place)] == focused)
    }

    fn key(&mut self, at: Place, key: Key) {
        match key {
            Key::Left | Key::Right | Key::Up | Key::Down => self.arrow(at, key),
            Key::Enter => self.act(at),
            Key::Space => self.turn_stock(),
        }
    }

    fn arrow(&mut self, at: Place, key: Key) {
        // On the pile picked up from, up and down take more cards or fewer.
        if let Some((place, count)) = self.chosen
            && place == at
            && matches!(place, Place::Tableau(_))
            && matches!(key, Key::Up | Key::Down)
        {
            let wanted = if key == Key::Up {
                count + 1
            } else {
                count.saturating_sub(1)
            };
            if self.game.held(place, wanted).is_some() {
                self.chosen = Some((place, wanted));
            }
            return;
        }
        if let Some(next) = step(at, key) {
            // The ring is the tree's focus. Moving it repaints two piles, and
            // the tree knows which two.
            self.ui.focus(Some(self.nodes.piles[index(next)]));
        }
    }

    /// Enter: turn the stock, pick up, or put down.
    fn act(&mut self, at: Place) {
        match self.chosen.take() {
            // Put back where it came from.
            Some((from, _)) if from == at => {}
            Some((from, count)) => {
                self.land();
                if self.game.play(from, count, at).is_none() {
                    self.chosen = Some((from, count));
                }
            }
            None if at == Place::Stock => self.turn_stock(),
            None => {
                if self.game.held(at, 1).is_some() {
                    self.chosen = Some((at, 1));
                }
            }
        }
    }

    fn turn_stock(&mut self) {
        self.chosen = None;
        self.land();
        self.game.deal();
    }

    /// The keys that are the application's and not a widget's.
    ///
    /// `asked` and `focused` are how things stood *before* the tree saw these
    /// events. Enter on "Keep playing" closes the question through the tree;
    /// read again here against the table that is now showing, the same Enter
    /// would pick a card up.
    fn shortcuts(&mut self, events: &[InputEvent], asked: bool, focused: Option<NodeId>) {
        let on_table = focused.is_some_and(|id| self.nodes.piles.contains(&id));
        for event in events {
            match *event {
                InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    modifiers,
                    repeat,
                } => {
                    let ctrl = modifiers.contains(Modifiers::CTRL);
                    self.chord = ctrl
                        || modifiers.contains(Modifiers::ALT)
                        || modifiers.contains(Modifiers::SUPER);
                    match code {
                        // The ways out, and Ctrl+C among them: it is what a
                        // hand tries first at a console. Under a question they
                        // leave at once — it has been asked already.
                        KeyCode::Q | KeyCode::C if ctrl && asked => self.exit = true,
                        KeyCode::Q | KeyCode::C if ctrl => self.leave(),
                        KeyCode::Escape if asked => self.answer(false),
                        _ if asked || repeat => {}
                        KeyCode::Z if ctrl => self.undo(),
                        KeyCode::N if ctrl => self.ask_new(self.game.turn()),
                        KeyCode::Backspace => self.undo(),
                        // Escape puts down what the keyboard picked up, and
                        // with nothing picked up it is the way out it is in
                        // every other example — asking first, here, since
                        // there may be a game to lose.
                        KeyCode::Escape if self.chosen.take().is_some() => {}
                        KeyCode::Escape => self.leave(),
                        // With the focus nowhere, the keys that would have
                        // gone to a pile bring it to the table instead.
                        KeyCode::Space if !on_table => self.turn_stock(),
                        KeyCode::ArrowLeft
                        | KeyCode::ArrowRight
                        | KeyCode::ArrowUp
                        | KeyCode::ArrowDown
                        | KeyCode::Enter
                        | KeyCode::NumpadEnter
                            if !on_table =>
                        {
                            self.ui
                                .focus(Some(self.nodes.piles[index(Place::Tableau(0))]));
                        }
                        _ => {}
                    }
                }
                // Letters come as text, so that N is the key marked N on a
                // keyboard laid out any way at all.
                InputEvent::Text { ch } if !self.chord => match (asked, ch) {
                    (true, 'y' | 'Y') => self.answer(true),
                    (true, 'n' | 'N') => self.answer(false),
                    (true, _) => {}
                    (false, 'n' | 'N') => self.ask_new(self.game.turn()),
                    (false, 'u' | 'U') => self.undo(),
                    (false, 't' | 'T') => self.on_message(Message::Turn),
                    (false, 'b') => self.wear(1),
                    (false, 'B') => self.wear(BACKS.len() - 1),
                    // Home: the card the keyboard is at, to its foundation.
                    (false, 'h' | 'H') => {
                        let at = places().find(|p| Some(self.nodes.piles[index(*p)]) == focused);
                        if at.is_some_and(|at| self.send_home(at)) {
                            self.chosen = None;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    // --------------------------------------------------------------- the game

    fn undo(&mut self) {
        // A game that is out is over: its time is taken.
        if self.game.won() {
            return;
        }
        self.land();
        self.chosen = None;
        self.game.undo();
    }

    /// Wears the back `by` after this one, round and round.
    fn wear(&mut self, by: usize) {
        self.back = (self.back + by) % BACKS.len();
    }

    /// Whether there is a game to lose by dealing or leaving.
    fn under_way(&self) -> bool {
        self.game.moves() > 0 && !self.game.won()
    }

    /// Deals again, asking first if there is a game under way to lose.
    fn ask_new(&mut self, turn: usize) {
        if self.under_way() {
            self.ask(Ask::Deal(turn));
        } else {
            self.deal_again(turn);
        }
    }

    /// Leaves, asking first if there is a game under way to lose.
    fn leave(&mut self) {
        if self.under_way() {
            self.ask(Ask::Leave);
        } else {
            self.exit = true;
        }
    }

    fn deal_again(&mut self, turn: usize) {
        self.land();
        self.drop_at(None);
        self.game = Game::new(self.seeds.number(), turn);
        self.chosen = None;
        self.pressed = None;
        self.began = None;
        self.took = None;
    }

    /// Sends the top card of `from` to its foundation, flying.
    ///
    /// The game has the card home at once; the table shows it arriving. The
    /// flight is [`Ui::animate_layout`] on the hand's node — the tree's own
    /// tween, sampled at the tree's own rate and damaged the way a moved node
    /// always is — so there is no animation code in this example at all.
    fn send_home(&mut self, from: Place) -> bool {
        let Some(&card) = self.game.face_up(from).last() else {
            return false;
        };
        if self.game.home(from).is_none() {
            return false;
        }
        // One card in the air at a time: the one before lands first.
        self.land();
        let start = self.layout.top(&self.game, from, 1);
        let Some(to) = self.game.play(from, 1, Place::Foundation(0)) else {
            return false;
        };
        let end = self.layout.slot(to);
        let picture = self.faces.card(card, self.layout.shape.card());
        self.hold(vec![picture], Point::new(start.x, start.y));
        let rest = self.layout.hand(Point::new(end.x, end.y), 1);
        self.ui.animate_layout(self.nodes.hand, rest, FLIGHT_MS);
        self.flight = Some(Flight {
            place: to,
            to: rest,
        });
        true
    }

    /// Lands a card that is in the air, wherever it has got to.
    fn land(&mut self) {
        if self.flight.take().is_some() {
            self.empty_hand();
        }
    }

    /// After anything at all: land what has arrived, play the next card of a
    /// game that plays itself, keep the clock, and show the result.
    fn settle(&mut self) {
        // Arrived when the tree's tween has put the node where it was sent.
        if let Some(flight) = &self.flight
            && self.ui.layout(self.nodes.hand) == Some(flight.to)
        {
            self.land();
        }
        // Nothing face down and nothing left to turn: the rest plays itself,
        // one card flying home at a time. Each landing is a tick of the tree,
        // which is an update, which comes back here for the next card.
        if self.flight.is_none()
            && self.drag.is_none()
            && self.dialog.is_none()
            && self.game.runs_out()
            && let Some(from) = self.game.next_home()
        {
            self.send_home(from);
        }
        // The clock starts at the first move and stops when the last card is
        // home.
        if self.began.is_none() && self.game.moves() > 0 {
            self.began = Some(self.now);
        }
        if self.game.won() && self.took.is_none() {
            self.took = Some(self.stands().seconds(self.now).unwrap_or(0));
        }
        self.refresh();
    }

    fn stands(&self) -> Stands {
        Stands {
            moves: self.game.moves(),
            won: self.game.won(),
            began: self.began,
            took: self.took,
        }
    }

    // ------------------------------------------------------- what is painted

    /// How many of a place's top cards are off the table: in the hand, or in
    /// the air on their way there.
    fn gone(&self, place: Place) -> usize {
        match (&self.drag, &self.flight) {
            (Some(drag), _) if drag.place == place => drag.count,
            (_, Some(flight)) if flight.place == place => 1,
            _ => 0,
        }
    }

    /// What `place` should show now.
    fn look(&self, place: Place) -> Look {
        let chosen = match self.chosen {
            Some((at, count)) if at == place => count,
            _ => 0,
        };
        self.layout
            .look(&self.game, place, self.gone(place), chosen, self.back)
    }

    /// Brings the tree in line with the game, writing only to what differs.
    ///
    /// Returns the places it wrote to, which nothing but the tests reads: it
    /// is how they say "a move is the piles it touched and no others" without
    /// decoding rectangles.
    fn refresh(&mut self) -> Vec<Place> {
        let mut written = Vec::new();
        let shape = self.layout.shape;
        for (i, place) in places().enumerate() {
            let node = self.nodes.piles[i];
            let look = self.look(place);
            // Read through `widget`, which marks nothing. `widget_mut` marks
            // the node on the way in whatever is then done with it, so asking
            // it thirteen times whether anything changed would repaint the
            // table to find out that nothing had.
            let same = self
                .ui
                .widget::<Pile>(node)
                .is_some_and(|pile| *pile.look() == look && *pile.shape() == shape);
            let extent = self.layout.extent(place, &look);
            if !same {
                let pictures = Pictures {
                    faces: look
                        .spots
                        .iter()
                        .map(|spot| spot.card.and_then(|c| self.faces.card(c, shape.card())))
                        .collect(),
                    back: look
                        .spots
                        .iter()
                        .any(|spot| spot.card.is_none())
                        .then(|| self.faces.back(self.back, shape.back()))
                        .flatten(),
                };
                if let Some(pile) = self.ui.widget_mut::<Pile>(node) {
                    pile.show(shape, look, pictures);
                }
                written.push(place);
            }
            // A pile's node is as big as its cards. `set_layout` does nothing
            // when the rectangle is the one it has.
            self.ui.set_layout(node, extent);
        }

        // The bar, by the same rule: read, compare, and write what differs.
        let alive = self.game.can_undo() && !self.game.won();
        self.ui.set_enabled(self.nodes.undo, alive);
        let turn = if self.game.turn() == 3 {
            "Turn three"
        } else {
            "Turn one"
        };
        self.relabel(self.nodes.turn, turn);
        self.relabel(self.nodes.back, &format!("Back: {}", BACKS[self.back].0));
        let stands = self.stands();
        let status = self.nodes.status;
        if self.ui.widget::<Status>(status).map(Status::stands) != Some(stands) {
            let now = self.now;
            if let Some(widget) = self.ui.widget_mut::<Status>(status) {
                widget.set(stands, now);
            }
            // The clock's first deadline, or its last: the widget answers
            // which the next time the tree ticks.
            self.ui.request_animation(status);
        }
        written
    }

    fn relabel(&mut self, id: NodeId, label: &str) {
        let same = self
            .ui
            .widget::<Button<Message>>(id)
            .is_some_and(|button| button.label() == label);
        if !same && let Some(button) = self.ui.widget_mut::<Button<Message>>(id) {
            button.set_label(label);
        }
    }

    /// Lays everything out again for a surface that has changed size.
    ///
    /// The tree has no layout engine and so nothing moves by itself: the same
    /// arithmetic that placed the nodes places them again. The cards change
    /// size with the table, which `refresh` notices as a different shape.
    fn arrange(&mut self, size: Size) {
        // Whatever was in the air or in the hand belongs to the old table.
        self.land();
        self.drop_at(None);
        self.layout = Layout::new(size, self.scale, self.offers_quit);
        let bar = self.layout.bar;
        self.ui
            .set_layout(self.nodes.bar, Self::bar_rect(&self.layout));
        self.ui.set_layout(self.nodes.new, bar.new);
        self.ui.set_layout(self.nodes.undo, bar.undo);
        self.ui.set_layout(self.nodes.turn, bar.turn);
        self.ui.set_layout(self.nodes.back, bar.back);
        if let (Some(node), Some(rect)) = (self.nodes.quit, bar.quit) {
            self.ui.set_layout(node, rect);
        }
        self.ui.set_layout(self.nodes.status, bar.status);
        if let Some(dialog) = &self.dialog {
            self.ui.set_layout(dialog.panel, self.dialog_rect());
        }
    }

    // ----------------------------------------------------------- the question

    fn dialog_rect(&self) -> Rect {
        let size = self.ui.size();
        let s = Rect::new(0, 0, 400, 164).scaled(self.scale);
        let width = s.width.min(size.width as i32);
        Rect::new(
            (size.width as i32 - width) / 2,
            (size.height as i32 - s.height) / 2,
            width,
            s.height,
        )
    }

    /// Puts a question over the table, on its own dimmed scene.
    ///
    /// A scene is the toolkit's whole answer to modality. The table underneath
    /// is still drawn and still there, and takes no input because the scene
    /// above it takes all of it — no pile is disabled and no flag is checked
    /// in `table.rs`. The clock on the bar goes on ticking under the veil,
    /// repainting its own strip through it.
    fn ask(&mut self, ask: Ask) {
        if self.dialog.is_some() {
            return;
        }
        // Pushing a scene cancels a held press, and the pile will say so; the
        // cards are put back now rather than a frame later.
        self.drop_at(None);
        let focus = self.ui.focused();
        let s = |r: Rect| r.scaled(self.scale);
        let px = |v: f32| (v * self.scale + 0.5) as u16;
        let rect = self.dialog_rect();
        let wide = ((rect.width as f32) / self.scale) as i32;

        let scene = self.ui.push_scene(VEIL);
        let panel = self.ui.add(scene, Panel::default(), rect).expect("dialog");
        let (title, lose) = match ask {
            Ask::Deal(_) => ("Deal again?", "New game"),
            Ask::Leave => ("Leave the game?", "Quit"),
        };
        self.ui.add(
            panel,
            Label::new(title).with_size(px(20.0)),
            s(Rect::new(20, 20, wide - 40, 26)),
        );
        self.ui.add(
            panel,
            Label::new("This game will be lost.").with_size(px(15.0)),
            s(Rect::new(20, 56, wide - 40, 22)),
        );
        let keep = self.ui.add(
            panel,
            Button::new("Keep playing", Message::Answer(false)).with_size(px(15.0)),
            s(Rect::new(20, 108, 150, 36)),
        );
        self.ui.add(
            panel,
            Button::new(lose, Message::Answer(true))
                .with_role(Role::Error)
                .with_size(px(15.0)),
            s(Rect::new(wide - 20 - 130, 108, 130, 36)),
        );
        // Focus lands on the answer that loses nothing, so Enter on a question
        // nobody read does the harmless thing. Y and N answer it outright.
        self.ui.focus(keep);
        self.dialog = Some(Dialog { ask, panel, focus });
    }

    fn answer(&mut self, yes: bool) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        self.ui.pop_scene();
        // Popping a scene leaves the focus nowhere. The keyboard goes back to
        // the pile it was on.
        self.ui.focus(dialog.focus);
        match dialog.ask {
            Ask::Deal(turn) if yes => self.deal_again(turn),
            Ask::Leave if yes => self.exit = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{App, Key, Message, Options, step};
    use crate::cards::{Card, KING, Rng, Suit};
    use crate::game::{Game, PILES, Pile as Cards, Place};
    use crate::layout::{index, places};
    use crate::table::Status;
    use denise::{
        BufferAge, ElementState, Frame, InputEvent, KeyCode, Modifiers, PixelFormat, Point,
        PointerButton, Rect, Size,
    };

    const SIZE: Size = Size::new(1100, 760);

    fn options() -> Options {
        Options {
            seed: 42,
            turn: 1,
            quit: false,
        }
    }

    /// An application with nothing drawn yet, and no pointer sprite: a window
    /// system draws its own, and the tests are about the table's damage.
    fn app() -> App {
        let mut app = App::new(SIZE, 1.0, None, options());
        app.ui.show_cursor(false);
        app
    }

    /// A game with two kings, two queens and a jack left to go home.
    fn nearly_out() -> App {
        let card = |suit, rank| Card { suit, rank };
        let mut piles: [Cards; PILES] = Default::default();
        piles[0].up = vec![card(Suit::Spades, KING), card(Suit::Hearts, 12)];
        piles[1].up = vec![card(Suit::Hearts, KING), card(Suit::Spades, 12)];
        let waste = vec![card(Suit::Hearts, 11)];
        let game = Game::set_out(piles, Vec::new(), waste, [KING, KING, 10, 11], 1);
        let mut app = App::with_game(SIZE, 1.0, None, options(), Rng::new(1), game);
        app.ui.show_cursor(false);
        app
    }

    /// Paints what is pending and retires it, as a backend does, and returns
    /// what was pending: the rectangles that frame repainted.
    fn frame(app: &mut App) -> Vec<Rect> {
        let pending = app.ui.pending_damage().to_vec();
        let whole = app.ui.needs_paint() && pending.is_empty();
        let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
        let mut frame = Frame::new(
            &mut pixels,
            SIZE,
            SIZE.width,
            PixelFormat::Xrgb8888,
            BufferAge::Frames(1),
        )
        .expect("frame");
        app.ui.paint(&mut frame);
        app.ui.presented();
        if whole {
            vec![Rect::from_size(SIZE)]
        } else {
            pending
        }
    }

    fn key(code: KeyCode, modifiers: Modifiers) -> InputEvent {
        InputEvent::Key {
            code,
            state: ElementState::Down,
            repeat: false,
            modifiers,
        }
    }

    fn plain(code: KeyCode) -> InputEvent {
        key(code, Modifiers::NONE)
    }

    fn button(state: ElementState, position: Point) -> InputEvent {
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position,
            modifiers: Modifiers::NONE,
        }
    }

    fn press(at: Point) -> InputEvent {
        button(ElementState::Down, at)
    }

    fn release(at: Point) -> InputEvent {
        button(ElementState::Up, at)
    }

    fn moved(position: Point) -> InputEvent {
        InputEvent::PointerMoved { position }
    }

    fn centre(r: Rect) -> Point {
        Point::new(r.x + r.width / 2, r.y + r.height / 2)
    }

    /// A point on the top card of `place`, near its corner.
    fn on(app: &App, place: Place) -> Point {
        let card = app.layout.top(&app.game, place, 1);
        Point::new(card.x + 10, card.y + 10)
    }

    fn extent(app: &App, place: Place) -> Rect {
        app.ui.bounds(app.nodes.piles[index(place)]).expect("pile")
    }

    /// Deals until some pile's top card goes on another's, and says which.
    /// The seed is fixed, so this is the same deal every run.
    fn a_move_between_piles(app: &mut App) -> (usize, usize) {
        for _ in 0..50 {
            for from in 0..PILES {
                for to in 0..PILES {
                    let (a, b) = (Place::Tableau(from), Place::Tableau(to));
                    if app.game.landing(a, 1, b).is_some() {
                        return (from, to);
                    }
                }
            }
            app.update(&[InputEvent::Text { ch: 'n' }], 0);
        }
        panic!("no deal of fifty has a move between piles");
    }

    #[test]
    fn the_arrows_walk_the_table_and_step_over_the_gap_in_the_top_row() {
        let t = Place::Tableau;
        assert_eq!(step(t(0), Key::Right), Some(t(1)));
        assert_eq!(step(t(0), Key::Left), None);
        assert_eq!(step(t(6), Key::Right), None);
        assert_eq!(step(t(4), Key::Up), Some(Place::Foundation(1)));
        assert_eq!(step(t(2), Key::Up), Some(Place::Waste), "under the gap");
        assert_eq!(step(Place::Waste, Key::Right), Some(Place::Foundation(0)));
        assert_eq!(step(Place::Foundation(0), Key::Left), Some(Place::Waste));
        assert_eq!(step(Place::Stock, Key::Down), Some(t(0)));
        assert_eq!(step(Place::Stock, Key::Up), None);
        assert_eq!(step(t(3), Key::Down), None);
    }

    #[test]
    fn a_fresh_table_has_thirteen_piles_and_holds_nobody_awake() {
        let mut app = app();
        app.update(&[], 0);
        for place in places() {
            let pile = extent(&app, place);
            assert!(Rect::from_size(SIZE).contains_rect(&pile), "{place:?}");
        }
        assert_eq!(frame(&mut app), [Rect::from_size(SIZE)], "the first frame");
        // Nothing moves, no clock runs: the loop may sleep until an input
        // device has something to say.
        app.update(&[], 16);
        app.update(&[], 5_000);
        assert_eq!(app.ui.animating(), 0);
        assert_eq!(app.ui.next_wake_ms(), None);
        assert!(!app.ui.needs_paint());
    }

    #[test]
    fn turning_the_stock_repaints_the_stock_the_waste_and_the_bar_and_no_pile() {
        let mut app = app();
        app.update(&[], 0);
        frame(&mut app);

        app.update(&[plain(KeyCode::Space)], 100);
        assert_eq!(app.game.moves(), 1);
        let damage = frame(&mut app);
        assert!(!damage.is_empty());
        let stock = extent(&app, Place::Stock);
        let waste = extent(&app, Place::Waste);
        let bar = app.layout.bar.rect;
        for rect in &damage {
            assert!(
                stock.contains_rect(rect) || waste.contains_rect(rect) || bar.contains_rect(rect),
                "{rect:?} is none of the stock, the waste and the bar"
            );
            for pile in 0..PILES {
                assert!(!rect.intersects(&extent(&app, Place::Tableau(pile))));
            }
        }
        assert!(damage.iter().any(|r| r.intersects(&waste)));

        // What the comparison wrote to, said without rectangles.
        app.game.deal();
        assert_eq!(app.refresh(), [Place::Waste]);
        assert_eq!(app.refresh(), [], "and nothing the second time");
    }

    #[test]
    fn a_card_is_dragged_to_where_it_goes_and_dropped_back_where_it_does_not() {
        let mut app = app();
        let (from, to) = a_move_between_piles(&mut app);
        app.update(&[], 0);
        let grab = on(&app, Place::Tableau(from));

        // Let go over nothing: it stays.
        app.update(&[press(grab)], 10);
        assert!(app.drag.is_some());
        assert!(app.ui.visible(app.nodes.hand));
        let nowhere = Point::new(-200, 300);
        app.update(&[moved(nowhere), release(nowhere)], 20);
        assert!(app.drag.is_none());
        assert!(!app.ui.visible(app.nodes.hand));
        assert_eq!(app.game.moves(), 0);

        // Let go over the other pile: it moves, and the clock starts.
        let target = app.layout.top(&app.game, Place::Tableau(to), 1);
        let over = Point::new(target.x + 12 + 10, target.y + 40 + 10);
        // Long after the first press, so this is not a double click.
        app.update(&[press(grab)], 2_000);
        app.update(&[moved(over)], 2_010);
        app.update(&[release(over)], 2_020);
        assert_eq!(app.game.moves(), 1);
        assert_eq!(app.began, Some(2_020));
        let status = app.ui.widget::<Status>(app.nodes.status).unwrap();
        assert_eq!(status.text(), "0:00  ·  1 move");
        // And wakes for its next second, not for a frame.
        app.update(&[], 2_030);
        assert_eq!(app.ui.next_wake_ms(), Some(3_020));
    }

    #[test]
    fn a_carried_card_repaints_where_it_was_and_where_it_is_and_nothing_else() {
        let mut app = app();
        let (from, _) = a_move_between_piles(&mut app);
        app.update(&[], 0);
        frame(&mut app);
        let grab = on(&app, Place::Tableau(from));

        // Picked up: the pile it left, and no other.
        app.update(&[press(grab)], 10);
        let source = extent(&app, Place::Tableau(from));
        for rect in frame(&mut app) {
            // The pile as it was, a card taller, is where the hand now is.
            assert!(
                rect.x >= source.x - 2 && rect.right() <= source.right() + 8,
                "{rect:?} is not the pile the card left"
            );
        }

        // Carried across the middle of the table, many reports to a frame.
        let hand = |app: &App| app.ui.bounds(app.nodes.hand).unwrap();
        let mut was = hand(&app);
        for step in 1..=20 {
            let reports: Vec<InputEvent> = (0..8)
                .map(|n| moved(Point::new(grab.x + step * 20 + n, grab.y + step * 9)))
                .collect();
            app.update(&reports, 10 + step as u64 * 16);
            let now = hand(&app);
            let damage = frame(&mut app);
            assert!(!damage.is_empty() && damage.len() <= 3, "{damage:?}");
            // Inside the two places the hand was and is. When the pointer
            // crosses the edge of the pile it took the card from, the tree
            // repaints that pile as well — it has stopped being hovered — and
            // the tracker joins rectangles that overlap, so there the pile is
            // let in too.
            let around = was.union(&now);
            for rect in &damage {
                let allowed = if rect.intersects(&source) {
                    around.union(&source)
                } else {
                    around
                };
                assert!(
                    allowed.contains_rect(rect),
                    "{rect:?} is outside {was:?} and {now:?}"
                );
            }
            // Two cards' worth, not the table's.
            let painted: u64 = damage.iter().map(Rect::area).sum();
            assert!(painted < SIZE.area() / 5, "{painted} pixels");
            was = now;
        }
        assert_eq!(app.refresh(), [], "nothing on the table changed");
    }

    #[test]
    fn a_move_writes_to_the_piles_it_touched_and_no_others() {
        let mut app = app();
        let (from, to) = a_move_between_piles(&mut app);
        app.update(&[], 0);
        let (a, b) = (Place::Tableau(from), Place::Tableau(to));

        app.game.play(a, 1, b);
        let mut written = app.refresh();
        written.sort_by_key(|place| index(*place));
        let mut expected = [a, b];
        expected.sort_by_key(|place| index(*place));
        assert_eq!(written, expected);

        // Taken back: the same two.
        app.game.undo();
        assert_eq!(app.refresh().len(), 2);

        // Another back is every pile that shows one, and the foundations and
        // the waste are not among them.
        app.wear(1);
        let written = app.refresh();
        assert!(written.contains(&Place::Stock));
        assert!(written.contains(&Place::Tableau(6)));
        assert!(!written.contains(&Place::Tableau(0)), "one card, face up");
        assert!(!written.contains(&Place::Waste));
        assert!(!written.contains(&Place::Foundation(0)));
    }

    #[test]
    fn a_pointer_crossing_the_table_never_repaints_more_than_the_piles_it_crossed() {
        let mut app = app();
        app.update(&[], 0);
        frame(&mut app);
        let y = on(&app, Place::Tableau(0)).y;
        let mut most = 0;
        for x in (0..SIZE.width as i32).step_by(7) {
            app.update(&[moved(Point::new(x, y))], 16);
            let damage = frame(&mut app);
            // The tree repaints a node the pointer enters or leaves. With a
            // node to a pile that is one pile or two — never the table.
            let painted: u64 = damage.iter().map(Rect::area).sum();
            most = most.max(painted);
            for rect in &damage {
                assert!(
                    (0..PILES).any(|i| extent(&app, Place::Tableau(i)).contains_rect(rect)),
                    "{rect:?} is not a pile"
                );
            }
        }
        assert!(most > 0 && most < SIZE.area() / 8, "{most}");
        // And over the open table below the cards it repaints nothing at all.
        app.update(&[moved(Point::new(300, SIZE.height as i32 - 5))], 32);
        frame(&mut app);
        app.update(&[moved(Point::new(700, SIZE.height as i32 - 5))], 48);
        assert!(!app.ui.needs_paint());
    }

    #[test]
    fn the_keyboard_s_ring_is_the_tree_s_focus_and_moves_between_two_piles() {
        let mut app = app();
        app.update(&[], 0);
        frame(&mut app);
        let node = |app: &App, place: Place| Some(app.nodes.piles[index(place)]);

        // The first arrow brings the focus to the table.
        assert_eq!(app.ui.focused(), None);
        app.update(&[plain(KeyCode::ArrowDown)], 10);
        assert_eq!(app.ui.focused(), node(&app, Place::Tableau(0)));
        let first = extent(&app, Place::Tableau(0));
        for rect in frame(&mut app) {
            assert!(first.contains_rect(&rect));
        }

        // The next moves it: the pile it left and the pile it reached.
        app.update(&[plain(KeyCode::ArrowRight)], 20);
        assert_eq!(app.ui.focused(), node(&app, Place::Tableau(1)));
        let second = extent(&app, Place::Tableau(1));
        let damage = frame(&mut app);
        assert_eq!(damage.len(), 2);
        for rect in damage {
            assert!(first.contains_rect(&rect) || second.contains_rect(&rect));
        }
        assert_eq!(app.game.moves(), 0, "an arrow is not a move");

        // Up to the top row, and Enter on the stock turns it.
        app.update(&[plain(KeyCode::ArrowUp), plain(KeyCode::ArrowLeft)], 30);
        assert_eq!(app.ui.focused(), node(&app, Place::Stock));
        app.update(&[plain(KeyCode::Enter)], 40);
        assert_eq!(app.game.waste().len(), 1);

        // A press with the pointer takes the ring away again.
        let at = on(&app, Place::Tableau(3));
        app.update(&[press(at), release(at)], 50);
        assert_eq!(app.ui.focused(), None);
        // And Space still turns the stock from nowhere.
        app.update(&[plain(KeyCode::Space)], 3_000);
        assert_eq!(app.game.waste().len(), 2);
    }

    #[test]
    fn enter_picks_up_and_puts_down_and_up_takes_one_card_more() {
        let mut app = app();
        let (from, to) = a_move_between_piles(&mut app);
        app.update(&[], 0);
        let (a, b) = (Place::Tableau(from), Place::Tableau(to));
        app.ui.focus(Some(app.nodes.piles[index(a)]));

        app.update(&[plain(KeyCode::Enter)], 10);
        assert_eq!(app.chosen, Some((a, 1)));
        // One card face up: there is no second to take, and one fewer than
        // one is none, which is not a thing to hold.
        app.update(&[plain(KeyCode::ArrowUp), plain(KeyCode::ArrowDown)], 20);
        assert_eq!(app.chosen, Some((a, 1)));
        assert_eq!(app.ui.focused(), Some(app.nodes.piles[index(a)]));
        // Escape puts it down, and does not leave.
        app.update(&[plain(KeyCode::Escape)], 30);
        assert_eq!(app.chosen, None);
        assert!(!app.exit);

        // Picked up again, carried over, put down.
        app.update(&[plain(KeyCode::Enter)], 40);
        app.ui.focus(Some(app.nodes.piles[index(b)]));
        app.update(&[plain(KeyCode::Enter)], 50);
        assert_eq!(app.chosen, None);
        assert_eq!(app.game.moves(), 1);
        // Where it does not go, it stays picked up.
        app.update(&[plain(KeyCode::Enter)], 60);
        let held = app.chosen.expect("picked up from where it was put");
        app.ui.focus(Some(app.nodes.piles[index(Place::Stock)]));
        app.update(&[plain(KeyCode::Enter)], 70);
        assert_eq!(app.chosen, Some(held));
        assert_eq!(app.game.moves(), 1);
        // U takes the move back, and Backspace has nothing left to.
        app.update(&[InputEvent::Text { ch: 'u' }], 80);
        assert_eq!(app.game.moves(), 0);
        app.update(&[plain(KeyCode::Backspace)], 90);
        assert_eq!(app.game.moves(), 0);
    }

    #[test]
    fn a_fresh_game_is_dealt_again_without_asking_and_one_under_way_asks() {
        let mut app = app();
        app.update(&[], 0);
        let dealt = app.game.tableau().clone();
        app.update(&[InputEvent::Text { ch: 'n' }], 10);
        assert!(app.dialog.is_none());
        assert_ne!(*app.game.tableau(), dealt, "another deal");

        // A move made: now it asks, by the key and by the button alike.
        app.update(&[plain(KeyCode::Space)], 20);
        assert_eq!(app.game.moves(), 1);
        let under_way = app.game.tableau().clone();
        let new = centre(app.layout.bar.new);
        app.update(&[press(new), release(new)], 30);
        assert!(app.dialog.is_some(), "asks before dealing again");
        assert_eq!(app.ui.scene_count(), 2);
        // Escape keeps playing.
        app.update(&[plain(KeyCode::Escape)], 40);
        assert!(app.dialog.is_none());
        assert_eq!(app.ui.scene_count(), 1);
        assert!(!app.exit, "Escape answered the question and did no more");
        assert_eq!(*app.game.tableau(), under_way);
        // So does N, the second time: the first asked.
        app.update(&[key(KeyCode::N, Modifiers::CTRL)], 50);
        assert!(app.dialog.is_some());
        app.update(&[plain(KeyCode::N), InputEvent::Text { ch: 'n' }], 60);
        assert!(app.dialog.is_none());
        // While it asks, the table takes nothing.
        app.update(&[InputEvent::Text { ch: 'n' }], 70);
        app.update(&[plain(KeyCode::U), InputEvent::Text { ch: 'u' }], 80);
        app.update(&[plain(KeyCode::Backspace)], 85);
        assert!(app.dialog.is_some());
        assert_eq!(app.game.moves(), 1);
        // Enter is on the answer that loses nothing, and the table it
        // uncovers does not take that Enter for its own.
        app.update(&[plain(KeyCode::Enter)], 90);
        assert!(app.dialog.is_none());
        assert_eq!(app.game.moves(), 1);
        assert_eq!(app.chosen, None);
        assert_eq!(app.ui.focused(), None);
        // Y deals.
        app.update(&[InputEvent::Text { ch: 'n' }], 100);
        app.update(&[InputEvent::Text { ch: 'y' }], 110);
        assert!(app.dialog.is_none());
        assert_eq!(app.game.moves(), 0);
        assert_ne!(*app.game.tableau(), under_way);
        assert_eq!(app.began, None, "and the clock is back at nothing");
    }

    #[test]
    fn the_question_is_answered_by_its_buttons_and_turning_three_deals_again() {
        let mut app = app();
        app.update(&[plain(KeyCode::Space)], 0);
        let turn = centre(app.layout.bar.turn);
        app.update(&[press(turn), release(turn)], 10);
        let panel = app.ui.bounds(app.dialog.as_ref().unwrap().panel).unwrap();
        assert!(Rect::from_size(SIZE).contains_rect(&panel));
        // A press on the panel but on no button answers nothing, and nothing
        // under the veil is pressed either.
        let idle = Point::new(panel.x + 6, panel.y + 6);
        app.update(&[press(idle), release(idle)], 20);
        let stock = on(&app, Place::Stock);
        app.update(&[press(stock), release(stock)], 30);
        assert!(app.dialog.is_some());
        assert_eq!(app.game.moves(), 1);
        // The left button keeps playing, turning one.
        let keep = Point::new(panel.x + 60, panel.bottom() - 40);
        app.update(&[press(keep), release(keep)], 40);
        assert!(app.dialog.is_none());
        assert_eq!(app.game.turn(), 1);
        // The right one deals, turning three.
        app.update(&[press(turn), release(turn)], 50);
        let lose = Point::new(panel.right() - 60, panel.bottom() - 40);
        app.update(&[press(lose), release(lose)], 60);
        assert!(app.dialog.is_none());
        assert_eq!(app.game.turn(), 3);
        assert_eq!(app.game.moves(), 0);
        // T does the same from the keyboard, and with no game to lose it
        // does not ask.
        app.update(&[InputEvent::Text { ch: 't' }], 70);
        assert_eq!(app.game.turn(), 1);
    }

    #[test]
    fn every_way_out_is_a_way_out_and_a_game_under_way_is_asked_about() {
        for way in [
            key(KeyCode::Q, Modifiers::CTRL),
            key(KeyCode::C, Modifiers::CTRL),
            plain(KeyCode::Escape),
        ] {
            let mut app = app();
            app.update(std::slice::from_ref(&way), 0);
            assert!(app.exit, "{way:?}");
        }
        // A letter by itself is the game's, and Ctrl+W is nobody's.
        let mut app = app();
        app.update(
            &[
                plain(KeyCode::Q),
                InputEvent::Text { ch: 'q' },
                key(KeyCode::W, Modifiers::CTRL),
                // The text a chord may leave behind is not a letter either.
                key(KeyCode::N, Modifiers::CTRL),
                InputEvent::Text { ch: 'n' },
            ],
            0,
        );
        assert!(!app.exit);
        assert!(app.dialog.is_none());

        // With a game under way it asks; Escape there keeps playing.
        app.update(&[plain(KeyCode::Space)], 10);
        app.update(&[plain(KeyCode::Escape)], 20);
        assert!(!app.exit);
        assert!(app.dialog.is_some());
        app.update(&[plain(KeyCode::Escape)], 30);
        assert!(app.dialog.is_none() && !app.exit);
        assert_eq!(app.game.moves(), 1);
        // Y leaves.
        app.update(&[key(KeyCode::Q, Modifiers::CTRL)], 40);
        app.update(&[plain(KeyCode::Y), InputEvent::Text { ch: 'y' }], 50);
        assert!(app.exit);
        // And the chord a second time does not wait to be answered.
        let mut app = super::App::new(SIZE, 1.0, None, options());
        app.update(&[plain(KeyCode::Space)], 0);
        app.update(&[key(KeyCode::C, Modifiers::CTRL)], 10);
        assert!(!app.exit);
        app.update(&[key(KeyCode::C, Modifiers::CTRL)], 20);
        assert!(app.exit);
    }

    #[test]
    fn a_display_with_no_window_gets_a_button_to_leave_by() {
        let windowed = app();
        assert!(windowed.nodes.quit.is_none());
        let mut bare = App::new(
            SIZE,
            1.0,
            None,
            Options {
                quit: true,
                ..options()
            },
        );
        let quit = centre(bare.layout.bar.quit.expect("a button"));
        bare.update(&[press(quit), release(quit)], 0);
        assert!(bare.exit);
    }

    #[test]
    fn a_double_click_sends_a_card_home_and_the_rest_of_the_game_plays_itself() {
        let mut app = nearly_out();
        app.update(&[], 0);
        frame(&mut app);
        // Nothing hidden: it has already begun, the lowest card first.
        assert_eq!(app.game.moves(), 1);
        let hearts = Place::Foundation(Suit::Hearts.index());
        assert_eq!(app.flight.as_ref().map(|f| f.place), Some(hearts));
        assert!(app.ui.visible(app.nodes.hand));
        // The card is home in the game and not yet on the table.
        assert_eq!(app.game.foundation(2).len(), 11);
        assert_eq!(app.look(hearts).spots[0].card.unwrap().rank, 10);

        // In the air it repaints where it was and is, and the tree asks to be
        // woken for the next sample of the flight.
        let mut now = 0;
        let mut frames = 0;
        while !app.game.won() || app.flight.is_some() {
            assert!(app.ui.next_wake_ms().is_some(), "nothing would wake it");
            now += 16;
            app.update(&[], now);
            let painted: u64 = frame(&mut app).iter().map(Rect::area).sum();
            assert!(painted < SIZE.area() / 4, "{painted} pixels in one frame");
            frames += 1;
            assert!(frames < 400, "the game did not run out");
        }
        assert_eq!(app.game.moves(), 5);
        assert!(!app.ui.visible(app.nodes.hand));
        assert_eq!(app.look(hearts).spots[0].card.unwrap().rank, KING);
        let status = app.ui.widget::<Status>(app.nodes.status).unwrap();
        assert!(status.text().starts_with("Out in 0:0"), "{}", status.text());
        // Out: the clock has stopped, and so has everything else.
        app.update(&[], now + 16);
        app.update(&[], now + 5_000);
        assert_eq!(app.ui.next_wake_ms(), None);
        // A game that is out is not taken back, and is dealt again unasked.
        app.update(&[InputEvent::Text { ch: 'u' }], now + 5_010);
        assert!(app.game.won());
        app.update(&[InputEvent::Text { ch: 'n' }], now + 5_020);
        assert!(app.dialog.is_none() && !app.game.won());
    }

    #[test]
    fn two_presses_on_a_card_that_has_a_home_send_it_there() {
        let card = |suit, rank| Card { suit, rank };
        let mut piles: [Cards; PILES] = Default::default();
        piles[0] = Cards {
            down: vec![card(Suit::Clubs, 9)],
            up: vec![card(Suit::Hearts, 1)],
        };
        piles[1].up = vec![card(Suit::Spades, 5)];
        let stock = vec![card(Suit::Diamonds, 7)];
        let game = Game::set_out(piles, stock, Vec::new(), [0; 4], 1);
        let mut app = App::with_game(SIZE, 1.0, None, options(), Rng::new(1), game);
        app.update(&[], 0);

        let ace = on(&app, Place::Tableau(0));
        app.update(&[press(ace), release(ace)], 100);
        assert_eq!(
            app.game.moves(),
            0,
            "one press is a card picked up and put back"
        );
        app.update(&[press(ace), release(ace)], 300);
        assert_eq!(app.game.foundation(Suit::Hearts.index()).len(), 1);
        assert!(app.flight.is_some());
        // The card under it has turned up while the ace is still in the air.
        assert_eq!(app.game.tableau()[0].up, [card(Suit::Clubs, 9)]);
        // Two presses far apart are two presses: the five has no home anyway,
        // and H on a card with none does nothing.
        let five = on(&app, Place::Tableau(1));
        app.update(&[press(five), release(five)], 1_000);
        app.update(&[press(five), release(five)], 1_100);
        app.ui
            .focus(Some(app.nodes.piles[index(Place::Tableau(1))]));
        app.update(&[InputEvent::Text { ch: 'h' }], 1_200);
        assert_eq!(app.game.moves(), 1);
    }

    #[test]
    fn the_back_goes_round_three_and_the_button_says_which() {
        let mut app = app();
        let label = |app: &App| {
            app.ui
                .widget::<denise_ui::Button<Message>>(app.nodes.back)
                .unwrap()
                .label()
                .to_owned()
        };
        assert_eq!(label(&app), "Back: Mist");
        let back = centre(app.layout.bar.back);
        app.update(&[press(back), release(back)], 0);
        assert_eq!(label(&app), "Back: Crimson");
        app.update(&[InputEvent::Text { ch: 'b' }], 10);
        assert_eq!(label(&app), "Back: 16-bit");
        app.update(&[InputEvent::Text { ch: 'b' }], 20);
        assert_eq!(label(&app), "Back: Mist");
        app.update(&[InputEvent::Text { ch: 'B' }], 30);
        assert_eq!(label(&app), "Back: 16-bit");
        // Undo is dead until there is a move to take back.
        assert!(!app.ui.enabled(app.nodes.undo));
        app.update(&[plain(KeyCode::Space)], 40);
        assert!(app.ui.enabled(app.nodes.undo));
    }

    #[test]
    fn a_surface_that_changes_size_is_laid_out_again() {
        let mut app = app();
        app.update(&[plain(KeyCode::Space)], 0);
        let before = *app
            .ui
            .widget::<super::Pile>(app.nodes.piles[0])
            .unwrap()
            .shape();
        let size = Size::new(1600, 1000);
        let resized = InputEvent::SurfaceResized {
            size,
            scale_factor: 1.0,
        };
        app.update(&[resized], 10);
        let after = *app
            .ui
            .widget::<super::Pile>(app.nodes.piles[0])
            .unwrap()
            .shape();
        assert!(after.width > before.width, "the cards grew with the table");
        for place in places() {
            assert!(Rect::from_size(size).contains_rect(&extent(&app, place)));
            let slot = app.layout.slot(place);
            assert!(extent(&app, place).contains_rect(&slot), "{place:?}");
        }
        assert_eq!(app.ui.bounds(app.nodes.status), Some(app.layout.bar.status));
        assert_eq!(app.game.moves(), 1, "and the game is the one it was");
    }
}
