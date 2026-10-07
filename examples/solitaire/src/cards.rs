//! A deck of cards, and shuffling it.

/// A suit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Suit {
    /// ♣
    Clubs,
    /// ♦
    Diamonds,
    /// ♥
    Hearts,
    /// ♠
    Spades,
}

impl Suit {
    /// Every suit, in the order the foundations stand in.
    pub const ALL: [Self; 4] = [Self::Clubs, Self::Diamonds, Self::Hearts, Self::Spades];

    /// Whether it is one of the red two.
    #[must_use]
    pub fn red(self) -> bool {
        matches!(self, Self::Diamonds | Self::Hearts)
    }

    /// Its place among [`Suit::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Clubs => 0,
            Self::Diamonds => 1,
            Self::Hearts => 2,
            Self::Spades => 3,
        }
    }
}

/// The king's rank; the ace's is 1.
pub const KING: u8 = 13;

/// A card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Card {
    /// Its suit.
    pub suit: Suit,
    /// 1 for the ace to 13 for the king.
    pub rank: u8,
}

impl Card {
    /// Its place among the fifty-two: clubs, diamonds, hearts, spades, each
    /// ace to king.
    #[must_use]
    pub fn index(self) -> usize {
        self.suit.index() * usize::from(KING) + usize::from(self.rank.clamp(1, KING)) - 1
    }
}

/// The fifty-two, in order.
#[must_use]
pub fn deck() -> Vec<Card> {
    Suit::ALL
        .into_iter()
        .flat_map(|suit| (1..=KING).map(move |rank| Card { suit, rank }))
        .collect()
}

/// Numbers that look random and are the same for the same seed: splitmix64.
/// A game of cards needs no more, and a seed makes a deal that can be dealt
/// again.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator from a seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next number.
    pub fn number(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n`, which must not be zero.
    pub fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n.max(1)).unwrap_or(1);
        usize::try_from(self.number() % n).unwrap_or(0)
    }
}

/// The deck, shuffled by `seed`.
#[must_use]
pub fn shuffled(seed: u64) -> Vec<Card> {
    let mut cards = deck();
    let mut rng = Rng::new(seed);
    for i in (1..cards.len()).rev() {
        cards.swap(i, rng.below(i + 1));
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::{Card, KING, Suit, deck, shuffled};
    use std::collections::HashSet;

    #[test]
    fn the_deck_is_fifty_two_different_cards_in_order() {
        let cards = deck();
        assert_eq!(cards.len(), 52);
        for (i, card) in cards.iter().enumerate() {
            assert_eq!(card.index(), i);
        }
        assert_eq!(
            cards[51],
            Card {
                suit: Suit::Spades,
                rank: KING
            }
        );
    }

    #[test]
    fn a_shuffle_keeps_every_card_and_follows_its_seed() {
        let a = shuffled(7);
        assert_eq!(a.iter().collect::<HashSet<_>>().len(), 52);
        assert_eq!(a, shuffled(7));
        assert_ne!(a, shuffled(8));
        assert_ne!(a, deck());
    }
}
