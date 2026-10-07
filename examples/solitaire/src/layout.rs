//! Where everything on the table is, worked out from the size of the surface.
//!
//! Denise has no layout engine and this example does not miss one: a card table
//! is seven columns and two rows, and the arithmetic for that is shorter than
//! the constraints that would describe it. The cards are as large as the
//! surface lets seven of them stand side by side with a fanned pile under the
//! top row, so the table fills a 1100×760 window and a 1920×1080 display alike.
//!
//! Nothing here draws and nothing here knows a widget exists. It answers three
//! kinds of question, all of them testable with no display:
//!
//! - **where a node goes** — [`Layout::extent`], the rectangle a pile is given
//!   in the tree;
//! - **what a pile shows** — [`Layout::look`], the cards lying on it and where,
//!   which is also the value compared to decide whether the pile changed;
//! - **what a point means** — [`Look::hit`] for a press, [`Layout::target`] for
//!   a card let go of.
//!
//! The bar's rectangles are written in logical pixels and multiplied by the
//! scale factor, the way every Denise example does it. The cards are not: they
//! are a fraction of the surface, and a fraction of a surface is already in
//! physical pixels.

use denise::{Point, Rect, Size};

use crate::cards::Card;
use crate::faces::ASPECT;
use crate::game::{Game, PILES, Pile, Place};

/// How many places cards lie at: the stock, the waste, four foundations and
/// seven piles. One node each.
pub const PLACES: usize = 2 + 4 + PILES;

/// Every place, in the order the tree holds their nodes.
pub fn places() -> impl Iterator<Item = Place> {
    [Place::Stock, Place::Waste]
        .into_iter()
        .chain((0..4).map(Place::Foundation))
        .chain((0..PILES).map(Place::Tableau))
}

/// A place's position among [`places`].
pub fn index(place: Place) -> usize {
    match place {
        Place::Stock => 0,
        Place::Waste => 1,
        Place::Foundation(i) => 2 + i.min(3),
        Place::Tableau(i) => 6 + i.min(PILES - 1),
    }
}

/// A card's measurements on this table, in physical pixels.
///
/// Copied into every pile, so that a pile can paint and hit-test from its own
/// state and its own bounds and nothing else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    /// A card's width.
    pub width: i32,
    /// A card's height.
    pub height: i32,
    /// A card's corner.
    pub radius: i32,
    /// The line round a card: what keeps one white card apart from the next.
    pub line: i32,
    /// The room round a pile for the ring the keyboard draws, which is why a
    /// pile's node is this much larger than its cards on every side.
    pub margin: i32,
    /// How far the shadow under carried cards falls to the right and below.
    pub shadow: (i32, i32),
}

impl Shape {
    /// A card's size, as the pictures are asked for.
    pub fn card(&self) -> Size {
        Size::new(self.width.max(1) as u32, self.height.max(1) as u32)
    }

    /// The white rim round the picture on a card's back.
    pub fn rim(&self) -> i32 {
        (self.width * 5 / 100).max(2)
    }

    /// The size of the picture on a card's back, inside the rim.
    pub fn back(&self) -> Size {
        let rim = self.rim();
        Size::new(
            (self.width - 2 * rim).max(1) as u32,
            (self.height - 2 * rim).max(1) as u32,
        )
    }
}

/// One card lying on a pile: which, and where from the pile's first card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    /// The card, or `None` for one that lies face down.
    pub card: Option<Card>,
    /// How far right of the pile's first card.
    pub dx: i32,
    /// How far below it.
    pub dy: i32,
}

/// What a pile shows.
///
/// A plain value that is cheap to compare, and that is the point of it: the
/// application works out the look of all thirteen piles after anything happens
/// and writes only the ones that differ from what their widget already holds.
/// The tree turns each write into that pile's rectangle of damage, so *which
/// piles a move repaints* is decided by `==` here and by nothing cleverer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// The cards lying here, the bottom one first.
    pub spots: Vec<Spot>,
    /// How many of the top cards the keyboard has picked up. They stay where
    /// they lie and get a ring round them.
    pub chosen: usize,
    /// The stock only: it is empty, and there is a waste to turn over again.
    pub again: bool,
    /// Which back the face-down cards wear.
    pub back: usize,
}

impl Look {
    /// What is under `at`, a point measured from the pile's first card: how
    /// many cards lie from the one pointed at to the top, or 0 for a card
    /// that is face down. `None` where no card is.
    ///
    /// Searched from the top card down, because the card seen is the one
    /// pointed at and a card lower in the pile shows only a strip of itself.
    pub fn hit(&self, shape: &Shape, at: Point) -> Option<(usize, Spot)> {
        let (index, spot) = self.spots.iter().enumerate().rev().find(|(_, spot)| {
            Rect::new(spot.dx, spot.dy, shape.width, shape.height).contains(at)
        })?;
        let count = match spot.card {
            Some(_) => self.spots.len() - index,
            None => 0,
        };
        Some((count, *spot))
    }

    /// Everything the cards cover, measured from the pile's first card. An
    /// empty pile covers its slot.
    pub fn covered(&self, shape: &Shape) -> Rect {
        let slot = Rect::new(0, 0, shape.width, shape.height);
        self.spots.iter().fold(slot, |all, spot| {
            all.union(&Rect::new(spot.dx, spot.dy, shape.width, shape.height))
        })
    }
}

/// The buttons on the bar, in the order they stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    /// The bar itself, across the top.
    pub rect: Rect,
    /// Deal again.
    pub new: Rect,
    /// Take the last move back.
    pub undo: Rect,
    /// Turn one card from the stock, or three.
    pub turn: Rect,
    /// The next back.
    pub back: Rect,
    /// Leave: only where there is no window to close.
    pub quit: Option<Rect>,
    /// What the bar says of the game, between the buttons and the right edge.
    pub status: Rect,
}

/// Where everything is, in physical pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The surface.
    pub size: Size,
    /// The bar and what is on it.
    pub bar: Bar,
    /// A card.
    pub shape: Shape,
    /// The space between piles.
    pub gap: i32,
    /// The first column's left edge.
    pub left: i32,
    /// The top of the row of stock, waste and foundations.
    pub top: i32,
    /// The top of the seven piles.
    pub piles: i32,
    /// How far a face-down card shows above the next, before a tall pile is
    /// squeezed.
    pub down: i32,
    /// How far a face-up one does.
    pub up: i32,
    /// How far each fanned card of the waste shows beside the next.
    pub fan: i32,
}

impl Layout {
    /// Lays the table out for a surface of `size` physical pixels at `scale`.
    /// `quit` asks for a button to leave by, at the right end of the bar.
    pub fn new(size: Size, scale: f32, quit: bool) -> Self {
        // The one multiply, as in every example: logical in, physical out.
        let px = |v: i32| (v as f32 * scale + 0.5) as i32;
        let w = size.width as i32;
        let h = size.height as i32;
        let columns = PILES as i32;

        let rect = Rect::new(0, 0, w, px(48));
        let tall = px(32);
        let y = (rect.height - tall) / 2;
        let mut x = px(12);
        let mut place = |width: i32| {
            let r = Rect::new(x, y, px(width), tall);
            x += px(width) + px(8);
            r
        };
        let new = place(104);
        let undo = place(72);
        let turn = place(112);
        let back = place(132);
        let quit = quit.then(|| Rect::new(w - px(12) - px(72), y, px(72), tall));
        let right = quit.map_or(w - px(16), |q| q.x - px(12));
        // As wide as the longest thing it says and no wider: the clock
        // repaints this rectangle once a second for as long as a game lasts.
        let wide = px(260).min(right - x - px(4)).max(0);
        let status = Rect::new(right - wide, y, wide, tall);

        // Seven across, and two rows with room for a fan under the second.
        let gap = (w / 70).max(px(8));
        let by_width = (w - gap * (columns + 1)) / columns;
        let tallest = (h - rect.height - gap * 3) * 10 / 32;
        let by_height = tallest * ASPECT.0 / ASPECT.1;
        let width = by_width.min(by_height).max(24);
        let height = width * ASPECT.1 / ASPECT.0;
        let across = columns * width + (columns - 1) * gap;
        let top = rect.bottom() + gap;

        Self {
            size,
            bar: Bar {
                rect,
                new,
                undo,
                turn,
                back,
                quit,
                status,
            },
            shape: Shape {
                width,
                height,
                radius: (width * 5 / 100).max(2),
                line: px(1).max(1),
                // Never more than half the gap, so the ring round one pile is
                // not painted over its neighbour.
                margin: px(6).min(gap / 2).max(2),
                shadow: (px(4), px(6)),
            },
            gap,
            left: (w - across) / 2,
            top,
            piles: top + height + gap,
            down: (height * 8 / 100).max(2),
            up: (height * 20 / 100).max(4),
            fan: width * 22 / 100,
        }
    }

    fn column(&self, index: usize) -> i32 {
        self.left + index as i32 * (self.shape.width + self.gap)
    }

    /// Where a pile's first card lies.
    pub fn slot(&self, place: Place) -> Rect {
        let (x, y) = match place {
            Place::Stock => (self.column(0), self.top),
            Place::Waste => (self.column(1), self.top),
            Place::Foundation(i) => (self.column(3 + i.min(3)), self.top),
            Place::Tableau(i) => (self.column(i.min(PILES - 1)), self.piles),
        };
        Rect::new(x, y, self.shape.width, self.shape.height)
    }

    /// How far each face-down and each face-up card of `pile` shows above the
    /// next: less than usual when the pile would leave the surface.
    pub fn steps(&self, pile: &Pile) -> (i32, i32) {
        let room = self.size.height as i32 - self.gap - self.piles - self.shape.height;
        let need =
            pile.down.len() as i32 * self.down + pile.up.len().saturating_sub(1) as i32 * self.up;
        if need <= room || need == 0 {
            return (self.down, self.up);
        }
        let room = room.max(0);
        (
            (self.down * room / need).max(1),
            (self.up * room / need).max(2),
        )
    }

    /// How many of the waste's cards are fanned out: three when the stock
    /// turns three.
    fn fanned(game: &Game, waste: usize) -> usize {
        waste.min(if game.turn() == 3 { 3 } else { 1 })
    }

    /// What `place` shows, with its top `gone` cards off the table — carried,
    /// or on their way somewhere — and its top `chosen` picked up by the
    /// keyboard.
    pub fn look(&self, game: &Game, place: Place, gone: usize, chosen: usize, back: usize) -> Look {
        let lying = |all: usize| all.saturating_sub(gone);
        let mut look = Look {
            back,
            ..Look::default()
        };
        match place {
            Place::Stock => {
                if game.stock().is_empty() {
                    look.again = !game.waste().is_empty();
                } else {
                    // One back, however many lie under it.
                    look.spots.push(Spot {
                        card: None,
                        dx: 0,
                        dy: 0,
                    });
                }
            }
            Place::Waste => {
                let waste = game.waste();
                let lying = lying(waste.len());
                let shown = Self::fanned(game, lying);
                for (k, card) in waste[lying - shown..lying].iter().enumerate() {
                    look.spots.push(Spot {
                        card: Some(*card),
                        dx: k as i32 * self.fan,
                        dy: 0,
                    });
                }
            }
            Place::Foundation(i) => {
                let cards = game.foundation(i);
                // The top card only: the ones under it are exactly covered.
                if let Some(card) = lying(cards.len()).checked_sub(1).and_then(|n| cards.get(n)) {
                    look.spots.push(Spot {
                        card: Some(*card),
                        dx: 0,
                        dy: 0,
                    });
                }
            }
            Place::Tableau(i) => {
                let Some(pile) = game.tableau().get(i) else {
                    return look;
                };
                // The steps of the whole pile, the carried cards included: a
                // pile must not spread out because its top card was picked up.
                let (down, up) = self.steps(pile);
                let hidden = pile.down.len();
                for n in 0..lying(hidden + pile.up.len()) {
                    let face_up = n.saturating_sub(hidden);
                    look.spots.push(Spot {
                        card: n.checked_sub(hidden).and_then(|k| pile.up.get(k)).copied(),
                        dx: 0,
                        dy: n.min(hidden) as i32 * down + face_up as i32 * up,
                    });
                }
            }
        }
        let face_up = look.spots.iter().filter(|s| s.card.is_some()).count();
        look.chosen = chosen.min(face_up);
        // A pile with no card face down shows no back, and is not another
        // pile for the backs having changed.
        if face_up == look.spots.len() {
            look.back = 0;
        }
        look
    }

    /// The rectangle a pile's node is given: what its cards cover, and the
    /// ring's margin round that.
    ///
    /// **Tight on purpose.** The tree repaints a node when the pointer enters
    /// or leaves it, whether or not the widget draws anything differently for
    /// that, so a pile that claimed its whole column down to the foot of the
    /// surface would be repainted by a pointer passing under it. A pile that
    /// claims only its cards is repainted by a pointer that crosses its cards.
    ///
    /// The waste is the exception, and claims the room for three fanned cards
    /// whatever it holds, so that its node does not move every time the stock
    /// is turned.
    pub fn extent(&self, place: Place, look: &Look) -> Rect {
        let slot = self.slot(place);
        let covered = match place {
            Place::Waste => Rect::new(0, 0, self.shape.width + 2 * self.fan, self.shape.height),
            _ => look.covered(&self.shape),
        };
        covered.translate(slot.x, slot.y).inflate(self.shape.margin)
    }

    /// Where the first of the top `count` cards of `place` lies: where a hand
    /// takes them from, and where a card on its way there ends up.
    pub fn top(&self, game: &Game, place: Place, count: usize) -> Rect {
        let slot = self.slot(place);
        let look = self.look(game, place, 0, 0, 0);
        match look.spots.len().checked_sub(count.max(1)) {
            Some(n) => slot.translate(look.spots[n].dx, look.spots[n].dy),
            None => slot,
        }
    }

    /// The pile a card let go of at `at` is meant for: the column it is over,
    /// in the row it is in. Generous — half the gap either side, and all the
    /// way down the surface — because a drop that misses by a few pixels is
    /// the most irritating thing a card game can do.
    pub fn target(&self, at: Point) -> Option<Place> {
        let reach = self.gap / 2;
        let over = |index: usize| {
            let x = self.column(index);
            at.x >= x - reach && at.x < x + self.shape.width + reach
        };
        if at.y < self.piles - reach {
            return (0..4)
                .find(|&i| over(3 + i) && at.y >= self.top - reach)
                .map(Place::Foundation);
        }
        (0..PILES).find(|&i| over(i)).map(Place::Tableau)
    }

    /// The node carried cards are painted in, with its first card at `at`:
    /// `count` cards fanned down, and room for the shadow they cast.
    pub fn hand(&self, at: Point, count: usize) -> Rect {
        let tail = count.saturating_sub(1) as i32 * self.up;
        Rect::new(
            at.x,
            at.y,
            self.shape.width + self.shape.shadow.0 + 1,
            self.shape.height + tail + self.shape.shadow.1 + 1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, PLACES, Spot, index, places};
    use crate::game::{Game, PILES, Place};
    use denise::{Point, Rect, Size};

    fn layout(w: u32, h: u32) -> Layout {
        Layout::new(Size::new(w, h), 1.0, false)
    }

    fn centre(r: Rect) -> Point {
        Point::new(r.x + r.width / 2, r.y + r.height / 2)
    }

    #[test]
    fn every_place_has_one_position_and_they_are_in_order() {
        assert_eq!(places().count(), PLACES);
        for (i, place) in places().enumerate() {
            assert_eq!(index(place), i, "{place:?}");
        }
    }

    #[test]
    fn seven_cards_fit_across_and_two_rows_down_on_any_surface() {
        for (w, h) in [
            (640, 480),
            (800, 480),
            (1100, 760),
            (1920, 1080),
            (900, 1600),
            (2560, 700),
            (3840, 2160),
        ] {
            for scale in [1.0, 1.5, 2.0] {
                let l = Layout::new(Size::new(w, h), scale, true);
                let last = l.slot(Place::Tableau(PILES - 1));
                assert!(l.left >= 0 && last.right() <= w as i32, "{w}x{h}");
                assert!(l.piles + l.shape.height < h as i32, "{w}x{h}");
                assert_eq!(l.shape.height, l.shape.width * 436 / 300);
                // The foundations stand over the last four piles.
                assert_eq!(l.slot(Place::Foundation(3)).x, last.x);
                // The ring round one pile stops short of the next pile's.
                assert!(2 * l.shape.margin <= l.gap, "{w}x{h} at {scale}");
            }
        }
    }

    #[test]
    fn every_node_is_on_the_surface_and_no_two_piles_claim_the_same_pixels() {
        for (w, h) in [(640, 480), (800, 480), (1100, 760), (1920, 1080)] {
            let l = layout(w, h);
            let mut game = Game::new(3, 3);
            game.deal();
            let surface = Rect::new(0, 0, w as i32, h as i32);
            let extents: Vec<Rect> = places()
                .map(|place| l.extent(place, &l.look(&game, place, 0, 0, 0)))
                .collect();
            for (i, a) in extents.iter().enumerate() {
                assert!(surface.contains_rect(a), "{w}x{h}: {a:?}");
                assert!(a.y >= l.bar.rect.bottom(), "{w}x{h}: under the bar");
                for b in &extents[i + 1..] {
                    assert!(!a.intersects(b), "{w}x{h}: {a:?} and {b:?}");
                }
            }
        }
    }

    #[test]
    fn the_bar_fits_its_buttons_and_leaves_room_to_say_how_the_game_stands() {
        for (w, quit) in [(800, true), (1100, false), (1920, true)] {
            let bar = Layout::new(Size::new(w, 600), 1.0, quit).bar;
            let buttons = [bar.new, bar.undo, bar.turn, bar.back];
            for pair in buttons.windows(2) {
                assert!(pair[0].right() < pair[1].x);
            }
            assert!(bar.status.x > bar.back.right());
            assert!(bar.status.width >= 200, "{w}: {}", bar.status.width);
            assert_eq!(bar.quit.is_some(), quit);
            if let Some(q) = bar.quit {
                assert!(q.right() <= w as i32 && bar.status.right() < q.x);
            }
        }
    }

    #[test]
    fn a_tall_pile_is_squeezed_to_stay_on_the_surface() {
        let l = layout(1000, 500);
        let mut game = Game::new(2, 1);
        for i in 0..PILES {
            let look = l.look(&game, Place::Tableau(i), 0, 0, 0);
            let extent = l.extent(Place::Tableau(i), &look);
            assert!(extent.bottom() <= 500, "pile {i}");
        }
        // Turning the stock does not move a pile.
        let before = l.look(&game, Place::Tableau(6), 0, 0, 0);
        game.deal();
        assert_eq!(l.look(&game, Place::Tableau(6), 0, 0, 0), before);
        // Nor does picking up its top card spread out the rest.
        let held = l.look(&game, Place::Tableau(6), 1, 0, 0);
        assert_eq!(held.spots[..], before.spots[..6]);
    }

    #[test]
    fn what_a_pile_shows_is_what_a_press_on_it_means() {
        let l = layout(1200, 800);
        let mut game = Game::new(4, 3);
        let s = &l.shape;
        // The stock is one back, and the waste is empty.
        let stock = l.look(&game, Place::Stock, 0, 0, 0);
        assert_eq!(stock.spots.len(), 1);
        assert_eq!(stock.hit(s, Point::new(5, 5)).map(|(n, _)| n), Some(0));
        assert!(l.look(&game, Place::Waste, 0, 0, 0).spots.is_empty());

        game.deal();
        // Three are fanned, and only the last is the top card.
        let waste = l.look(&game, Place::Waste, 0, 0, 0);
        assert_eq!(waste.spots.len(), 3);
        let top = waste.spots[2];
        assert_eq!(top.dx, 2 * l.fan);
        let on_top = Point::new(top.dx + s.width - 2, 5);
        assert_eq!(waste.hit(s, on_top).map(|(n, _)| n), Some(1));
        // The strip of the first one that shows is not the top card.
        assert_eq!(waste.hit(s, Point::new(2, 5)).map(|(n, _)| n), Some(3));
        assert_eq!(waste.hit(s, Point::new(-3, 5)), None);
        assert_eq!(
            l.top(&game, Place::Waste, 1).x,
            l.slot(Place::Waste).x + 2 * l.fan
        );

        // The last pile: six face down, one up.
        let pile = l.look(&game, Place::Tableau(6), 0, 0, 0);
        assert_eq!(pile.spots.len(), 7);
        assert_eq!(pile.hit(s, Point::new(5, 1)).map(|(n, _)| n), Some(0));
        let last = pile.spots[6];
        assert_eq!(
            pile.hit(s, Point::new(s.width / 2, last.dy + s.height / 2)),
            Some((1, last))
        );
        assert_eq!(pile.hit(s, Point::new(5, last.dy + s.height + 1)), None);
        // An empty foundation shows nothing and covers its slot.
        let home = l.look(&game, Place::Foundation(2), 0, 0, 0);
        assert!(home.spots.is_empty());
        assert_eq!(home.covered(s), Rect::new(0, 0, s.width, s.height));
    }

    #[test]
    fn cards_in_the_hand_are_gone_from_their_pile_and_chosen_ones_are_not() {
        let l = layout(1200, 800);
        let game = Game::new(4, 1);
        let whole = l.look(&game, Place::Tableau(3), 0, 0, 0);
        let held = l.look(&game, Place::Tableau(3), 1, 0, 0);
        assert_eq!(held.spots.len(), whole.spots.len() - 1);
        assert!(held.spots.iter().all(|s| s.card.is_none()));
        // Chosen cards stay, and no more can be chosen than lie face up.
        let chosen = l.look(&game, Place::Tableau(3), 0, 5, 0);
        assert_eq!(chosen.spots, whole.spots);
        assert_eq!(chosen.chosen, 1);
        assert_ne!(chosen, whole);
        // Another back is another look for a pile that shows one, which is
        // what makes changing it repaint every such pile and no other.
        assert_ne!(l.look(&game, Place::Tableau(3), 0, 0, 1), whole);
        let first = l.look(&game, Place::Tableau(0), 0, 0, 0);
        assert_eq!(l.look(&game, Place::Tableau(0), 0, 0, 1), first);
        assert_eq!(
            whole.spots[0],
            Spot {
                card: None,
                dx: 0,
                dy: 0
            }
        );
    }

    #[test]
    fn a_card_let_go_belongs_to_the_column_it_is_over() {
        let l = layout(1200, 800);
        for i in 0..PILES {
            let slot = l.slot(Place::Tableau(i));
            assert_eq!(l.target(centre(slot)), Some(Place::Tableau(i)));
            // Far down the surface is still that pile.
            assert_eq!(
                l.target(Point::new(slot.x + 3, 790)),
                Some(Place::Tableau(i))
            );
        }
        for i in 0..4 {
            let slot = l.slot(Place::Foundation(i));
            assert_eq!(l.target(centre(slot)), Some(Place::Foundation(i)));
        }
        assert_eq!(l.target(centre(l.slot(Place::Stock))), None);
        assert_eq!(l.target(centre(l.slot(Place::Waste))), None);
        assert_eq!(l.target(Point::new(-50, 600)), None);
    }

    #[test]
    fn the_hand_has_room_for_its_cards_and_their_shadow() {
        let l = layout(1200, 800);
        let one = l.hand(Point::new(10, 20), 1);
        assert_eq!((one.x, one.y), (10, 20));
        assert!(one.width > l.shape.width && one.height > l.shape.height);
        assert_eq!(l.hand(Point::new(0, 0), 4).height, one.height + 3 * l.up);
    }
}
