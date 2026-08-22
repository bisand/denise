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
//! Shift is a one-shot: armed by a tap and spent by the next key. There is no
//! clock in the press path, so no double-tap window could latch it — and none
//! is wanted, because Caps Lock has a key of its own where Caps Lock goes.
//!
//! Caps is a latch and not a held Shift: it applies to letters and leaves the
//! digit row alone, which is the difference between a locked keyboard typing
//! `1` and typing `!`, and caps over shift gives lower case the way a hand
//! expects. The [`Composer`] models that already, so it is latched with a
//! `CapsLock` key rather than reimplemented here. Ctrl is a one-shot too, and
//! reaches the events it modifies.
//!
//! Every key that changes what the *next* press means says which state it is
//! in, and the number and punctuation keys carry what Shift would give in a
//! small second legend — the `!` over the `1`. Letters do not: a capital `Q`
//! over a `q` is not news.
//!
//! The keys whose meaning is a *name* are drawn rather than lettered — see
//! [`icons`] — which is why they look the same on a machine with no fonts
//! installed as on one with DejaVu.
//!
//! The third level is the layout's own `AltGr` rather than a page of symbols
//! chosen here, because there is no such page to choose: `@` is `AltGr`+`2` on a
//! Norwegian keyboard and `Shift`+`2` on a US one, and a fixed grid would be
//! wrong on one of them. It latches, since a finger cannot hold one key and
//! press another.
//!
//! # Layouts
//!
//! [`Keyboard::from_system`] starts from whatever the machine is configured
//! for, which is the answer the hardware path starts from too — so a panel with
//! a keyboard plugged into it and one without agree about what the `;` position
//! types. It hands back a [`LayoutSource`], and
//! [`LayoutSource::Unknown`] is the one
//! worth showing somebody: the system asked for a layout there is no table for
//! and got US.
//!
//! The layout key walks the built-ins. Switching **reletters the keys where
//! they stand** rather than rebuilding them, because a position does not move
//! when the layout changes — `KeyCode::Semicolon` is where `ø` lives on
//! Norwegian and `ö` on German, and it is the same key.
//!
//! Switching the keyboard does not switch a physical keyboard attached to the
//! same machine. An application that wants both in step calls
//! `InputBackend::set_layout` as well; the toolkit does not couple them,
//! because it does not know the two are meant to agree.
//!
//! # The shape of it
//!
//! A compact physical keyboard rather than a phone one: fourteen columns,
//! Backspace top right, Tab opening the second row, Enter closing the home row,
//! Shift at both ends of the bottom one. A panel is something somebody stands
//! in front of and types an address into, so the digits stay on screen instead
//! of going behind a `123` page, and Tab is how a form gets crossed.
//!
//! The width is what makes the layouts complete — see [`ROWS`] for which
//! positions carry what, and why a narrower grid could not type `å`.
//!
//! # Holding a key
//!
//! Backspace repeats while it is held and nothing else does, which is what a
//! phone does and what stops a slow finger typing `aaaaaa`. [`Keyboard::tick`]
//! collects what a held key has earned, once a frame; it costs nothing when
//! nobody is touching one, because a repeating key asks the tree to wake it
//! only between its press and its release.
//!
//! Holding a *letter* offers its alternates instead — `é è ê ë` over the `e`,
//! in a framed strip above the key, chosen by where the finger lifts. The
//! characters come from the layout, so they change when it does. That gesture
//! needs the application's help for one call: see [`Keyboard::handle`].
//!
//! A field focused under the keyboard is scrolled clear of it where it sits in
//! something that scrolls; where it does not, [`Keyboard::occluded`] says what
//! to move it clear of.
//!
//! [`InputEvent::Key`]: denise::InputEvent::Key
//! [`InputEvent::Text`]: denise::InputEvent::Text
//! [`Ui::handle`]: denise_ui::Ui::handle
//! [`Ui::push_shelf`]: denise_ui::Ui::push_shelf
//! [`Button`]: denise_ui::widgets::Button
//! [`Button::no_focus`]: denise_ui::widgets::Button::no_focus
//! [`TextInput`]: denise_ui::widgets::TextInput

use denise::{ElementState, InputEvent, KeyCode, Modifiers, Point, Rect, Role};
use denise_layout::{Composer, Layout, LayoutSource, Output};
use denise_text::TextStyle;
use denise_ui::widgets::{Button, Panel, TextInput};
use denise_ui::{NodeId, Side, Ui};

mod grid;
pub mod icons;

pub use grid::{Key, ROWS, Row};

/// What the third-level key says.
const LEVEL3_LEGEND: &str = "alt";

/// The position the layout key borrows.
///
/// A key in the grid has to *be* a position, and no real position means "change
/// layout". `Unidentified` is what the tree already uses for a key it cannot
/// name, and no layout table letters it — so nothing can be pressed by accident
/// and nothing else will ever claim it.
pub(crate) const LAYOUT_KEY: KeyCode = KeyCode::Unidentified(u32::MAX);

/// Height of one key, in logical pixels.
///
/// A finger, not a mouse: this is the smallest target that is comfortable on a
/// panel somebody is standing in front of.
pub const KEY_HEIGHT: i32 = 48;

/// Space between keys, and around the edge of the shelf.
pub const KEY_GAP: i32 = 6;

/// Legend size, in logical pixels, when the application names no style.
const KEY_TEXT: u16 = 16;

/// How long Backspace waits before it starts deleting on its own.
///
/// Long enough that no ordinary tap reaches it, short enough that somebody who
/// meant to hold does not first wonder whether it is broken.
pub const REPEAT_DELAY_MS: u64 = 450;

/// The margin of frame showing around a strip of alternates, in logical pixels.
///
/// Wide enough that the border reads as the edge of something floating rather
/// than as a seam between keys.
const STRIP_PAD: i32 = 8;

/// How thick that frame is drawn.
const STRIP_BORDER: i32 = 2;

/// How long a letter is held before it offers its alternates.
///
/// Longer than the pause before Backspace starts repeating, and deliberately:
/// a repeat that starts a shade early costs one character, and a strip of
/// letters that opens under a finger that meant to type `e` costs the gesture.
pub const HOLD_MS: u64 = 500;

/// How often it deletes after that.
///
/// Roughly fifteen a second: fast enough to clear a URL bar in a moment, slow
/// enough to stop where you meant to.
pub const REPEAT_INTERVAL_MS: u64 = 65;

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
}

impl Shift {
    /// What the key says.
    #[inline]
    pub const fn legend(self) -> &'static str {
        match self {
            Shift::Off => "shift",
            Shift::Once => "SHIFT",
        }
    }

    /// The state after a tap.
    #[inline]
    const fn next(self) -> Self {
        match self {
            Shift::Off => Shift::Once,
            Shift::Once => Shift::Off,
        }
    }

    /// Whether Shift itself is held for the next press.
    ///
    /// Caps Lock is deliberately not part of this. It is not a held Shift —
    /// treating it as one is the bug that makes a locked keyboard type `!` for
    /// `1` — and it now has a key of its own, latched in the composer, applying
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
    /// Ctrl armed for the next key. One-shot, like Shift.
    ctrl: bool,
    /// The alternates on screen, if a key is being held: the key they came
    /// from, and one node per character offered.
    offering: Option<Offering>,
    shelf: Option<NodeId>,
    keys: Vec<(KeyCode, NodeId)>,
    scale: f32,
    style: TextStyle,
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
            ctrl: false,
            offering: None,
            shelf: None,
            keys: Vec::new(),
            scale: 1.0,
            style: TextStyle::built_in(KEY_TEXT),
        }
    }

    /// The same keyboard at a display scale.
    ///
    /// The grid is written in logical pixels — [`KEY_HEIGHT`] is a fingertip,
    /// not a count of device pixels — and this is what turns them into the ones
    /// the surface has. The same `scale` the application scales its own layout
    /// by, and the same one [`Theme::scaled`](denise::Theme::scaled) takes.
    ///
    /// Set it before opening. A keyboard already on screen is not relaid out,
    /// because the surface it is sitting on has not changed size either.
    #[must_use]
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Changes the face the legends are drawn in, keys already up included.
    ///
    /// [`with_style`](Self::with_style) is the one to reach for; this is for the
    /// application that cannot yet know its own font — the table editor builds
    /// its tree with the built-in face so that it has *a* face whether or not a
    /// font file turned up, and restyles everything once one has.
    pub fn set_style<M: Clone + 'static>(&mut self, ui: &mut Ui<M>, style: TextStyle) {
        self.style = style;
        for &(_, node) in &self.keys {
            if let Some(button) = ui.widget_mut::<Button<M>>(node) {
                button.set_style(style);
            }
        }
    }

    /// The face and size the legends are drawn in.
    ///
    /// Worth setting, and the reason is what the default has to be: a widget
    /// cannot know which fonts the application loaded, so [`Button`] falls back
    /// to the built-in 8x8 bitmap face and so does this. On a panel that has a
    /// real font that fallback is visible — the one widget somebody is touching
    /// is the one drawn in a different typeface — and on a layout with `ß` or a
    /// composed `ü` on it, the built-in face has no such glyph to draw.
    ///
    /// Already scaled, like every other style an application builds: this does
    /// not multiply `size_px` by [`with_scale`](Self::with_scale).
    #[must_use]
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// A keyboard in whatever layout the machine is configured for.
    ///
    /// The same answer the hardware path starts from, so a panel with a
    /// keyboard plugged in and one without agree about what the `;` position
    /// types. Returns the [`LayoutSource`] alongside, which is worth showing
    /// somebody: [`LayoutSource::Unknown`] means the system asked for a layout
    /// there is no table for and got US, and a keyboard silently in the wrong
    /// language is a bad afternoon.
    pub fn from_system() -> (Self, LayoutSource) {
        let (layout, source) = denise_layout::from_system();
        (Self::new(layout), source)
    }

    /// Changes layout, relettering the keys where they stand.
    ///
    /// A position does not move when the layout changes — `KeyCode::Semicolon`
    /// is where `ø` lives on Norwegian and `ö` on German — so this replaces
    /// legends rather than rebuilding the grid.
    ///
    /// Any half-typed dead key is dropped: a mark waiting for a base character
    /// means nothing once the layout that was going to supply it has gone.
    /// Shift, Caps Lock and the third level survive, because they are facts
    /// about the keyboard rather than about the layout.
    pub fn set_layout<M: Clone + 'static>(&mut self, ui: &mut Ui<M>, layout: &'static Layout) {
        if core::ptr::eq(self.layout, layout) {
            return;
        }
        self.layout = layout;
        // `Composer::set_layout` drops the pending dead key and keeps Caps Lock
        // and the third level, which is exactly the split wanted: a half-typed
        // mark belonged to the old layout, and the user's hands have not moved.
        self.composer.set_layout(layout);
        self.relabel(ui);
    }

    /// Moves to the next built-in layout, wrapping.
    ///
    /// What the layout key does. Returns the layout it moved to, whose `name`
    /// is what the key then says.
    pub fn cycle_layout<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) -> &'static Layout {
        let next = denise_layout::BUILT_IN
            .iter()
            .position(|l| core::ptr::eq(*l, self.layout))
            .map_or(0, |i| (i + 1) % denise_layout::BUILT_IN.len());
        let layout = denise_layout::BUILT_IN[next];
        self.set_layout(ui, layout);
        layout
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
    ///
    /// [`Keyboard::height`] is this at the keyboard's scale, and is the one an
    /// application wants; this is the constant it is derived from.
    pub const LOGICAL_HEIGHT: i32 = ROWS.len() as i32 * (KEY_HEIGHT + KEY_GAP) + KEY_GAP;

    /// The height the whole keyboard occupies, in the surface's own pixels.
    ///
    /// [`Self::LOGICAL_HEIGHT`] through [`with_scale`](Self::with_scale). What
    /// the shelf is pushed at, and what an application subtracts when it wants
    /// to know how much screen it has left.
    #[inline]
    pub fn height(&self) -> i32 {
        self.scaled(Self::LOGICAL_HEIGHT)
    }

    /// One logical length in surface pixels.
    ///
    /// Through [`Rect::scaled`] rather than a multiplication written again here,
    /// so a key's edges and the shelf's height round the same way and the bottom
    /// row does not end a pixel short of the shelf it sits in.
    #[inline]
    fn scaled(&self, logical: i32) -> i32 {
        Rect::new(0, 0, 0, logical).scaled(self.scale).height
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
        let shelf = ui.push_shelf(Side::Below, self.height())?;
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
            KeyCode::CapsLock => {
                self.tap_caps();
                self.relabel(ui);
                Vec::new()
            }
            KeyCode::ControlLeft => {
                self.tap_ctrl();
                self.relabel(ui);
                Vec::new()
            }
            KeyCode::AltRight => {
                self.tap_level3();
                self.relabel(ui);
                Vec::new()
            }
            LAYOUT_KEY => {
                self.cycle_layout(ui);
                Vec::new()
            }
            _ => {
                let spent = self.shift == Shift::Once || self.ctrl;
                let events = self.press(code);
                // A one-shot modifier has just been spent, so the keys have to
                // stop claiming it is still armed.
                if spent {
                    self.relabel(ui);
                }
                events
            }
        }
    }

    /// Collects whatever a held key has earned, once a frame.
    ///
    /// Call it beside [`follow_focus`](Self::follow_focus), and hand the result
    /// to [`Ui::handle`](denise_ui::Ui::handle) the way a key press is handed
    /// over. Empty on nearly every frame: only Backspace repeats, and only while
    /// a finger is actually on it.
    ///
    /// The events are the ones a real keyboard sends for an auto-repeat —
    /// [`InputEvent::Key`] with `repeat: true`, and whatever that types — which
    /// is what lets a widget tell a repeat from a deliberate second press. A
    /// `TextInput` inserts both; something that must not act twice on one
    /// gesture can look.
    ///
    /// **Nothing is polled.** A repeating key asks the tree to wake it while it
    /// is held and stops asking the moment it is released, so a panel with
    /// nobody touching it schedules nothing — this call simply finds a tally of
    /// nought and returns.
    ///
    /// [`InputEvent::Key`]: denise::InputEvent::Key
    pub fn tick<M: Clone + 'static>(&mut self, ui: &mut Ui<M>, now_ms: u64) -> Vec<InputEvent> {
        let _ = now_ms;
        let mut out = Vec::new();
        // Collected by position rather than from one remembered key, because
        // "which key is held" is the tree's fact and not this crate's.
        //
        // Read first, take second, and the split is not tidiness: `widget_mut`
        // damages the node it hands out, because it cannot know whether the
        // caller changed anything. Asking sixty keys mutably once a frame
        // therefore repainted the whole keyboard on every frame anything else
        // woke the tree for — which on a panel is a keyboard that flickers.
        let owed: Vec<(KeyCode, NodeId)> = self
            .keys
            .iter()
            .copied()
            .filter(|&(_, node)| {
                ui.widget::<Button<M>>(node)
                    .is_some_and(|button| button.repeats_pending() > 0)
            })
            .collect();
        let held: Vec<(KeyCode, u32)> = owed
            .into_iter()
            .filter_map(|(code, node)| {
                let repeats = ui.widget_mut::<Button<M>>(node)?.take_repeats();
                (repeats > 0).then_some((code, repeats))
            })
            .collect();
        for (code, repeats) in held {
            for _ in 0..repeats {
                out.extend(self.press_repeat(code));
            }
        }
        self.offer_alternates(ui);
        out
    }

    /// Opens a held key's alternates, once it has been held long enough.
    ///
    /// A strip of characters above the key, drawn as ordinary nodes at the top
    /// of the shelf rather than as a popup. That is not a shortcut: a popup
    /// pushes a scene, a pushed scene cancels whatever press it covers, and the
    /// press it would cancel here is the one holding the key that opened it.
    fn offer_alternates<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) {
        if self.offering.is_some() || self.shelf.is_none() {
            return;
        }
        // The one key held past the threshold, read without writing.
        let ready = self.keys.iter().copied().find(|&(_, node)| {
            ui.widget::<Button<M>>(node)
                .and_then(Button::held_ms)
                .is_some_and(|held| held >= HOLD_MS)
        });
        let Some((code, key)) = ready else {
            return;
        };
        self.open_alternates(ui, code, key);
    }

    /// Builds the strip for a key, wherever the decision to came from.
    fn open_alternates<M: Clone + 'static>(&mut self, ui: &mut Ui<M>, code: KeyCode, key: NodeId) {
        if self.offering.is_some() {
            return;
        }
        let Some(shelf) = self.shelf else {
            return;
        };
        let Some(base) = self.legend(code) else {
            return;
        };
        let choices: Vec<char> = self.layout.alternates_for(base).collect();
        if choices.is_empty() {
            return;
        }
        let Some(bounds) = ui.layout(key) else {
            return;
        };

        // Centred over the key, and kept on screen: a strip that ran off the
        // edge would put half its choices where no finger can reach them.
        //
        // The frame is not decoration. Drawn flush and in the keys' own colours
        // this landed as *another row of the keyboard* — five accented letters
        // sitting on the digits, indistinguishable from them, so the one thing
        // the gesture has to say (these five, right now, and not the sixty
        // behind them) was the one thing it did not. So: a border in the accent
        // colour, a margin wide enough to read as an edge rather than a seam,
        // and a clear gap between the strip and the row it floats over.
        let cell = bounds.height;
        let pad = self.scaled(STRIP_PAD);
        let width = cell * choices.len() as i32 + pad * 2;
        let height = cell + pad * 2;
        let x = (bounds.x + bounds.width / 2 - width / 2)
            .max(0)
            .min((ui.size().width as i32 - width).max(0));
        let y = (bounds.y - height - self.scaled(KEY_GAP) * 2).max(0);

        let Some(strip) = ui.add(
            shelf,
            Panel::filled(Role::Base300)
                .backdrop()
                .with_radius(denise::theme::Radius::Box)
                .with_border(Role::Primary, self.scaled(STRIP_BORDER).max(1)),
            Rect::new(x, y, width, height),
        ) else {
            return;
        };
        let mut nodes = Vec::with_capacity(choices.len());
        for (i, ch) in choices.iter().enumerate() {
            // Under the strip rather than beside it, so that taking the strip
            // away takes the choices with it: `Ui::remove` drops a subtree, and
            // a choice parented to the shelf would outlive the gesture that
            // made it and sit there being pressable.
            let at = Rect::new(pad + cell * i as i32, pad, cell, cell);
            if let Some(node) = ui.add(
                strip,
                // Inert, and that is the design rather than an omission: the
                // press that opened this strip is still down on the key, so the
                // tree never presses one of these and they would never emit.
                // The choice is made by where the finger lifts, which
                // `Keyboard::handle` answers.
                Button::<M>::inert(ch.to_string())
                    .no_focus()
                    .with_role(Role::Base100)
                    .with_style(self.style),
                at,
            ) {
                nodes.push((*ch, node));
            }
        }
        self.offering = Some(Offering {
            strip,
            choices: nodes,
            over: None,
        });
    }

    /// Input the keyboard answers itself, before the tree sees it.
    ///
    /// Only the alternates gesture needs this, and it needs it because the
    /// gesture has no precedent in the tree: the press that opened the strip is
    /// still down on the *key*, so the tree quite correctly keeps sending
    /// everything there, and the choice is made by where the finger lifts
    /// instead. The keyboard therefore does its own hit test against the strip
    /// it drew.
    ///
    /// Call it beside [`Ui::handle`](denise_ui::Ui::handle), with the same
    /// events. Returns what to type, which is empty on nearly every call —
    /// nothing happens here unless a strip is open.
    ///
    /// A character chosen this way arrives as [`InputEvent::Text`] alone, with
    /// no [`InputEvent::Key`] around it. That is the honest shape: `é` is not at
    /// a position on this keyboard, nothing pressed a key to get it, and a
    /// binding watching for keys should not think one was pressed.
    ///
    /// Lifting anywhere else ends the gesture and types nothing — except on
    /// the key itself, which types what it always types. That is why the strip
    /// does not repeat the base character among its choices: the key is still
    /// there underneath it, still where the finger already is, so a hold opened
    /// by accident is undone by not moving.
    ///
    /// [`InputEvent::Text`]: denise::InputEvent::Text
    /// [`InputEvent::Key`]: denise::InputEvent::Key
    pub fn handle<M: Clone + 'static>(
        &mut self,
        ui: &mut Ui<M>,
        events: &[InputEvent],
    ) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for event in events {
            let Some(offering) = self.offering.as_ref() else {
                return out;
            };
            match event {
                InputEvent::PointerMoved { position } | InputEvent::TouchMoved { position, .. } => {
                    let at = self.choice_at(ui, *position);
                    if let Some(offering) = self.offering.as_mut()
                        && offering.over != at
                    {
                        offering.over = at;
                        self.highlight(ui);
                    }
                }
                InputEvent::PointerButton {
                    state: ElementState::Up,
                    position,
                    ..
                }
                | InputEvent::TouchUp {
                    position,
                    cancelled: false,
                    ..
                } => {
                    let chosen = self
                        .choice_at(ui, *position)
                        .and_then(|i| offering.choices.get(i))
                        .map(|(ch, _)| *ch);
                    self.close_offering(ui);
                    if let Some(ch) = chosen {
                        out.push(InputEvent::Text { ch });
                    }
                }
                // A sequence the system took away is not a choice, wherever the
                // last position happened to land. The strip goes, and nothing
                // is typed — the same answer as lifting off it, because that is
                // what a cancelled gesture is.
                InputEvent::TouchUp {
                    cancelled: true, ..
                } => self.close_offering(ui),
                _ => {}
            }
        }
        out
    }

    /// Which choice a point is over, if any.
    fn choice_at<M: Clone + 'static>(&self, ui: &Ui<M>, at: Point) -> Option<usize> {
        let offering = self.offering.as_ref()?;
        offering
            .choices
            .iter()
            .position(|&(_, node)| ui.bounds(node).is_some_and(|b| b.contains(at)))
    }

    /// Paints the choice under the finger differently from the rest.
    fn highlight<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) {
        let Some(offering) = self.offering.as_ref() else {
            return;
        };
        let over = offering.over;
        let nodes: Vec<(usize, NodeId)> = offering
            .choices
            .iter()
            .enumerate()
            .map(|(i, &(_, node))| (i, node))
            .collect();
        for (i, node) in nodes {
            let role = if Some(i) == over {
                Role::Primary
            } else {
                Role::Base100
            };
            // Read before writing: `widget_mut` repaints whatever it hands out,
            // and a strip that repainted every key on every pointer move is the
            // flicker bug in miniature.
            let same = ui
                .widget::<Button<M>>(node)
                .is_some_and(|button| button.role() == role);
            if !same && let Some(button) = ui.widget_mut::<Button<M>>(node) {
                button.set_role(role);
            }
        }
    }

    /// Takes the strip away, leaving the key it came from alone.
    fn close_offering<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) {
        if let Some(offering) = self.offering.take() {
            ui.remove(offering.strip);
        }
    }

    /// Opens a key's alternates without waiting for a finger.
    ///
    /// For a headless run — a snapshot, a test — which has no finger to hold
    /// anything with and no clock running while it does. The gesture itself
    /// goes through [`tick`](Self::tick) and a real press.
    pub fn offer_for_test<M: Clone + 'static>(&mut self, ui: &mut Ui<M>, code: KeyCode) {
        let Some(&(_, key)) = self.keys.iter().find(|&&(c, _)| c == code) else {
            return;
        };
        self.open_alternates(ui, code, key);
    }

    /// The characters currently on offer, and the nodes showing them.
    ///
    /// Empty unless a key is being held past [`HOLD_MS`]. Given out so a test
    /// can aim at one, and so an application that wants to drive the gesture
    /// some other way can.
    pub fn choices(&self) -> &[(char, NodeId)] {
        self.offering.as_ref().map_or(&[], |o| &o.choices)
    }

    /// Whether a held key is offering its alternates.
    #[inline]
    pub const fn offering(&self) -> bool {
        self.offering.is_some()
    }

    /// One auto-repeat of a key already down.
    ///
    /// [`press`](Self::press) with `repeat: true`, and without the one-shot
    /// Shift bookkeeping: a repeat is the *same* press arriving again, so it
    /// cannot spend a shift that the first press already spent.
    pub fn press_repeat(&mut self, code: KeyCode) -> Vec<InputEvent> {
        let mut out = Vec::with_capacity(3);
        let modifiers = self.modifiers();
        for state in [ElementState::Down, ElementState::Up] {
            out.push(InputEvent::Key {
                code,
                state,
                repeat: true,
                modifiers,
            });
            let composed = self.composer.feed(code, state, modifiers);
            for &ch in composed.as_slice() {
                out.push(InputEvent::Text { ch });
            }
        }
        out
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
        // A one-shot modifier is spent on the character it modified. Doing this
        // after the feed rather than before is what makes it apply to exactly
        // one key.
        if self.shift == Shift::Once {
            self.shift = Shift::Off;
        }
        self.ctrl = false;
        out
    }

    /// Taps the Shift key: off, then once, then locked, then off again.
    ///
    /// Returns the state it moved to. The caller relabels with
    /// [`Keyboard::relabel`] — or lets [`Keyboard::press_key`] do both.
    pub fn tap_shift(&mut self) -> Shift {
        self.shift = self.shift.next();
        self.shift
    }

    /// Caps Lock on or off. Returns the state it moved to.
    ///
    /// A latch of its own rather than a third state of Shift, which is what a
    /// real keyboard does and what the composer already modelled: it applies to
    /// letters and spares the digit row, so a locked keyboard types `1` and not
    /// `!`. Told through the key stream, exactly as a real Caps Lock reaches it.
    pub fn tap_caps(&mut self) -> bool {
        let modifiers = self.modifiers();
        self.composer
            .feed(KeyCode::CapsLock, ElementState::Down, modifiers);
        self.composer
            .feed(KeyCode::CapsLock, ElementState::Up, modifiers);
        self.composer.caps_lock()
    }

    /// Whether Caps Lock is on.
    #[inline]
    pub fn caps(&self) -> bool {
        self.composer.caps_lock()
    }

    /// Ctrl for the next key, on or off. Returns the state it moved to.
    ///
    /// One-shot like Shift, and spent by the next key that types: a modifier
    /// that stayed on would be a keyboard that could not type a plain letter
    /// again without somebody noticing why.
    pub fn tap_ctrl(&mut self) -> bool {
        self.ctrl = !self.ctrl;
        self.ctrl
    }

    /// Whether Ctrl is armed for the next key.
    #[inline]
    pub const fn ctrl(&self) -> bool {
        self.ctrl
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

    /// The screen rectangle the keyboard is covering, or `None` when it is not
    /// up.
    ///
    /// Focusing a field already scrolls it clear of this, where the field is in
    /// something that scrolls and has somewhere to scroll to. This is for when
    /// it is not: a form at fixed rectangles has no scroll to give, and getting
    /// a field out from under the keyboard means the application moving
    /// something — shrinking a viewport by this height, or sliding a panel up.
    ///
    /// The keyboard's resting place from the moment it is opened, so an
    /// application acting on it during the slide aims where the keyboard is
    /// going.
    #[inline]
    pub fn occluded<M: Clone + 'static>(&self, ui: &Ui<M>) -> Option<Rect> {
        self.shelf.and(ui.occluded())
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
        let mut modifiers = self.modifiers;
        if self.shift.holds_shift() {
            modifiers |= Modifiers::SHIFT;
        }
        if self.ctrl {
            modifiers |= Modifiers::CTRL;
        }
        modifiers
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

    /// What a key says right now.
    ///
    /// The three keys whose legends come from the keyboard's own state rather
    /// than from the layout, then the layout's answer. One function because
    /// [`build`](Self::build) and [`relabel`](Self::relabel) must agree: when
    /// they did not, the layout key came up blank and only found its name after
    /// something else had caused a relabel.
    fn label_for(&self, code: KeyCode) -> String {
        match code {
            KeyCode::ShiftLeft => self.shift.legend().to_string(),
            KeyCode::CapsLock => if self.caps() { "CAPS" } else { "caps" }.to_string(),
            KeyCode::ControlLeft => if self.ctrl { "CTRL" } else { "ctrl" }.to_string(),
            KeyCode::AltRight => LEVEL3_LEGEND.to_string(),
            LAYOUT_KEY => self.layout.name.to_string(),
            _ => grid::legend_of(code)
                .map(str::to_string)
                .or_else(|| self.legend(code).map(|ch| ch.to_string()))
                .unwrap_or_default(),
        }
    }

    /// What is printed small in a key's top-right corner, if anything.
    ///
    /// What the key would type with Shift held — the `!` over the `1`, the `?`
    /// over the `+` — which is the whole reason a real keyboard prints it: you
    /// cannot discover Shift by pressing Shift, because pressing it is what
    /// changes the legend.
    ///
    /// **Numbers and punctuation only.** A letter's shifted form is its own
    /// capital and tells nobody anything, and forty keys each carrying a second
    /// glyph is a keyboard that reads as noise. This is the rule the keyboards
    /// it is shaped after use, and the reason they use it.
    ///
    /// Empty while Shift is held, because the main legend has already become
    /// the shifted character and printing it twice on one key says nothing.
    fn corner_for(&self, code: KeyCode) -> String {
        if code == LAYOUT_KEY {
            // The one corner that is not a shifted character. The key wears a
            // globe, which says what it is for and cannot say which layout is
            // live — so the name it used to carry as its legend moves here
            // rather than being dropped. On a panel with no other keyboard,
            // "am I typing Norwegian?" is a question the keyboard itself has to
            // answer.
            return self.layout.name.to_string();
        }
        if grid::legend_of(code).is_some() {
            // A key with a fixed word on it — back, enter, tab — types nothing
            // and has no other state to advertise.
            return String::new();
        }
        let Some(base) = self.legend(code) else {
            return String::new();
        };
        if base.is_alphabetic() {
            return String::new();
        }
        let shifted = match self.composer.output_for(code, true) {
            Output::Char(ch) | Output::Dead(ch) => ch,
            Output::None => return String::new(),
        };
        if shifted == base {
            return String::new();
        }
        shifted.to_string()
    }

    /// Rewrites every key's legend for the current shift and level.
    ///
    /// A position does not move when a modifier changes — only what it types —
    /// so this replaces labels on the keys that are already there.
    pub fn relabel<M: Clone + 'static>(&mut self, ui: &mut Ui<M>) {
        for &(code, node) in &self.keys {
            let label = self.label_for(code);
            let corner = self.corner_for(code);
            if let Some(button) = ui.widget_mut::<Button<M>>(node) {
                button.set_label(label);
                button.set_corner(corner);
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
        // A backdrop first, and it is not decoration: a shelf is a bare
        // container, so without one the page underneath shows through the gaps
        // between the keys — which on a browser is a paragraph of text running
        // between the rows. Added before the keys so it paints behind them.
        //
        // A *backdrop* and not an ordinary panel, which is what stops a finger
        // landing in the gap between two keys from falling through to the page
        // and taking the focus with it — a near-miss used to dismiss the whole
        // keyboard.
        //
        // `Base300` rather than `Base200`, which is two steps from the keys
        // rather than one: at one step a light theme's keys barely lift off the
        // deck and the whole thing reads as a flat grey slab. The dark themes
        // were always fine; this is the light one catching up.
        ui.add(
            shelf,
            Panel::filled(Role::Base300).backdrop(),
            Rect::new(0, 0, width, self.height()),
        );

        // The surface's width, back in the units the grid below is written in.
        // Laying out logically and scaling each rectangle at the end is what
        // keeps a row reaching both edges at 1.5x: `Rect::scaled` scales
        // *edges*, so keys that were a gap apart still are.
        let width = if self.scale > 0.0 {
            Rect::new(0, 0, width, 0).scaled(1.0 / self.scale).width
        } else {
            width
        };
        for (r, row) in ROWS.iter().enumerate() {
            let y = KEY_GAP + r as i32 * (KEY_HEIGHT + KEY_GAP);
            // Every key in a row shares the leftover width, so a row of ten and
            // a row of three both reach both edges.
            //
            // Each key's edges are placed from its running share of the row
            // rather than from a rounded width per unit, and the difference is
            // the whole point: a width of `leftover / units` throws away the
            // remainder once per key, and eleven keys of it leaves the row
            // visibly short of the edge it was supposed to reach. Placing edges
            // spends the remainder across the row instead, a pixel at a time.
            let count = row.keys.len() as i32;
            let units: i32 = row.keys.iter().map(|k| k.units).sum::<i32>().max(1);
            let leftover = (width - KEY_GAP * (count + 1)).max(count);
            let mut done = 0;
            for (i, key) in row.keys.iter().enumerate() {
                let gaps = KEY_GAP * (i as i32 + 1);
                let x = gaps + leftover * done / units;
                done += key.units;
                let w = gaps + leftover * done / units - x;
                let mut button = Button::new(self.label_for(key.code), on_key(key.code))
                    .no_focus()
                    .with_role(role_of(key.code))
                    .with_style(self.style)
                    .with_corner(self.corner_for(key.code));
                // The word stays the button's label even where a picture is
                // drawn over it: that is what the key still reports, and what a
                // test reads.
                if let Some(icon) = grid::icon_of(key.code) {
                    button = button.with_icon(icon);
                }
                if key.repeats {
                    button = button.with_repeat(REPEAT_DELAY_MS, REPEAT_INTERVAL_MS);
                } else if grid::legend_of(key.code).is_none() {
                    // Only the keys that type a character can offer alternates,
                    // so only they need to be asked how long they have been
                    // held. A key with a word on it has nothing to offer and
                    // stays free.
                    button = button.watching_hold();
                }
                if let Some(node) = ui.add(
                    shelf,
                    button,
                    Rect::new(x, y, w, KEY_HEIGHT).scaled(self.scale),
                ) {
                    self.keys.push((key.code, node));
                }
            }
        }
    }
}

/// What colour a key wears.
///
/// Two kinds, and the split is what the key does rather than what it looks
/// like: keys that type a character are the neutral field the eye skims over,
/// and keys that change what the *next* press means stand out from them,
/// because those are the ones a user has to find deliberately. Enter is with
/// the second group for the same reason — it is the key with a consequence.
///
/// [`Role::Primary`] is the toolkit's default for a button and is wrong for
/// every key here: forty of them shouting at once is not emphasis.
fn role_of(code: KeyCode) -> Role {
    match code {
        KeyCode::ShiftLeft
        | KeyCode::CapsLock
        | KeyCode::ControlLeft
        | KeyCode::AltRight
        | KeyCode::Backspace
        | KeyCode::Tab
        | KeyCode::Escape
        | KeyCode::ArrowLeft
        | KeyCode::ArrowRight
        | LAYOUT_KEY => Role::Neutral,
        KeyCode::Enter => Role::Primary,
        _ => Role::Base100,
    }
}

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;

/// The alternates a held key is offering, while it offers them.
struct Offering {
    /// The strip's backdrop, removed with the rest.
    strip: NodeId,
    /// One node per character, in the order they are shown.
    choices: Vec<(char, NodeId)>,
    /// Which one the finger is over, if any.
    over: Option<usize>,
}
