//! The widgets that ship with Denise.
//!
//! Deliberately few. This is a toolkit for panels and kiosks, not a general
//! application framework: a [`Panel`], a [`Label`], a [`Button`] and a
//! [`TextInput`] are what CoreCanvas 0.4 shipped, and anything more specific is
//! better written against [`Widget`](crate::Widget) in the application than
//! guessed at here.
//!
//! The bar a new one has to clear is being something several panels would
//! otherwise each get subtly wrong — focus handling, keyboard semantics, hit
//! areas, disabled states — rather than saving a caller three `fill_rect` calls.
//! [`Checkbox`] and [`Toggle`] are the first additions beyond that original
//! four, and both are here for the keyboard rules as much as for the drawing:
//! Space toggles, Enter does not, autorepeat does not, and the label is part of
//! the hit area. [`Toggle`] adds the only animation in the set, and is written
//! so that losing it costs nothing — see its own documentation for why that
//! matters.
//!
//! Every one of them names theme *roles* rather than colours, so a panel built
//! from these survives a theme swap without a single widget knowing it happened.

mod button;
mod checkbox;
mod label;
mod panel;
mod style;
mod text_input;
mod toggle;

pub use button::Button;
pub use checkbox::Checkbox;
pub use label::Label;
pub use panel::Panel;
pub use style::Align;
pub use text_input::TextInput;
pub use toggle::Toggle;
