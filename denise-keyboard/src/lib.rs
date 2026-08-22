//! An on-screen keyboard for panels that have no other one.
//!
//! # It is not a special case
//!
//! The point of this crate is that nothing downstream of it can tell the
//! difference between a key tapped here and a key pressed on a keyboard plugged
//! into the machine. [`denise`] splits keyboard input into two events —
//! [`InputEvent::Key`], a physical position, and [`InputEvent::Text`], a
//! character somebody meant to insert — and the hardware path in `denise-evdev`
//! emits the first followed by whatever the second turns out to be.
//!
//! [`Keyboard::press`] emits the same two events, in the same order, produced by
//! the same [`Composer`] from the same [`Layout`] tables. The application hands
//! them to [`Ui::handle`], which is the call the hardware path's events arrive
//! through as well. So a [`TextInput`] inserts them without knowing, a key
//! binding on Enter fires exactly as it would, and every widget that already
//! handles keys handles these.
//!
//! That is worth stating because the alternative is so tempting: a method on the
//! text field that inserts a character directly. It would work, and then Enter
//! would do nothing, Escape would do nothing, and every widget that is not a
//! text field would be deaf to the keyboard.
//!
//! # It is built from widgets
//!
//! Every key is a [`Button`] — [`Button::no_focus`], so pressing one does not
//! move the caret out of the field being typed into. They sit on a
//! [`Ui::push_shelf`], which slides up from the bottom without pushing a scene,
//! so the field keeps focus while the keyboard is up. Neither of those is a
//! keyboard feature; they are toolkit features this is the first user of.
//!
//! # What is not here yet
//!
//! Shift, caps lock, a symbol page and key repeat are the next issue; layout
//! switching the one after. This crate types letters, digits, space, backspace
//! and enter, in whatever layout it was given.
//!
//! [`InputEvent::Key`]: denise::InputEvent::Key
//! [`InputEvent::Text`]: denise::InputEvent::Text
//! [`Ui::handle`]: denise_ui::Ui::handle
//! [`Ui::push_shelf`]: denise_ui::Ui::push_shelf
//! [`Button`]: denise_ui::widgets::Button
//! [`Button::no_focus`]: denise_ui::widgets::Button::no_focus
//! [`TextInput`]: denise_ui::widgets::TextInput

use denise::{ElementState, InputEvent, KeyCode, Modifiers, Rect};
use denise_layout::{Composer, Layout, Output};
use denise_ui::widgets::Button;
use denise_ui::{NodeId, Side, Ui};

mod grid;

pub use grid::{ROWS, Row};

/// Height of one key, in logical pixels.
///
/// A finger, not a mouse: this is the smallest target that is comfortable on a
/// panel somebody is standing in front of.
pub const KEY_HEIGHT: i32 = 48;

/// Space between keys, and around the edge of the shelf.
pub const KEY_GAP: i32 = 6;

/// An on-screen keyboard, and the composition state that goes with it.
///
/// Holds no widgets of its own: [`Keyboard::open`] builds them into a shelf and
/// [`Keyboard::close`] takes them away with it. What it keeps between those is
/// the part that has to survive a key press — the layout and the composer's
/// half-finished dead keys.
pub struct Keyboard {
    layout: &'static Layout,
    composer: Composer,
    modifiers: Modifiers,
    shelf: Option<NodeId>,
    keys: Vec<(KeyCode, NodeId)>,
}

impl Keyboard {
    /// A keyboard in `layout`.
    ///
    /// `denise_layout::from_system()` is the argument that makes it agree with
    /// whatever the machine is configured for.
    pub fn new(layout: &'static Layout) -> Self {
        Self {
            layout,
            composer: Composer::new(layout),
            modifiers: Modifiers::NONE,
            shelf: None,
            keys: Vec::new(),
        }
    }

    /// The layout its keys are lettered from.
    #[inline]
    pub const fn layout(&self) -> &'static Layout {
        self.layout
    }

    /// The node each key was added as, in grid order.
    ///
    /// Kept because a layout switch relabels these rather than rebuilding them:
    /// a position does not move when the layout changes, only its legend does.
    #[inline]
    pub fn keys(&self) -> &[(KeyCode, NodeId)] {
        &self.keys
    }

    /// Whether the keyboard is on screen.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.shelf.is_some()
    }

    /// The height a shelf needs for the whole keyboard, in logical pixels.
    #[inline]
    pub const fn height() -> i32 {
        ROWS.len() as i32 * (KEY_HEIGHT + KEY_GAP) + KEY_GAP
    }

    /// Slides the keyboard up and letters its keys from the current layout.
    ///
    /// `on_key` is how a key press reaches the application, the same shape every
    /// other widget uses to carry a value into a message. The application answers
    /// by calling [`Keyboard::press`] and handing the result to
    /// [`Ui::handle`](denise_ui::Ui::handle).
    ///
    /// Returns the shelf, or `None` when one is already up — the tree allows one
    /// at a time.
    pub fn open<M: Clone + 'static>(
        &mut self,
        ui: &mut Ui<M>,
        on_key: fn(KeyCode) -> M,
    ) -> Option<NodeId> {
        if self.shelf.is_some() {
            return None;
        }
        let shelf = ui.push_shelf(Side::Below, Self::height())?;
        let width = ui.size().width as i32;
        self.build(ui, shelf, width, on_key);
        self.shelf = Some(shelf);
        Some(shelf)
    }

    /// Slides the keyboard out. The keys go with it.
    ///
    /// Any half-typed dead key goes too: a mark waiting for a base character
    /// means nothing once the keyboard that was going to supply it is gone.
    pub fn close<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) {
        if self.shelf.take().is_some() {
            self.keys.clear();
            self.composer.set_layout(self.layout);
            ui.close_shelf();
        }
    }

    /// One key, tapped: the events a real keyboard would have sent.
    ///
    /// [`InputEvent::Key`] down, then whatever that typed as
    /// [`InputEvent::Text`], then [`InputEvent::Key`] up — the order the
    /// hardware path uses, so that a binding on the key runs before any text
    /// arrives and a field can insert every character it sees without filtering.
    ///
    /// A dead key types nothing and returns just the two `Key` events; the mark
    /// arrives folded into the next character, or beside it when the two cannot
    /// combine.
    ///
    /// [`InputEvent::Key`]: denise::InputEvent::Key
    /// [`InputEvent::Text`]: denise::InputEvent::Text
    pub fn press(&mut self, code: KeyCode) -> Vec<InputEvent> {
        let mut out = Vec::with_capacity(3);
        for state in [ElementState::Down, ElementState::Up] {
            out.push(InputEvent::Key {
                code,
                state,
                repeat: false,
                modifiers: self.modifiers,
            });
            let composed = self.composer.feed(code, state, self.modifiers);
            for &ch in composed.as_slice() {
                out.push(InputEvent::Text { ch });
            }
        }
        out
    }

    /// What to print on a key, at the level the keyboard is currently showing.
    ///
    /// A dead key shows its mark, which is what the user is about to be holding.
    fn legend(&self, code: KeyCode) -> Option<char> {
        let entry = self.layout.entry(code)?;
        match entry.base {
            Output::Char(ch) | Output::Dead(ch) => Some(ch),
            Output::None => None,
        }
    }

    /// Lays the rows out and adds a button per key.
    ///
    /// Explicit rectangles, because the toolkit has no layout engine and a
    /// keyboard is the case that wants none: a fixed grid, measured once.
    fn build<M: Clone + 'static>(
        &mut self,
        ui: &mut Ui<M>,
        shelf: NodeId,
        width: i32,
        on_key: fn(KeyCode) -> M,
    ) {
        for (r, row) in ROWS.iter().enumerate() {
            let y = KEY_GAP + r as i32 * (KEY_HEIGHT + KEY_GAP);
            // Every key in a row shares the leftover width, so a row of ten and
            // a row of three both reach both edges.
            let units: i32 = row.keys.iter().map(|k| k.units).sum();
            let gaps = KEY_GAP * (row.keys.len() as i32 + 1);
            let unit = (width - gaps).max(row.keys.len() as i32) / units.max(1);
            let mut x = KEY_GAP;
            for key in row.keys {
                let w = unit * key.units;
                let label = key
                    .legend
                    .map(str::to_string)
                    .or_else(|| self.legend(key.code).map(String::from))
                    .unwrap_or_default();
                if let Some(node) = ui.add(
                    shelf,
                    Button::new(label, on_key(key.code)).no_focus(),
                    Rect::new(x, y, w, KEY_HEIGHT),
                ) {
                    self.keys.push((key.code, node));
                }
                x += w + KEY_GAP;
            }
        }
    }
}

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
