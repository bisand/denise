//! The widgets that ship with Denise.
//!
//! Deliberately few. This is a toolkit for panels and kiosks, not a general
//! application framework: a Label, a Button, a TextInput and a Panel cover what
//! CoreCanvas 0.4 shipped, and anything more specific is better written against
//! [`Widget`](crate::Widget) in the application than guessed at here.

mod panel;

pub use panel::Panel;
