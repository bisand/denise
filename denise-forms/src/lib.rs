#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[cfg(feature = "codegen")]
pub mod codegen;

mod build;
mod deadline;
mod error;
mod form;

pub use build::{
    Built, DESIGN, FORM_PROPERTIES, Handler, NODE_PROPERTIES, Page, Picture, Placed, Wiring,
    default_size, form_property, kind_properties, node_property, owns_children, seed, seed_form,
};
pub use deadline::{MAX_ABANDONED, PATIENCE};
pub use error::{At, Error, Reason};
pub use form::{
    Edit, Form, FormKind, Literal, MAX_COMMENTED_DEPTH, MAX_DEPTH, MAX_SOURCE, Placement, Scaling,
    THEMES, VERSION, Written, after_removing, fragment, tidy,
};

pub use denise_ui::widgets::Payload;
