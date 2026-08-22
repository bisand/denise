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
//! # Modifiers
//!
//! Shift cycles rather than latching on a double tap: off, once, locked. There
//! is no clock in the press path and threading a timestamp through every key to
//! serve one of them is a poor trade, so the key says which state it is in and
//! a tap moves to the next.
//!
//! `Locked` is Caps Lock and not a held Shift — it applies to letters and
//! leaves the digit row alone, which is the difference between a locked
//! keyboard typing `1` and typing `!`. The [`Composer`] models that already, so
//! it is latched with a `CapsLock` key rather than reimplemented here.
//!
//! The third level is the layout's own `AltGr` rather than a page of symbols
//! chosen here, because there is no such page to choose: `@` is `AltGr`+`2` on a
//! Norwegian keyboard and `Shift`+`2` on a US one, and a fixed grid would be
//! wrong on one of them. It latches, since a finger cannot hold one key and
//! press another.
//!
//! # What is not here yet
//!
//! Key repeat — holding Backspace to keep deleting — needs a press-and-hold
//! signal the toolkit does not have; `Button` emits on release. Layout
//! switching from a key on the keyboard is the next issue.
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
use denise_ui::widgets::{Button, TextInput};
use denise_ui::{NodeId, Side, Ui};

mod grid;

pub use grid::{Key, ROWS, Row};

/// What the third-level key says.
const LEVEL3_LEGEND: &str = "alt";

/// Height of one key, in logical pixels.
///
/// A finger, not a mouse: this is the smallest target that is comfortable on a
/// panel somebody is standing in front of.
pub const KEY_HEIGHT: i32 = 48;

/// Space between keys, and around the edge of the shelf.
pub const KEY_GAP: i32 = 6;

/// What the Shift key is currently doing.
///
/// Three states rather than a shift that latches on a double tap, and the
/// reason is that [`Keyboard::press`] is not given a clock. A double-tap window
/// needs one, and threading a timestamp through every key press to serve one key
/// is a poor trade against a cycle a user can see: the key says which state it
/// is in, and tapping it moves to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shift {
    /// Lower case, and the next character is not shifted.
    #[default]
    Off,
    /// The next character is shifted, and then this releases.
    Once,
    /// Every letter is shifted until this is turned off. Caps Lock, and like
    /// Caps Lock it leaves the digit row alone.
    Locked,
}

impl Shift {
    /// What the key says.
    #[inline]
    pub const fn legend(self) -> &'static str {
        match self {
            Shift::Off => "shift",
            Shift::Once => "SHIFT",
            Shift::Locked => "CAPS",
        }
    }

    /// The state after a tap.
    #[inline]
    const fn next(self) -> Self {
        match self {
            Shift::Off => Shift::Once,
            Shift::Once => Shift::Locked,
            Shift::Locked => Shift::Off,
        }
    }

    /// Whether Shift itself is held for the next press.
    ///
    /// `Locked` is **not** included: Caps Lock is not a held Shift, and treating
    /// it as one is the bug that makes a locked keyboard type `!` for `1`. The
    /// composer models it properly, latched by a `CapsLock` key, and applies it
    /// to letters only.
    #[inline]
    const fn holds_shift(self) -> bool {
        matches!(self, Shift::Once)
    }
}

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
    shift: Shift,
    level3: bool,
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
            shift: Shift::Off,
            level3: false,
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

    /// Opens or closes the keyboard to follow the focus, once a frame.
    ///
    /// The ordinary policy, and the one a panel wants: focus lands on a
    /// [`TextInput`] and the keyboard appears;
    /// focus goes anywhere else, or nowhere, and it leaves. Call it in the
    /// application's turn, beside [`Ui::drain_messages`](denise_ui::Ui::drain_messages).
    ///
    /// It answers *is this a text field* by asking the tree for the node as one,
    /// so there is no list of fields to keep in step with the tree.
    ///
    /// A press on a key moves no focus, so the keyboard does not close itself
    /// mid-word. Escape is the application's to bind: a shelf pushes no scene,
    /// so the tree does not claim the key and will not close the keyboard for
    /// you.
    ///
    /// An application wanting a different rule — a search box that already has a
    /// hardware keyboard, a field that should never summon one — ignores this and
    /// reads [`Ui::focus_changed`](denise_ui::Ui::focus_changed) itself. That is
    /// the whole of what this does.
    pub fn follow_focus<M: Clone + 'static>(&mut self, ui: &mut Ui<M>, on_key: fn(KeyCode) -> M) {
        let Some(focus) = ui.focus_changed() else {
            return;
        };
        let wants = focus.is_some_and(|id| ui.widget::<TextInput<M>>(id).is_some());
        if wants {
            self.open(ui, on_key);
        } else {
            self.close(ui);
        }
    }

    /// One key from the grid, tapped — modifier keys included.
    ///
    /// The call an application makes when a key's message arrives, and the one
    /// that does the right thing whichever key it was: Shift and the third-level
    /// key change state and relabel the keyboard, everything else types.
    ///
    /// Returns the events to hand to [`Ui::handle`](denise_ui::Ui::handle);
    /// empty for a modifier key, which changes what the *next* press means and
    /// sends nothing itself.
    pub fn press_key<M: Clone + 'static>(
        &mut self,
        ui: &mut Ui<M>,
        code: KeyCode,
    ) -> Vec<InputEvent> {
        match code {
            KeyCode::ShiftLeft => {
                self.tap_shift();
                self.relabel(ui);
                Vec::new()
            }
            KeyCode::AltRight => {
                self.tap_level3();
                self.relabel(ui);
                Vec::new()
            }
            _ => {
                let was_once = self.shift == Shift::Once;
                let events = self.press(code);
                // A one-shot shift has just been spent, so the keys have to stop
                // claiming they are still shifted.
                if was_once {
                    self.relabel(ui);
                }
                events
            }
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
        let modifiers = self.modifiers();
        for state in [ElementState::Down, ElementState::Up] {
            out.push(InputEvent::Key {
                code,
                state,
                repeat: false,
                modifiers,
            });
            let composed = self.composer.feed(code, state, modifiers);
            for &ch in composed.as_slice() {
                out.push(InputEvent::Text { ch });
            }
        }
        // A one-shot shift is spent on the character it shifted. Doing this
        // after the feed rather than before is what makes it apply to exactly
        // one key.
        if self.shift == Shift::Once {
            self.shift = Shift::Off;
        }
        out
    }

    /// Taps the Shift key: off, then once, then locked, then off again.
    ///
    /// Returns the state it moved to. The caller relabels with
    /// [`Keyboard::relabel`] — or lets [`Keyboard::press_key`] do both.
    pub fn tap_shift(&mut self) -> Shift {
        let was_locked = self.shift == Shift::Locked;
        self.shift = self.shift.next();
        let locked = self.shift == Shift::Locked;
        if locked != was_locked {
            // The composer latches Caps Lock from the key stream, exactly as it
            // does for a real one, so it is told the same way.
            self.composer
                .feed(KeyCode::CapsLock, ElementState::Down, self.modifiers());
            self.composer
                .feed(KeyCode::CapsLock, ElementState::Up, self.modifiers());
        }
        self.shift
    }

    /// Taps the third-level key, the one a physical keyboard spells `AltGr`.
    ///
    /// This is the symbol page, and it is the layout's own third level rather
    /// than a grid of symbols chosen here: `@` is `AltGr`+`2` on a Norwegian
    /// keyboard and `Shift`+`2` on a US one, and a fixed grid would be wrong on
    /// one of them. It latches rather than being held, because a finger cannot
    /// hold one key and press another.
    ///
    /// Returns whether the third level is now on.
    pub fn tap_level3(&mut self) -> bool {
        self.level3 = !self.level3;
        // The composer tracks this from the key stream exactly as it does for a
        // real AltGr, so it is told the same way.
        let state = if self.level3 {
            ElementState::Down
        } else {
            ElementState::Up
        };
        self.composer
            .feed(KeyCode::AltRight, state, self.modifiers());
        self.level3
    }

    /// The state of the Shift key.
    #[inline]
    pub const fn shift(&self) -> Shift {
        self.shift
    }

    /// Whether the third level — the layout's `AltGr` — is showing.
    #[inline]
    pub const fn level3(&self) -> bool {
        self.level3
    }

    /// The modifiers a key press reports right now.
    fn modifiers(&self) -> Modifiers {
        if self.shift.holds_shift() {
            self.modifiers | Modifiers::SHIFT
        } else {
            self.modifiers
        }
    }

    /// What to print on a key, at the level the keyboard is currently showing.
    ///
    /// A dead key shows its mark, which is what the user is about to be holding.
    /// What to print on a key: exactly what pressing it would produce.
    ///
    /// Asked of the composer rather than worked out here, so Caps Lock sparing
    /// the digit row and the third level are right by construction instead of by
    /// a rule repeated in two places.
    fn legend(&self, code: KeyCode) -> Option<char> {
        match self.composer.output_for(code, self.shift.holds_shift()) {
            Output::Char(ch) | Output::Dead(ch) => Some(ch),
            Output::None => None,
        }
    }

    /// Rewrites every key's legend for the current shift and level.
    ///
    /// A position does not move when a modifier changes — only what it types —
    /// so this replaces labels on the keys that are already there.
    pub fn relabel<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) {
        for &(code, node) in &self.keys {
            let label = match code {
                KeyCode::ShiftLeft => self.shift.legend().to_string(),
                KeyCode::AltRight => LEVEL3_LEGEND.to_string(),
                _ => grid::legend_of(code)
                    .map(str::to_string)
                    .or_else(|| self.legend(code).map(|ch| ch.to_string()))
                    .unwrap_or_default(),
            };
            if let Some(button) = ui.widget_mut::<Button<M>>(node) {
                button.set_label(label);
            }
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
