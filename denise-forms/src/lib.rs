#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod build;
mod error;
mod form;

pub use build::{
    Built, Handler, NODE_PROPERTIES, Picture, Placed, Wiring, default_size, node_property,
    owns_children, seed,
};
pub use error::{At, Error, Reason};
pub use form::{Edit, Form, FormKind, Literal, MAX_DEPTH, MAX_SOURCE, VERSION, after_removing};

pub use denise_ui::widgets::Payload;
