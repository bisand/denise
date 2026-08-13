//! The widgets that ship with Denise.
//!
//! Deliberately few. This is a toolkit for panels and kiosks, not a general
//! application framework, and the bar a widget has to clear is being something
//! several panels would otherwise each get subtly wrong — focus handling,
//! keyboard semantics, hit areas, disabled states — rather than saving a caller
//! three `fill_rect` calls. Anything more specific is better written against
//! [`Widget`](crate::Widget) in the application than guessed at here.
//!
//! | | |
//! |---|---|
//! | [`Panel`] | A surface with an optional border |
//! | [`Label`] | Static text, aligned in its box |
//! | [`Button`] | Emits a message of your type |
//! | [`TextInput`] | Editing, a caret, and the only widget that animates by default |
//! | [`Checkbox`] | A boolean. Space toggles, Enter does not, the label is part of the hit area |
//! | [`Toggle`] | The same boolean as a switch, with a sliding knob |
//! | [`RadioGroup`] | One choice from a few. **One node, so one tab stop** |
//! | [`Progress`] | Purely an output. Clamps a value nobody checked |
//! | [`Slider`] | A value in a range. Keeps the pointer after a drag leaves it |
//! | [`Divider`] | A rule, optionally with a label in it |
//! | [`Badge`] | A short string in a coloured pill |
//! | [`Alert`] | A coloured banner with a wrapped message |
//! | [`Tabs`] | A row of labels, one selected. **One node, so one tab stop** |
//! | [`List`] | Rows, at most one selected. Selecting and activating are separate |
//! | [`RadialProgress`] | The circular counterpart, with room for a number in the middle |
//! | [`Spinner`] | An arc that turns. **The one widget that can keep a device awake** |
//! | [`Select`] | The closed half of a dropdown. [`open_select`] is the open half |
//!
//! The first four are what CoreCanvas 0.4 shipped. The rest are being added one
//! at a time against <https://github.com/bisand/denise/issues/6>.
//!
//! Every one of them names theme *roles* rather than colours, so a panel built
//! from these survives a theme swap without a single widget knowing it happened.
//!
//! # Two rules they all share
//!
//! **A setter is silent.** `set_checked`, `set_value`, `set_selected` change the
//! widget and emit nothing. The message reports what a *person* did, and an
//! application that assigned and got its own message back would either loop or
//! have to guard against itself.
//!
//! **A message carries the new value**, as a `fn(T) -> M` rather than a fixed
//! message, so an application matches on what the widget became instead of
//! looking it up afterwards. An enum's tuple variant already is such a function:
//! `Checkbox::new("Mute", Message::Muted)`.
//!
mod alert;
mod badge;
mod button;
mod checkbox;
mod divider;
mod label;
mod list;
mod panel;
mod progress;
mod radial;
mod radio;
mod select;
mod slider;
mod spinner;
mod style;
mod tabs;
mod text_input;
mod toggle;

pub use alert::Alert;
pub use badge::Badge;
pub use button::Button;
pub use checkbox::Checkbox;
pub use divider::Divider;
pub use label::Label;
pub use list::{List, ListItem};
pub use panel::Panel;
pub use progress::Progress;
pub use radial::RadialProgress;
pub use radio::RadioGroup;
pub use select::{Select, open_select};
pub use slider::Slider;
pub use spinner::Spinner;
pub use style::{Align, Orientation};
pub use tabs::Tabs;
pub use text_input::TextInput;
pub use toggle::Toggle;
