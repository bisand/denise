//! Klondike: what is on the table, and what may be done with it.
//!
//! Seven piles, the first of one card and the last of seven, each with its
//! last card face up; the rest of the deck is the stock. Cards are built
//! down the piles in alternating colours and up the four foundations by
//! suit, from the ace. The stock is turned onto the waste one card at a
//! time, or three, and turned over again when it runs out, as often as
//! wanted.
//!
//! Nothing here knows about a widget, a pixel or a pointer. A move is asked
//! for by where the cards are and where they should go, and either happens or
//! does not — which is what lets every rule be tested with no display, and
//! what keeps the rest of the example about the toolkit and not about cards.

use crate::cards::{Card, KING, shuffled};

/// A pile of the tableau: the cards face down, and on them those face up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pile {
    /// Face down, the last nearest the top.
    pub down: Vec<Card>,
    /// Face up, the last on top.
    pub up: Vec<Card>,
}

/// Somewhere cards lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    /// The deck not yet turned.
    Stock,
    /// What has been turned from it.
    Waste,
    /// One of the four foundations, by [`Suit::index`](crate::cards::Suit::index).
    Foundation(usize),
    /// One of the seven piles.
    Tableau(usize),
}

/// How many piles the tableau has.
pub const PILES: usize = 7;

/// Everything on the table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Table {
    stock: Vec<Card>,
    waste: Vec<Card>,
    foundations: [Vec<Card>; 4],
    tableau: [Pile; PILES],
}

/// A game of Klondike.
#[derive(Debug, Clone)]
pub struct Game {
    table: Table,
    /// The table before each move made, the last the most recent.
    before: Vec<Table>,
    turn: usize,
    moves: u32,
}

impl Game {
    /// Deal a game from the deck `seed` shuffles, turning `turn` cards from
    /// the stock at a time: one or three.
    #[must_use]
    pub fn new(seed: u64, turn: usize) -> Self {
        let mut cards = shuffled(seed);
        let mut tableau: [Pile; PILES] = Default::default();
        for (i, pile) in tableau.iter_mut().enumerate() {
            pile.down = cards.split_off(cards.len() - i);
            pile.up = cards.split_off(cards.len() - 1);
        }
        Self {
            table: Table {
                stock: cards,
                waste: Vec::new(),
                foundations: Default::default(),
                tableau,
            },
            before: Vec::new(),
            turn: turn.clamp(1, 3),
            moves: 0,
        }
    }

    /// The stock, its top card last.
    #[must_use]
    pub fn stock(&self) -> &[Card] {
        &self.table.stock
    }

    /// The waste, its top card last.
    #[must_use]
    pub fn waste(&self) -> &[Card] {
        &self.table.waste
    }

    /// A foundation's cards, the ace first. Empty for one that is not there.
    #[must_use]
    pub fn foundation(&self, index: usize) -> &[Card] {
        self.table.foundations.get(index).map_or(&[], Vec::as_slice)
    }

    /// The seven piles.
    #[must_use]
    pub fn tableau(&self) -> &[Pile; PILES] {
        &self.table.tableau
    }

    /// How many cards the stock turns at a time.
    #[must_use]
    pub fn turn(&self) -> usize {
        self.turn
    }

    /// How many moves have been made, turns of the stock among them.
    #[must_use]
    pub fn moves(&self) -> u32 {
        self.moves
    }

    /// Whether there is a move to take back.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.before.is_empty()
    }

    /// Whether every card is on a foundation.
    #[must_use]
    pub fn won(&self) -> bool {
        self.table
            .foundations
            .iter()
            .all(|f| f.len() == usize::from(KING))
    }

    /// The cards face up at `place`, the top one last: all that could be
    /// picked up from it, were the rules to allow each.
    #[must_use]
    pub fn face_up(&self, place: Place) -> &[Card] {
        match place {
            Place::Stock => &[],
            Place::Waste => &self.table.waste,
            Place::Foundation(i) => self.foundation(i),
            Place::Tableau(i) => self.table.tableau.get(i).map_or(&[], |p| p.up.as_slice()),
        }
    }

    /// The top `count` cards of `place`, if that many may be picked up
    /// together: one from the waste or a foundation, any number of a pile's
    /// face-up cards.
    #[must_use]
    pub fn held(&self, place: Place, count: usize) -> Option<&[Card]> {
        let cards = self.face_up(place);
        let most = match place {
            Place::Stock => 0,
            Place::Waste | Place::Foundation(_) => 1,
            Place::Tableau(_) => cards.len(),
        };
        (count >= 1 && count <= most.min(cards.len())).then(|| &cards[cards.len() - count..])
    }

    /// Where the top `count` cards of `from` come to rest when put on `to`,
    /// if the rules allow it. Any foundation stands for the card's own.
    #[must_use]
    pub fn landing(&self, from: Place, count: usize, to: Place) -> Option<Place> {
        let cards = self.held(from, count)?;
        let first = *cards.first()?;
        match to {
            Place::Foundation(_) => {
                let home = Place::Foundation(first.suit.index());
                let there = self.foundation(first.suit.index());
                (count == 1 && from != home && usize::from(first.rank) == there.len() + 1)
                    .then_some(home)
            }
            Place::Tableau(i) => {
                let pile = self.table.tableau.get(i)?;
                if from == to {
                    return None;
                }
                let fits = match pile.up.last() {
                    Some(top) => top.suit.red() != first.suit.red() && top.rank == first.rank + 1,
                    None => pile.down.is_empty() && first.rank == KING,
                };
                fits.then_some(to)
            }
            Place::Stock | Place::Waste => None,
        }
    }

    /// Put the top `count` cards of `from` on `to`. Returns where they went,
    /// or `None` with nothing changed if the rules do not allow it.
    pub fn play(&mut self, from: Place, count: usize, to: Place) -> Option<Place> {
        let landing = self.landing(from, count, to)?;
        self.remember();
        let source = self.cards_mut(from)?;
        let cards = source.split_off(source.len() - count);
        self.cards_mut(landing)?.extend(cards);
        if let Place::Tableau(i) = from {
            let pile = &mut self.table.tableau[i];
            if pile.up.is_empty()
                && let Some(card) = pile.down.pop()
            {
                pile.up.push(card);
            }
        }
        Some(landing)
    }

    /// Turn the stock: its next cards onto the waste, or the waste back over
    /// when it has run out. `false` when both are empty.
    pub fn deal(&mut self) -> bool {
        if self.table.stock.is_empty() && self.table.waste.is_empty() {
            return false;
        }
        self.remember();
        if self.table.stock.is_empty() {
            self.table.stock = std::mem::take(&mut self.table.waste);
            self.table.stock.reverse();
            return true;
        }
        for _ in 0..self.turn {
            if let Some(card) = self.table.stock.pop() {
                self.table.waste.push(card);
            }
        }
        true
    }

    /// The foundation the top card of `from` may go to now.
    #[must_use]
    pub fn home(&self, from: Place) -> Option<Place> {
        self.landing(from, 1, Place::Foundation(0))
    }

    /// Whether what is left plays itself: nothing face down, nothing in the
    /// stock, and nothing but the top card left in the waste.
    #[must_use]
    pub fn runs_out(&self) -> bool {
        !self.won()
            && self.table.stock.is_empty()
            && self.table.waste.len() <= 1
            && self.table.tableau.iter().all(|p| p.down.is_empty())
    }

    /// A place whose top card may go to its foundation, the lowest card
    /// first: what [`Game::runs_out`] plays next.
    #[must_use]
    pub fn next_home(&self) -> Option<Place> {
        std::iter::once(Place::Waste)
            .chain((0..PILES).map(Place::Tableau))
            .filter(|&from| self.home(from).is_some())
            .min_by_key(|&from| self.face_up(from).last().map_or(u8::MAX, |c| c.rank))
    }

    /// Take the last move back. `false` when there is none.
    pub fn undo(&mut self) -> bool {
        match self.before.pop() {
            Some(table) => {
                self.table = table;
                self.moves = self.moves.saturating_sub(1);
                true
            }
            None => false,
        }
    }

    fn remember(&mut self) {
        self.before.push(self.table.clone());
        self.moves = self.moves.saturating_add(1);
    }

    fn cards_mut(&mut self, place: Place) -> Option<&mut Vec<Card>> {
        match place {
            Place::Stock => None,
            Place::Waste => Some(&mut self.table.waste),
            Place::Foundation(i) => self.table.foundations.get_mut(i),
            Place::Tableau(i) => self.table.tableau.get_mut(i).map(|p| &mut p.up),
        }
    }

    /// A game set out by hand, for tests: piles of face-down and face-up
    /// cards, a stock, a waste, and how many cards of each suit are already
    /// home.
    #[cfg(test)]
    #[must_use]
    pub fn set_out(
        tableau: [Pile; PILES],
        stock: Vec<Card>,
        waste: Vec<Card>,
        home: [u8; 4],
        turn: usize,
    ) -> Self {
        let foundations = crate::cards::Suit::ALL.map(|suit| {
            (1..=home[suit.index()].min(KING))
                .map(|rank| Card { suit, rank })
                .collect()
        });
        Self {
            table: Table {
                stock,
                waste,
                foundations,
                tableau,
            },
            before: Vec::new(),
            turn: turn.clamp(1, 3),
            moves: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Game, PILES, Pile, Place};
    use crate::cards::{Card, KING, Rng, Suit};
    use std::collections::HashSet;

    fn card(suit: Suit, rank: u8) -> Card {
        Card { suit, rank }
    }

    fn up(cards: &[Card]) -> Pile {
        Pile {
            down: Vec::new(),
            up: cards.to_vec(),
        }
    }

    fn every_card(game: &Game) -> Vec<Card> {
        let mut all: Vec<Card> = game.stock().to_vec();
        all.extend(game.waste());
        for i in 0..4 {
            all.extend(game.foundation(i));
        }
        for pile in game.tableau() {
            all.extend(&pile.down);
            all.extend(&pile.up);
        }
        all
    }

    #[test]
    fn a_deal_is_seven_piles_one_card_up_on_each_and_the_rest_in_the_stock() {
        let game = Game::new(1, 1);
        for (i, pile) in game.tableau().iter().enumerate() {
            assert_eq!(pile.down.len(), i);
            assert_eq!(pile.up.len(), 1);
        }
        assert_eq!(game.stock().len(), 24);
        assert!(game.waste().is_empty());
        assert_eq!(every_card(&game).iter().collect::<HashSet<_>>().len(), 52);
        assert!(!game.won());
        assert!(!game.can_undo());
    }

    #[test]
    fn the_stock_turns_one_or_three_and_comes_round_again() {
        let mut one = Game::new(3, 1);
        let top = *one.stock().last().unwrap();
        assert!(one.deal());
        assert_eq!(one.waste(), [top]);
        for _ in 0..23 {
            assert!(one.deal());
        }
        assert!(one.stock().is_empty());
        assert_eq!(one.waste().len(), 24);
        // Turned over, the first card turned is the first again.
        assert!(one.deal());
        assert!(one.waste().is_empty());
        assert_eq!(*one.stock().last().unwrap(), top);

        let mut three = Game::new(3, 3);
        assert!(three.deal());
        assert_eq!(three.waste().len(), 3);
        assert_eq!(three.waste()[0], top);
    }

    #[test]
    fn an_empty_stock_and_waste_do_not_turn() {
        let mut game = Game::set_out(Default::default(), Vec::new(), Vec::new(), [0; 4], 1);
        assert!(!game.deal());
        assert_eq!(game.moves(), 0);
    }

    #[test]
    fn piles_build_down_in_alternating_colours() {
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = up(&[card(Suit::Spades, 8)]);
        piles[1] = up(&[card(Suit::Hearts, 7)]);
        piles[2] = up(&[card(Suit::Clubs, 7)]);
        piles[3] = up(&[card(Suit::Diamonds, 9)]);
        let mut game = Game::set_out(piles, Vec::new(), Vec::new(), [0; 4], 1);
        let (t0, t1, t2, t3) = (
            Place::Tableau(0),
            Place::Tableau(1),
            Place::Tableau(2),
            Place::Tableau(3),
        );
        assert_eq!(game.landing(t2, 1, t0), None, "black on black");
        assert_eq!(game.landing(t1, 1, t3), None, "a seven on a nine");
        assert_eq!(game.landing(t1, 1, t1), None, "onto itself");
        assert_eq!(game.play(t1, 1, t0), Some(t0));
        assert_eq!(game.tableau()[0].up.len(), 2);
        // The eight and the seven go to the nine together.
        assert_eq!(game.landing(t0, 1, t3), None);
        assert_eq!(game.play(t0, 2, t3), Some(t3));
        assert_eq!(game.tableau()[3].up.len(), 3);
        assert_eq!(game.moves(), 2);
    }

    #[test]
    fn only_a_king_goes_to_an_empty_pile() {
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = up(&[card(Suit::Hearts, KING), card(Suit::Spades, 12)]);
        piles[1] = up(&[card(Suit::Clubs, 5)]);
        let mut game = Game::set_out(piles, Vec::new(), Vec::new(), [0; 4], 1);
        assert_eq!(game.landing(Place::Tableau(1), 1, Place::Tableau(2)), None);
        assert_eq!(game.landing(Place::Tableau(0), 1, Place::Tableau(2)), None);
        assert_eq!(
            game.play(Place::Tableau(0), 2, Place::Tableau(2)),
            Some(Place::Tableau(2))
        );
    }

    #[test]
    fn the_card_under_the_last_one_taken_is_turned_up() {
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = Pile {
            down: vec![card(Suit::Clubs, 2), card(Suit::Clubs, 9)],
            up: vec![card(Suit::Hearts, 4)],
        };
        piles[1] = up(&[card(Suit::Spades, 5)]);
        let mut game = Game::set_out(piles, Vec::new(), Vec::new(), [0; 4], 1);
        assert!(game.play(Place::Tableau(0), 1, Place::Tableau(1)).is_some());
        let pile = &game.tableau()[0];
        assert_eq!(pile.up, [card(Suit::Clubs, 9)]);
        assert_eq!(pile.down, [card(Suit::Clubs, 2)]);
    }

    #[test]
    fn foundations_build_up_by_suit_whichever_one_is_pointed_at() {
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = up(&[card(Suit::Hearts, 2), card(Suit::Hearts, 1)]);
        piles[1] = up(&[card(Suit::Spades, 2)]);
        let mut game = Game::set_out(piles, Vec::new(), Vec::new(), [0; 4], 1);
        let hearts = Place::Foundation(Suit::Hearts.index());
        assert_eq!(game.home(Place::Tableau(1)), None, "no ace under it");
        assert_eq!(
            game.landing(Place::Tableau(0), 2, hearts),
            None,
            "two at once"
        );
        assert_eq!(
            game.play(Place::Tableau(0), 1, Place::Foundation(0)),
            Some(hearts)
        );
        assert_eq!(game.home(Place::Tableau(0)), Some(hearts));
        assert_eq!(game.play(Place::Tableau(0), 1, hearts), Some(hearts));
        assert_eq!(game.foundation(Suit::Hearts.index()).len(), 2);
        // And back down again, onto a black three.
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = up(&[card(Suit::Spades, 3)]);
        let mut game = Game::set_out(piles, Vec::new(), Vec::new(), [0, 0, 2, 0], 1);
        assert_eq!(game.landing(hearts, 1, hearts), None);
        assert_eq!(
            game.play(hearts, 1, Place::Tableau(0)),
            Some(Place::Tableau(0))
        );
    }

    #[test]
    fn the_waste_gives_its_top_card_only() {
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = up(&[card(Suit::Spades, 6)]);
        let waste = vec![card(Suit::Hearts, 5), card(Suit::Diamonds, 5)];
        let mut game = Game::set_out(piles, Vec::new(), waste, [0; 4], 3);
        assert!(game.held(Place::Waste, 2).is_none());
        assert!(game.held(Place::Stock, 1).is_none());
        assert_eq!(
            game.play(Place::Waste, 1, Place::Tableau(0)),
            Some(Place::Tableau(0))
        );
        assert_eq!(game.waste(), [card(Suit::Hearts, 5)]);
    }

    #[test]
    fn a_move_refused_changes_nothing_and_a_move_made_can_be_taken_back() {
        let mut game = Game::new(11, 1);
        let dealt = game.clone();
        assert_eq!(game.play(Place::Stock, 1, Place::Tableau(0)), None);
        assert_eq!(game.play(Place::Tableau(0), 5, Place::Tableau(1)), None);
        assert_eq!(game.moves(), 0);
        assert!(!game.undo());
        assert!(game.deal());
        assert!(game.deal());
        assert_eq!(game.moves(), 2);
        assert!(game.undo());
        assert!(game.undo());
        assert_eq!(game.table, dealt.table);
        assert_eq!(game.moves(), 0);
    }

    #[test]
    fn a_game_with_nothing_hidden_runs_out_lowest_card_first_and_is_won() {
        let mut piles: [Pile; PILES] = Default::default();
        piles[0] = up(&[card(Suit::Spades, KING), card(Suit::Hearts, 12)]);
        piles[1] = up(&[card(Suit::Hearts, KING), card(Suit::Spades, 12)]);
        let waste = vec![card(Suit::Hearts, 11)];
        let mut game = Game::set_out(piles, Vec::new(), waste, [KING, KING, 10, 11], 1);
        assert!(game.runs_out());
        assert_eq!(game.next_home(), Some(Place::Waste));
        let mut played = 0;
        while let Some(from) = game.next_home() {
            assert!(game.play(from, 1, Place::Foundation(0)).is_some());
            played += 1;
        }
        assert_eq!(played, 5);
        assert!(game.won());
        assert!(!game.runs_out());
    }

    #[test]
    fn a_game_with_a_card_face_down_does_not_run_out() {
        assert!(!Game::new(5, 1).runs_out());
    }

    /// Whatever is tried, in whatever order, the table holds the same
    /// fifty-two cards and every pile's face-up cards still build down.
    #[test]
    fn no_sequence_of_moves_loses_a_card_or_breaks_a_pile() {
        let places: Vec<Place> = [Place::Stock, Place::Waste]
            .into_iter()
            .chain((0..4).map(Place::Foundation))
            .chain((0..PILES).map(Place::Tableau))
            .collect();
        for seed in 0..20 {
            let mut game = Game::new(seed, 1 + usize::try_from(seed % 2).unwrap() * 2);
            let mut rng = Rng::new(seed);
            for _ in 0..600 {
                match rng.below(8) {
                    0 => {
                        game.deal();
                    }
                    1 => {
                        game.undo();
                    }
                    _ => {
                        let from = places[rng.below(places.len())];
                        let to = places[rng.below(places.len())];
                        game.play(from, 1 + rng.below(4), to);
                    }
                }
            }
            assert_eq!(every_card(&game).len(), 52);
            assert_eq!(every_card(&game).iter().collect::<HashSet<_>>().len(), 52);
            for pile in game.tableau() {
                assert!(pile.down.is_empty() || !pile.up.is_empty());
                for pair in pile.up.windows(2) {
                    assert_eq!(pair[0].rank, pair[1].rank + 1);
                    assert_ne!(pair[0].suit.red(), pair[1].suit.red());
                }
            }
            for i in 0..4 {
                for (n, c) in game.foundation(i).iter().enumerate() {
                    assert_eq!(c.suit.index(), i);
                    assert_eq!(usize::from(c.rank), n + 1);
                }
            }
        }
    }
}
