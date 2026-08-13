//! Denise's scene graph, widgets and compositor.
//!
//! A retained tree of widgets in a generational arena, stacked into scenes, drawn
//! through [`denise_render`] into a [`denise::Surface`]. This is the layer that
//! turns "a rasteriser and a display" into "a user interface".
//!
//! ```no_run
//! # use denise::{Rect, Size, theme};
//! # use denise_ui::{Ui, widgets::Panel};
//! # fn demo(surface: &mut impl denise::Surface) -> Result<(), denise::SurfaceError> {
//! #[derive(Clone, Debug)]
//! enum Msg { Ok }
//!
//! let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
//! let root = ui.root();
//! ui.add(root, Panel::default(), Rect::new(40, 40, 400, 240));
//!
//! loop {
//!     // ui.handle(&events);
//!     ui.render(surface)?;   // draws nothing at all when nothing changed
//!     for message in ui.drain_messages() {
//!         match message { Msg::Ok => {} }
//!     }
//! #   break;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Why a separate crate
//!
//! [`denise`] is the platform-agnostic contract — geometry, colour, the pixel
//! buffer, input, damage, theming — and [`denise_render`] is the rasteriser that
//! depends on it. Widgets need both, so they cannot live in either without a
//! dependency cycle. Keeping them here also means a signage application that draws
//! its own scene links no arena, no tree and no widget code at all.
//!
//! # What is not here
//!
//! No layout engine. Nodes are positioned with explicit rectangles relative to
//! their parent, which is what a fixed-resolution panel actually wants; a
//! constraint solver can be added over this without changing anything below it.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod cursor;
mod node;
pub mod overlay;
mod ui;
pub mod widget;
pub mod widgets;

pub use cursor::{ARROW, CROSSHAIR, Cursor, CursorImage};
pub use node::NodeId;
pub use overlay::{Side, anchored};
pub use ui::Ui;
pub use widget::{Animation, Event, EventCtx, Handled, PaintCtx, VisualState, Void, Widget};
pub use widgets::{
    Alert, Align, Badge, Button, Checkbox, Divider, Label, List, ListItem, Orientation, Panel,
    Progress, RadioGroup, Slider, Tabs, TextInput, Toggle,
};

// Re-exported so an application names one crate rather than three to style a
// label, and so `FontId(0)` means the same thing everywhere.
pub use denise_text::{FontId, GlyphSource, TextEngine, TextStyle};

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
