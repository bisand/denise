#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod build;
mod error;
mod form;

pub use build::{Built, Handler, Picture, Placed, Wiring};
pub use error::{At, Error, Reason};
pub use form::{Edit, Form, FormKind, MAX_DEPTH, MAX_SOURCE, VERSION};

pub use denise_ui::widgets::Payload;
