//! The cards' pictures: PNGs in the binary, scaled to the size they are drawn at.
//!
//! Fifty-two faces of 300 by 436 and three backs are carried in the program,
//! about 900 KiB of it — `cards/README.md` says where each came from.
//! `denise-image` decodes them, which is the whole reason this example depends
//! on it: one feature, `png`, and the decoder the toolkit already ships.
//!
//! # Why the scaling is done here and not by the painter
//!
//! [`Pen::blit_scaled`](denise::Pen::blit_scaled) samples the nearest pixel.
//! That is the right trade for a photograph stretched a little and the wrong
//! one for a playing card shrunk to a third: the pips survive and the hairlines
//! of a court card do not. So each picture is resampled **by area** to exactly
//! the size of a card on this table, once, the first time a card shows it, and
//! kept until the table changes size. After that a card is a straight copy of
//! rows, which is the cheapest thing the rasteriser does.
//!
//! A picture is handed out as an [`Rc`], so that a pile holds the pixels it
//! paints without holding the cache they came from: a widget owns its own state,
//! and a widget that reached back into the application for its pictures while
//! painting would not.

use std::rc::Rc;

use denise::Size;

use crate::cards::Card;

/// A picture at the size it is drawn: opaque `0xFFRRGGBB`, row by row.
#[derive(Debug, PartialEq, Eq)]
pub struct Face {
    /// The pixels, `size.width` to a row.
    pub pixels: Vec<u32>,
    /// How big it is.
    pub size: Size,
}

/// The backs there are to choose from: what each is called, and its picture.
///
/// Three, because a toolkit's example wants to show that the choice exists and
/// not to be a deck shop. The last is the one a toolkit named Denise could not
/// leave out.
pub const BACKS: [(&str, &[u8]); 3] = [
    ("Mist", include_bytes!("../cards/back-01.png")),
    ("Crimson", include_bytes!("../cards/back-03.png")),
    ("16-bit", include_bytes!("../cards/back-12.png")),
];

macro_rules! pictures {
    ($($name:literal),* $(,)?) => {
        [$(include_bytes!(concat!("../cards/", $name, ".png")).as_slice()),*]
    };
}

/// The PNGs, by [`Card::index`].
const PICTURES: [&[u8]; 52] = pictures![
    "c01", "c02", "c03", "c04", "c05", "c06", "c07", "c08", "c09", "c10", "c11", "c12", "c13",
    "d01", "d02", "d03", "d04", "d05", "d06", "d07", "d08", "d09", "d10", "d11", "d12", "d13",
    "h01", "h02", "h03", "h04", "h05", "h06", "h07", "h08", "h09", "h10", "h11", "h12", "h13",
    "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09", "s10", "s11", "s12", "s13",
];

/// A picture's width over its height, as 300 to 436: the shape of a card.
pub const ASPECT: (i32, i32) = (300, 436);

/// The pictures at the size this table draws them.
///
/// Decoded lazily and one at a time. A deal shows seven faces and one back, so
/// a table is on screen after eight pictures rather than fifty-five — which on
/// a Raspberry Pi 3 is the difference between a first frame and a pause.
#[derive(Debug, Default)]
pub struct Faces {
    size: Size,
    scaled: Vec<Option<Rc<Face>>>,
    back: Option<(usize, Rc<Face>)>,
}

impl Faces {
    /// `card`'s face at `size`. `None` only if its PNG will not decode, which
    /// the tests below say does not happen.
    pub fn card(&mut self, card: Card, size: Size) -> Option<Rc<Face>> {
        if self.size != size || self.scaled.len() != PICTURES.len() {
            self.size = size;
            self.scaled = vec![None; PICTURES.len()];
        }
        let slot = self.scaled.get_mut(card.index())?;
        if slot.is_none() {
            *slot = picture(PICTURES[card.index()], size);
        }
        slot.clone()
    }

    /// The `which`th of [`BACKS`] at `size`.
    pub fn back(&mut self, which: usize, size: Size) -> Option<Rc<Face>> {
        let kept = self
            .back
            .as_ref()
            .is_some_and(|(index, face)| *index == which && face.size == size);
        if !kept {
            let (_, bytes) = BACKS.get(which)?;
            self.back = picture(bytes, size).map(|face| (which, face));
        }
        self.back.as_ref().map(|(_, face)| Rc::clone(face))
    }
}

fn picture(bytes: &[u8], size: Size) -> Option<Rc<Face>> {
    let decoded = denise_image::decode_png(bytes).ok()?;
    Some(Rc::new(scale(decoded.pixels(), decoded.size(), size)))
}

/// Area-averages `src` to exactly `to`.
///
/// In 32 bits throughout. A card's picture is a hundred and thirty thousand
/// pixels of at most 255 a channel, which a `u32` holds many times over, and on
/// a 32-bit ARM a sum in 64 bits is what turns a card coming face up for the
/// first time into something you can see happen.
///
/// The pictures are opaque, so the premultiplied words `denise-image` returns
/// are plain colour and averaging the channels is all there is to it. Alpha is
/// set, not carried: a card has no transparent pixels to average wrongly.
fn scale(words: &[u32], from: Size, to: Size) -> Face {
    let (w, h) = (from.width as usize, from.height as usize);
    let size = Size::new(to.width.max(1), to.height.max(1));
    let (dw, dh) = (size.width as usize, size.height as usize);
    let mut pixels = vec![0xFFFF_FFFFu32; dw * dh];
    // A picture too large to sum in 32 bits is not one of ours.
    if w == 0 || h == 0 || words.len() < w * h || w * h > (u32::MAX / 255) as usize {
        return Face { pixels, size };
    }
    // The source columns each destination column covers, at least one. The
    // same for every row, so worked out once.
    let columns: Vec<(usize, usize)> = (0..dw)
        .map(|dx| {
            let x0 = dx * w / dw;
            (x0, ((dx + 1) * w).div_ceil(dw).max(x0 + 1).min(w))
        })
        .collect();
    for (dy, row) in pixels.chunks_exact_mut(dw).enumerate() {
        let y0 = dy * h / dh;
        let y1 = ((dy + 1) * h).div_ceil(dh).max(y0 + 1).min(h);
        for (out, &(x0, x1)) in row.iter_mut().zip(&columns) {
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for word in &words[sy * w + x0..sy * w + x1] {
                    sum[0] += (word >> 16) & 0xFF;
                    sum[1] += (word >> 8) & 0xFF;
                    sum[2] += word & 0xFF;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            *out = 0xFF00_0000 | ((sum[0] / n) << 16) | ((sum[1] / n) << 8) | (sum[2] / n);
        }
    }
    Face { pixels, size }
}

#[cfg(test)]
mod tests {
    use super::{ASPECT, BACKS, Faces, PICTURES, scale};
    use crate::cards::deck;
    use denise::Size;
    use std::rc::Rc;

    #[test]
    fn every_card_and_every_back_has_a_picture_of_the_size_said() {
        let full = Size::new(ASPECT.0 as u32, ASPECT.1 as u32);
        for (card, bytes) in deck().into_iter().zip(PICTURES) {
            let picture = denise_image::decode_png(bytes)
                .unwrap_or_else(|e| panic!("{card:?} does not decode: {e:?}"));
            assert_eq!(picture.size(), full, "{card:?} is another size");
            // Opaque, which is what lets `scale` average without unpremultiplying.
            assert!(picture.pixels().iter().all(|p| p >> 24 == 0xFF), "{card:?}");
        }
        let mut faces = Faces::default();
        for (which, (name, bytes)) in BACKS.iter().enumerate() {
            let back = denise_image::decode_png(bytes)
                .unwrap_or_else(|e| panic!("the back {name} does not decode: {e:?}"));
            assert_eq!(back.size(), full, "{name}");
            let small = faces.back(which, Size::new(27, 40)).expect(name);
            assert_eq!(small.pixels.len(), 27 * 40);
        }
        assert!(faces.back(BACKS.len(), Size::new(27, 40)).is_none());
    }

    /// A red card's corner is red and a black one's is not: the pictures are in
    /// the order the cards are, which no amount of looking at one file shows.
    #[test]
    fn the_pictures_are_in_the_cards_order() {
        for (card, bytes) in deck().into_iter().zip(PICTURES) {
            let picture = denise_image::decode_png(bytes).unwrap();
            let red = picture
                .pixels()
                .iter()
                .filter(|p| {
                    let (r, g, b) = ((*p >> 16) & 0xFF, (*p >> 8) & 0xFF, *p & 0xFF);
                    r > 180 && g < 80 && b < 80
                })
                .count();
            // A court card of a black suit wears red too, but less of it than
            // a red suit's pips and letters make.
            if card.rank <= 10 {
                assert_eq!(red > 500, card.suit.red(), "{card:?}: {red} red pixels");
            }
        }
    }

    #[test]
    fn scaling_averages_what_it_covers() {
        let (black, white) = (0xFF00_0000, 0xFFFF_FFFF);
        let two = [black, white, white, black];
        let from = Size::new(2, 2);
        assert_eq!(scale(&two, from, Size::new(1, 1)).pixels, [0xFF7F_7F7F]);
        assert_eq!(scale(&two, from, from).pixels, two);
        // A source that is not the size it claims is paper, not a panic.
        assert_eq!(scale(&two, Size::new(3, 3), from).pixels, [white; 4]);
    }

    #[test]
    fn a_picture_is_kept_until_the_table_changes_size() {
        let mut faces = Faces::default();
        let card = deck()[0];
        let first = faces.card(card, Size::new(30, 44)).unwrap();
        assert_eq!(first.size, Size::new(30, 44));
        assert_eq!(first.pixels.len(), 30 * 44);
        // The same allocation, not an equal one: nothing was scaled twice.
        let again = faces.card(card, Size::new(30, 44)).unwrap();
        assert!(Rc::ptr_eq(&first, &again));
        assert_eq!(faces.card(card, Size::new(60, 87)).unwrap().size.width, 60);
    }
}
