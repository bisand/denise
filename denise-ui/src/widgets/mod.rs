//! The widgets that ship with Denise.
//!
//! Deliberately few. This is a toolkit for panels and kiosks, not a general
//! application framework: a [`Panel`], a [`Label`], a [`Button`] and a
//! [`TextInput`] are what CoreCanvas 0.4 shipped, and anything more specific is
//! better written against [`Widget`](crate::Widget) in the application than
//! guessed at here.
//!
//! Every one of them names theme *roles* rather than colours, so a panel built
//! from these survives a theme swap without a single widget knowing it happened.

mod button;
mod label;
mod panel;
mod style;
mod text_input;

pub use button::Button;
pub use label::Label;
pub use panel::Panel;
pub use style::Align;
pub use text_input::TextInput;
