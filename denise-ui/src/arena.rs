//! The generational arena the tree lives in.
//!
//! A `Vec` of slots and a free list threaded through the empty ones. A key is a
//! slot's index and the generation the slot had when the key was issued; taking
//! a value out bumps the generation, so every key to it goes stale at once and
//! the next tenant of the slot is somebody no old key can name.
//!
//! This used to be the `slotmap` crate, and it was the last crate between the
//! widget tree and a dependency list of nothing. The tree asks six things of
//! its storage — put, take, look, look mutably, ask whether, count — which is
//! not enough to be worth somebody else's build script.

use alloc::vec::Vec;
use core::num::NonZeroU32;
use core::ops::{Index, IndexMut};

/// Identifies a node for exactly as long as that node exists.
///
/// The generation in the key is the point: an application that keeps an id
/// after removing the node gets `None` back, not somebody else's widget. That
/// is also why the tree stores ids rather than references — parent-linked
/// component graphs are what forced `Rc<RefCell<_>>` on CoreCanvas, and this
/// is the replacement.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    index: u32,
    /// Never zero, so that `Option<NodeId>` is no bigger than a `NodeId` — the
    /// tree holds a great many of those.
    generation: NonZeroU32,
}

impl NodeId {
    /// An id no arena ever issues: the index is one past the last a slot can have.
    const NONE: Self = Self {
        index: u32::MAX,
        generation: NonZeroU32::MIN,
    };

    /// The key as a plain `u64`, for carrying across the C ABI: the generation
    /// in the high half and the slot, counted from one, in the low half. Never
    /// `0`, which is what the C side uses for "no node".
    #[inline]
    pub fn as_ffi(self) -> u64 {
        (u64::from(self.generation.get()) << 32) | u64::from(self.index.wrapping_add(1))
    }

    /// Rebuilds a key from [`NodeId::as_ffi`]. A value that never came from there
    /// simply fails to resolve.
    #[inline]
    pub fn from_ffi(value: u64) -> Self {
        // Both halves are truncations on purpose: that is how the two were packed.
        let (slot, generation) = (value as u32, (value >> 32) as u32);
        match (slot.checked_sub(1), NonZeroU32::new(generation)) {
            (Some(index), Some(generation)) => Self { index, generation },
            _ => Self::NONE,
        }
    }
}

impl Default for NodeId {
    /// An id that resolves to nothing, in any tree.
    fn default() -> Self {
        Self::NONE
    }
}

impl core::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "NodeId({}v{})", self.index, self.generation)
    }
}

enum Entry<T> {
    Occupied(T),
    /// The next slot on the free list, if this is not the last of it.
    Free(Option<u32>),
}

struct Slot<T> {
    /// The generation of the key that names what is here now, or, when nothing
    /// is, of the key the next value put here will get.
    generation: NonZeroU32,
    entry: Entry<T>,
}

pub(crate) struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Option<u32>,
    len: usize,
}

impl<T> Arena<T> {
    pub(crate) const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: None,
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn insert(&mut self, value: T) -> NodeId {
        self.len += 1;
        if let Some(index) = self.free {
            let slot = &mut self.slots[index as usize];
            let Entry::Free(next) = slot.entry else {
                unreachable!("the free list only threads free slots");
            };
            self.free = next;
            slot.entry = Entry::Occupied(value);
            return NodeId {
                index,
                generation: slot.generation,
            };
        }
        // `u32::MAX` itself is `NodeId::NONE`'s, so the last slot is the one before.
        let index = u32::try_from(self.slots.len())
            .ok()
            .filter(|&index| index < u32::MAX)
            .expect("a tree of four thousand million nodes");
        let generation = NonZeroU32::MIN;
        self.slots.push(Slot {
            generation,
            entry: Entry::Occupied(value),
        });
        NodeId { index, generation }
    }

    pub(crate) fn remove(&mut self, id: NodeId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation || matches!(slot.entry, Entry::Free(_)) {
            return None;
        }
        // A slot that has run out of generations is retired rather than wrapped
        // round to ones it has already handed out: it stays empty and off the
        // free list, and costs a few bytes. Wrapping is the one way a stale id
        // could come to name a live node.
        let entry = match slot.generation.checked_add(1) {
            Some(next) => {
                slot.generation = next;
                Entry::Free(self.free.replace(id.index))
            }
            None => Entry::Free(None),
        };
        self.len -= 1;
        match core::mem::replace(&mut slot.entry, entry) {
            Entry::Occupied(value) => Some(value),
            Entry::Free(_) => unreachable!("checked above"),
        }
    }

    pub(crate) fn get(&self, id: NodeId) -> Option<&T> {
        match self.slots.get(id.index as usize) {
            Some(Slot {
                generation,
                entry: Entry::Occupied(value),
            }) if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    pub(crate) fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        match self.slots.get_mut(id.index as usize) {
            Some(Slot {
                generation,
                entry: Entry::Occupied(value),
            }) if *generation == id.generation => Some(value),
            _ => None,
        }
    }

    pub(crate) fn contains_key(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }
}

impl<T> Index<NodeId> for Arena<T> {
    type Output = T;

    fn index(&self, id: NodeId) -> &T {
        self.get(id).expect("a node that is no longer in the tree")
    }
}

impl<T> IndexMut<NodeId> for Arena<T> {
    fn index_mut(&mut self, id: NodeId) -> &mut T {
        self.get_mut(id)
            .expect("a node that is no longer in the tree")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_round_trip_preserves_identity() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        let b = arena.insert(2);
        assert_eq!(NodeId::from_ffi(a.as_ffi()), a);
        assert_eq!(NodeId::from_ffi(b.as_ffi()), b);
        assert_ne!(a.as_ffi(), b.as_ffi());
    }

    #[test]
    fn the_first_id_is_the_one_c_callers_have_always_seen() {
        // Generation one, slot one. Nobody should depend on it, and somebody does.
        let mut arena = Arena::new();
        assert_eq!(arena.insert(()).as_ffi(), 0x0000_0001_0000_0001);
    }

    #[test]
    fn no_id_crosses_the_abi_as_zero_and_zero_names_nothing() {
        let mut arena = Arena::new();
        let first = arena.insert(1);
        assert_ne!(first.as_ffi(), 0);
        assert_eq!(arena.get(NodeId::from_ffi(0)), None);
        assert_eq!(arena.get(NodeId::default()), None);
    }

    #[test]
    fn nonsense_from_the_other_side_of_the_abi_resolves_to_nothing() {
        let mut arena = Arena::new();
        arena.insert(1);
        for value in [
            1,
            1 << 32,
            u64::MAX,
            0xdead_beef_0000_0001,
            0x0000_0001_0000_0009,
        ] {
            assert_eq!(arena.get(NodeId::from_ffi(value)), None, "{value:#x}");
        }
    }

    #[test]
    fn a_stale_id_does_not_resolve_to_the_next_node() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        assert_eq!(arena.remove(a), Some(1));
        let b = arena.insert(2);
        assert_ne!(a, b);
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get_mut(a), None);
        assert!(!arena.contains_key(a));
        assert_eq!(arena.remove(a), None, "and cannot take the new tenant out");
        assert_eq!(arena.get(b), Some(&2));
    }

    #[test]
    fn a_vacated_slot_is_used_again_before_the_arena_grows() {
        let mut arena = Arena::new();
        let ids: Vec<NodeId> = (0..4).map(|n| arena.insert(n)).collect();
        arena.remove(ids[1]);
        arena.remove(ids[2]);
        assert_eq!(arena.len(), 2);
        let (c, d, e) = (arena.insert(10), arena.insert(11), arena.insert(12));
        assert_eq!(arena.slots.len(), 5, "two reused, one new");
        assert_eq!(arena.len(), 5);
        assert_eq!((arena[c], arena[d], arena[e]), (10, 11, 12));
        assert_eq!((arena[ids[0]], arena[ids[3]]), (0, 3));
    }

    #[test]
    fn removing_twice_takes_nothing_the_second_time() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        let b = arena.insert(2);
        assert_eq!(arena.remove(a), Some(1));
        assert_eq!(arena.remove(a), None);
        assert_eq!(arena.len(), 1);
        // The free list still holds the slot once, not twice.
        let c = arena.insert(3);
        let d = arena.insert(4);
        assert_eq!((arena[b], arena[c], arena[d]), (2, 3, 4));
    }

    #[test]
    fn a_slot_out_of_generations_is_retired_not_wrapped() {
        let mut arena = Arena::new();
        let first = arena.insert(1);
        arena.remove(first);
        arena.slots[0].generation = NonZeroU32::MAX;
        let last = arena.insert(2);
        assert_eq!(last.generation, NonZeroU32::MAX);
        assert_eq!(arena.remove(last), Some(2));
        assert_eq!(arena.get(last), None);
        assert_eq!(arena.len(), 0);

        let next = arena.insert(3);
        assert_eq!(next.index, 1, "the worn-out slot is not handed out again");
        assert_eq!(arena.get(first), None);
        assert_eq!(arena.get(last), None);
    }

    #[test]
    fn an_optional_id_is_no_bigger_than_an_id() {
        assert_eq!(size_of::<Option<NodeId>>(), size_of::<NodeId>());
        assert_eq!(size_of::<NodeId>(), 8);
    }

    #[test]
    #[should_panic(expected = "no longer in the tree")]
    fn indexing_with_a_stale_id_panics() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena.remove(a);
        let _ = arena[a];
    }
}
