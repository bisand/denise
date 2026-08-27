//! How big the designer's own text is, and why those sizes and not others.
//!
//! # The distinctions used to be free
//!
//! The built-in face is a five-by-seven glyph in an eight-pixel cell, drawn at
//! whole-number multiples of it, so `size_px / 8` decides everything and **every
//! size from 8 to 15 rendered identically**. This crate named 11, 12, 13, 14, 15
//! and 17 — five of which were one size on screen. Writing a different number
//! cost nothing and did nothing, so nobody ever had to mean one.
//!
//! [#130](https://github.com/bisand/denise/issues/130) gave the designer a real
//! face and the distinctions became real overnight. The window gained a size
//! hierarchy nobody had chosen, and the pane with the most text in it — the
//! inspector, at 11 — landed at the bottom of it. That is what
//! [#155](https://github.com/bisand/denise/issues/155) was reported as: *the
//! properties box looks a bit smaller than the rest.* It was.
//!
//! # Four steps, and the gaps are visible ones
//!
//! Measured with Arial, which is what a Mac supplies:
//!
//! | px | ascent | `"widget"` |
//! |----|--------|------------|
//! | 10 | 9      | 30         |
//! | 11 | 10     | 31         |
//! | 12 | 11     | 36         |
//! | 13 | 12     | 37         |
//! | 14 | 13     | 41         |
//! | 15 | 14     | 42         |
//! | 16 | 14     | 47         |
//! | 17 | 15     | 48         |
//!
//! The sizes **pair up**: 10 and 11 differ by one pixel across six characters,
//! and so do 12 and 13, 14 and 15, 16 and 17. Naming two of a pair in one window
//! is a distinction that cannot be seen, which is how a scale ends up with seven
//! steps and three appearances. So this takes every *other* size, and there are
//! four of them.
//!
//! # Why an enum
//!
//! The same reason the widgets publish `Property` descriptors rather than this
//! crate keeping a table: a number that cannot be written cannot drift. There is
//! no `with_size(12)` to reach for, so a twenty-eighth label is one of these four
//! or it does not compile.

/// A step on the designer's type scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// Text that is *about* the interface rather than in it: the status line,
    /// the strip captions, the message log, a sheet's explanatory second line.
    ///
    /// The only step allowed to be small, because it is the only one nobody has
    /// to read to get their work done.
    Caption,
    /// Everything a person reads and edits.
    ///
    /// Property names and their editors, outline rows, palette rows, toolbar
    /// buttons, the filter. Most of the window is this, and the whole of #155 is
    /// that several panes were below it for no reason.
    Body,
    /// A pane's name, and the line that says what is selected.
    Heading,
    /// A sheet's title. Two in the crate, and they are the only text that is
    /// meant to be seen before it is read.
    Title,
}

impl Text {
    /// Every step, for the tests that check they stay distinguishable and that
    /// each still fits the box it is given.
    #[cfg(test)]
    pub const ALL: [Self; 4] = [Self::Caption, Self::Body, Self::Heading, Self::Title];

    /// The size in logical pixels.
    ///
    /// Logical, like every constant in this crate:
    /// [`Scale`](crate::scale::Scale) multiplies it on the way into the tree.
    pub const fn px(self) -> u16 {
        match self {
            Self::Caption => 11,
            Self::Body => 13,
            Self::Heading => 15,
            Self::Title => 17,
        }
    }

    /// A box tall enough to hold one line of it, in logical pixels.
    ///
    /// Face-independent and deliberately a little generous: the real line height
    /// belongs to whatever face the machine supplied — Arial wants 15 at
    /// `Body` and the built-in bitmap wants 10 — and a label box is centred, so
    /// spare room costs nothing while a pixel too few clips the descenders.
    pub const fn line(self) -> i32 {
        self.px() as i32 * 4 / 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step is far enough from the next to be seen.
    ///
    /// The whole of #155: with a real face, 10 and 11 differ by one pixel across
    /// six characters, and so do 12 and 13. A scale with two of a pair on it is
    /// a scale claiming a difference nobody can point at — which is how this
    /// crate came to name seven sizes and show three.
    #[test]
    fn no_two_steps_are_a_size_apart() {
        let mut sizes: Vec<u16> = Text::ALL.iter().map(|step| step.px()).collect();
        sizes.sort_unstable();
        sizes.dedup();
        assert_eq!(sizes.len(), Text::ALL.len(), "two steps are the same size");
        for pair in sizes.windows(2) {
            assert!(
                pair[1] - pair[0] >= 2,
                "{} and {} are a single pixel apart, which is not a step",
                pair[0],
                pair[1]
            );
        }
    }

    /// A box is tall enough for the line it holds, at every step.
    ///
    /// Checked against the two faces that actually turn up: the built-in bitmap,
    /// whose line height is well under the size, and a real one, which wants
    /// about `size * 8 / 7` — Arial measures 12, 15, 17 and 20 for the four
    /// steps.
    #[test]
    fn a_line_of_each_step_fits_the_box_it_is_given() {
        for step in Text::ALL {
            let real = i32::from(step.px()) * 8 / 7;
            assert!(
                step.line() >= real,
                "{step:?} is {} px and its box is {}, which clips a real face at {real}",
                step.px(),
                step.line()
            );
            // And not so generous that a row of them stops fitting its pane.
            assert!(step.line() <= i32::from(step.px()) * 3 / 2);
        }
    }

    /// The steps go up in the order they are written.
    #[test]
    fn the_scale_is_in_order() {
        for pair in Text::ALL.windows(2) {
            assert!(
                pair[0].px() < pair[1].px(),
                "{:?} is not smaller than {:?}",
                pair[0],
                pair[1]
            );
        }
    }
}
