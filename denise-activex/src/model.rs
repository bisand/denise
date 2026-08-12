//! The scriptable state, shared between the COM object and the tree it drives.
//!
//! Two objects need this: the [`DenisePanel`](crate::DenisePanel) a container
//! holds, which is where `Text` is assigned, and the delegate inside the child
//! window, which is where somebody types. Neither owns the other, so the state
//! sits in an `Rc<RefCell<..>>` between them.
//!
//! `Rc` rather than `Arc` because the class is registered
//! `ThreadingModel=Apartment`: the object, its window and its window procedure
//! all run on one thread, and a lock here would be ceremony around a contention
//! that cannot happen.
//!
//! # The one rule
//!
//! [`Model::inside`] is true while the tree's delegate is running. A property put
//! that arrives then came from an event handler the tree itself called, and
//! pushing it back into the tree from there would re-enter the control's own
//! `RefCell` — a panic, unwinding out through a COM method into the host. So a
//! put made while `inside` records the change and stops; the delegate applies it
//! before it returns.

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::System::Com::IDispatch;

use crate::connections::Connections;

/// The state, as both halves see it.
pub(crate) type Shared = Rc<RefCell<Model>>;

/// Everything a script can read, write or be told about.
pub(crate) struct Model {
    /// The field's contents. Written by a script and mirrored back out of the
    /// tree after every pass, which is what makes `Change` fire once.
    pub text: String,
    /// The heading.
    pub caption: String,
    /// Whether the field and the button take input.
    pub enabled: bool,
    /// A property changed and the tree has not applied it yet.
    pub dirty: bool,
    /// `Refresh` was called: repaint everything rather than just the damage.
    pub refresh: bool,
    /// True while the delegate is running. See the module comment.
    pub inside: bool,
    /// Advised event sinks.
    connections: Connections<IDispatch>,
}

impl Model {
    /// The state a freshly created control has, before any container speaks to it.
    pub fn new() -> Shared {
        Rc::new(RefCell::new(Self {
            text: String::new(),
            caption: "Denise".to_string(),
            enabled: true,
            // The tree is built from these, so there is nothing pending yet.
            dirty: false,
            refresh: false,
            inside: false,
            connections: Connections::new(),
        }))
    }

    /// Records a property change for the tree to pick up.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// Adds a sink and returns its cookie.
    pub fn advise(&mut self, sink: IDispatch) -> u32 {
        self.connections.advise(sink)
    }

    /// Removes a sink, reporting whether that cookie was connected.
    pub fn unadvise(&mut self, cookie: u32) -> bool {
        self.connections.unadvise(cookie)
    }

    /// The sinks to raise an event on, copied out so a handler may advise or
    /// unadvise while it runs.
    pub fn sinks(&self) -> Vec<IDispatch> {
        self.connections.sinks()
    }

    /// Drops every sink.
    ///
    /// A sink holds the control and the control holds the sink, so a container
    /// that forgets to unadvise leaves a cycle neither side can break. `Close` is
    /// where a control is allowed to break it for them.
    pub fn clear_sinks(&mut self) {
        self.connections.clear();
    }
}
