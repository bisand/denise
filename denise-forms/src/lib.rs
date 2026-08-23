#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod build;
mod error;
mod form;

pub use build::{Built, Handler, Picture, Wiring};
pub use error::{At, Error, Reason};
pub use form::{Form, FormKind, VERSION};

pub use denise_ui::widgets::Payload;
