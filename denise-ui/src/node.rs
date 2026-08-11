//! Tree storage: one node per widget, addressed by a generational key.

use alloc::vec::Vec;

use denise::Rect;

use crate::widget::{BoxedWidget, VisualState};

slotmap::new_key_type! {
    /// Identifies a node for exactly as long as that node exists.
    ///
    /// The generation in the key is the point: an application that keeps an id
    /// after removing the node gets `None` back, not somebody else's widget. That
    /// is also why the tree stores ids rather than references — parent-linked
    /// component graphs are what forced `Rc<RefCell<_>>` on CoreCanvas, and this
    /// is the replacement.
    pub struct NodeId;
}

impl NodeId {
    /// The key as a plain `u64`, for carrying across the C ABI in M5.
    #[inline]
    pub fn as_ffi(self) -> u64 {
        use slotmap::Key as _;
        self.data().as_ffi()
    }

    /// Rebuilds a key from [`NodeId::as_ffi`]. A value that never came from there
    /// simply fails to resolve.
    #[inline]
    pub fn from_ffi(value: u64) -> Self {
        use slotmap::KeyData;
        NodeId::from(KeyData::from_ffi(value))
    }
}

pub(crate) struct Node<M> {
    pub(crate) widget: BoxedWidget<M>,
    /// Position and extent relative to the parent's origin.
    pub(crate) layout: Rect,
    /// Absolute bounds, recomputed when the node or an ancestor moves.
    pub(crate) bounds: Rect,
    /// [`Node::bounds`] intersected with every ancestor's bounds: what the widget
    /// may actually paint into, and what damage from this node covers.
    pub(crate) clip: Rect,
    /// Sort key among siblings. Ties keep insertion order.
    pub(crate) z: i32,
    pub(crate) visible: bool,
    pub(crate) enabled: bool,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    /// Which scene this node belongs to, as an index into the stack.
    pub(crate) scene: usize,
    pub(crate) state: VisualState,
}

impl<M> Node<M> {
    pub(crate) fn new(widget: BoxedWidget<M>, layout: Rect, scene: usize) -> Self {
        Self {
            widget,
            layout,
            bounds: layout,
            clip: layout,
            z: 0,
            visible: true,
            enabled: true,
            parent: None,
            children: Vec::new(),
            scene,
            state: VisualState::NONE,
        }
    }

    /// Returns `true` if this node can be painted at all.
    #[inline]
    pub(crate) fn paintable(&self) -> bool {
        self.visible && !self.clip.is_empty()
    }
}

/// One layer of the scene stack.
///
/// The stack is how CoreCanvas did dialogs and it is how Denise does them: a modal
/// is not a widget inside the page, it is a scene pushed on top of it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Scene {
    pub(crate) root: NodeId,
    /// Alpha of the backdrop painted under this scene, `0` for none.
    pub(crate) dim: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_round_trip_preserves_identity() {
        use slotmap::SlotMap;
        let mut map: SlotMap<NodeId, u32> = SlotMap::with_key();
        let a = map.insert(1);
        assert_eq!(NodeId::from_ffi(a.as_ffi()), a);
    }

    #[test]
    fn a_stale_id_does_not_resolve_to_the_next_node() {
        use slotmap::SlotMap;
        let mut map: SlotMap<NodeId, u32> = SlotMap::with_key();
        let a = map.insert(1);
        map.remove(a);
        let b = map.insert(2);
        assert_ne!(a, b);
        assert_eq!(map.get(a), None);
    }
}
