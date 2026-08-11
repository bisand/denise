//! Dirty-rectangle tracking.
//!
//! Damage is core, not a later optimisation: it is what makes a 1920×1080 panel
//! redraw a blinking cursor for the cost of a blinking cursor. Everything here is
//! fixed-capacity and allocation-free so it can sit in the render hot path.
//!
//! Two things this handles that a naive dirty-rect list does not:
//!
//! - **Coalescing.** Beyond [`MAX_DAMAGE_RECTS`] regions, tracking individual
//!   rectangles costs more than it saves, so the list collapses to its bounds.
//! - **Buffer age.** With double buffering, the buffer you are handed is two frames
//!   old. Repainting only the current frame's damage leaves stale pixels in the
//!   other buffer, which shows up as flicker at exactly the refresh rate.

use crate::geom::{Rect, Size};
use crate::surface::BufferAge;

/// Maximum tracked regions before the list collapses to its bounding box.
pub const MAX_DAMAGE_RECTS: usize = 16;

/// Frames of damage history kept, and therefore the deepest swapchain that can be
/// repainted incrementally. Anything older forces a full repaint.
pub const MAX_TRACKED_FRAMES: usize = 4;

/// A fixed-capacity, self-coalescing set of rectangles.
#[derive(Clone, Copy, Debug)]
pub struct RectList {
    rects: [Rect; MAX_DAMAGE_RECTS],
    len: usize,
}

impl Default for RectList {
    fn default() -> Self {
        Self::new()
    }
}

impl RectList {
    /// An empty list.
    pub const fn new() -> Self {
        Self {
            rects: [Rect::ZERO; MAX_DAMAGE_RECTS],
            len: 0,
        }
    }

    /// Discards every rectangle.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// The rectangles, in no particular order. Guaranteed non-overlapping only in
    /// the sense that overlapping inputs were merged; abutting ones may remain.
    #[inline]
    pub fn as_slice(&self) -> &[Rect] {
        &self.rects[..self.len]
    }

    /// Number of rectangles.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if nothing is damaged.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Total area, counting any residual overlap once per rectangle.
    pub fn area(&self) -> u64 {
        self.as_slice().iter().map(Rect::area).sum()
    }

    /// Smallest rectangle covering the whole list.
    pub fn bounds(&self) -> Rect {
        self.as_slice()
            .iter()
            .fold(Rect::ZERO, |acc, r| acc.union(r))
    }

    /// Replaces the list with a single rectangle.
    pub fn set_single(&mut self, rect: Rect) {
        self.len = 0;
        if !rect.is_empty() {
            self.rects[0] = rect;
            self.len = 1;
        }
    }

    /// Adds a rectangle, merging it with anything it touches.
    ///
    /// Merging on contact rather than only on overlap is deliberate: two abutting
    /// scanline runs cost less as one rectangle than as two.
    pub fn insert(&mut self, rect: Rect) {
        let mut rect = rect;
        if rect.is_empty() {
            return;
        }

        // Absorb into an existing rectangle if one already covers this.
        if self.as_slice().iter().any(|e| e.contains_rect(&rect)) {
            return;
        }

        // Merge with everything the (growing) rectangle touches. Restart after each
        // merge because growing can bring further rectangles into contact.
        let mut merged = true;
        while merged {
            merged = false;
            let mut i = 0;
            while i < self.len {
                if self.rects[i].touches(&rect) || rect.contains_rect(&self.rects[i]) {
                    rect = rect.union(&self.rects[i]);
                    self.len -= 1;
                    self.rects[i] = self.rects[self.len];
                    merged = true;
                } else {
                    i += 1;
                }
            }
        }

        if self.len == MAX_DAMAGE_RECTS {
            // Out of slots: tracking finer detail now costs more than it saves.
            let collapsed = self.bounds().union(&rect);
            self.set_single(collapsed);
            return;
        }

        self.rects[self.len] = rect;
        self.len += 1;
    }

    /// Adds every rectangle from another list.
    pub fn insert_all(&mut self, rects: &[Rect]) {
        for r in rects {
            self.insert(*r);
        }
    }
}

/// Accumulates per-frame damage and resolves it against buffer age.
///
/// One tracker per surface. Call [`add`](DamageTracker::add) as things change,
/// [`resolve`](DamageTracker::resolve) once you know the age of the buffer you got,
/// and [`end_frame`](DamageTracker::end_frame) after presenting.
#[derive(Clone, Debug)]
pub struct DamageTracker {
    history: [RectList; MAX_TRACKED_FRAMES],
    head: usize,
    resolved: RectList,
    surface: Size,
    force_full: bool,
}

impl DamageTracker {
    /// Creates a tracker for a surface of `size`, starting fully damaged.
    pub fn new(size: Size) -> Self {
        Self {
            history: [RectList::new(); MAX_TRACKED_FRAMES],
            head: 0,
            resolved: RectList::new(),
            surface: size,
            force_full: true,
        }
    }

    /// Surface extent damage is clipped against.
    #[inline]
    pub const fn surface_size(&self) -> Size {
        self.surface
    }

    /// Resizes the surface. Every buffer in flight is now the wrong shape, so all
    /// history is discarded and the next frame is a full repaint.
    pub fn resize(&mut self, size: Size) {
        if size == self.surface {
            return;
        }
        self.surface = size;
        for list in &mut self.history {
            list.clear();
        }
        self.resolved.clear();
        self.force_full = true;
    }

    /// Marks a region of the current frame dirty. Clipped to the surface.
    pub fn add(&mut self, rect: Rect) {
        if self.force_full {
            return;
        }
        if let Some(clipped) = rect.clip_to_size(self.surface) {
            self.history[self.head].insert(clipped);
        }
    }

    /// Marks the whole surface dirty for this frame.
    pub fn add_full(&mut self) {
        self.force_full = true;
    }

    /// Returns `true` if nothing has been marked dirty this frame.
    #[inline]
    pub fn is_clean(&self) -> bool {
        !self.force_full && self.history[self.head].is_empty()
    }

    /// Damage that must actually be repainted into a buffer of the given age.
    ///
    /// For [`BufferAge::Frames(n)`](BufferAge::Frames) this is the union of the
    /// last `n` frames' damage, current frame included. For
    /// [`BufferAge::Undefined`], or an age deeper than [`MAX_TRACKED_FRAMES`], it
    /// is the whole surface.
    ///
    /// Pass the result to both the renderer and [`crate::Surface::present`].
    pub fn resolve(&mut self, age: BufferAge) -> &[Rect] {
        let full = Rect::from_size(self.surface);

        let frames = match age {
            BufferAge::Undefined => 0,
            BufferAge::Frames(n) => n as usize,
        };

        if self.force_full || frames == 0 || frames > MAX_TRACKED_FRAMES {
            self.resolved.set_single(full);
            return self.resolved.as_slice();
        }

        self.resolved.clear();
        for i in 0..frames {
            let idx = (self.head + MAX_TRACKED_FRAMES - i) % MAX_TRACKED_FRAMES;
            self.resolved.insert_all(self.history[idx].as_slice());
        }

        // Once the damage covers most of the surface, one rectangle beats many:
        // the per-rect setup cost stops paying for itself.
        if self.resolved.len() > 1 && self.resolved.area() * 4 >= full.area() * 3 {
            self.resolved.set_single(full);
        }

        self.resolved.as_slice()
    }

    /// The slice most recently returned by [`resolve`](DamageTracker::resolve).
    #[inline]
    pub fn resolved(&self) -> &[Rect] {
        self.resolved.as_slice()
    }

    /// Advances to the next frame, retiring the oldest history slot.
    pub fn end_frame(&mut self) {
        if self.force_full {
            // The full-repaint frame we just drew supersedes all history.
            for list in &mut self.history {
                list.clear();
            }
            self.history[self.head].set_single(Rect::from_size(self.surface));
            self.force_full = false;
        }
        self.head = (self.head + 1) % MAX_TRACKED_FRAMES;
        self.history[self.head].clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: Size = Size::new(200, 100);

    fn tracker() -> DamageTracker {
        let mut t = DamageTracker::new(SURFACE);
        // Burn the initial forced-full frame so tests start from a clean slate.
        t.resolve(BufferAge::Undefined);
        t.end_frame();
        t
    }

    #[test]
    fn first_frame_is_always_full() {
        let mut t = DamageTracker::new(SURFACE);
        assert_eq!(t.resolve(BufferAge::Frames(1)), &[Rect::from_size(SURFACE)]);
    }

    #[test]
    fn undefined_age_forces_full_repaint() {
        let mut t = tracker();
        t.add(Rect::new(0, 0, 4, 4));
        assert_eq!(t.resolve(BufferAge::Undefined), &[Rect::from_size(SURFACE)]);
    }

    #[test]
    fn single_buffer_sees_only_current_damage() {
        let mut t = tracker();
        t.add(Rect::new(10, 10, 5, 5));
        assert_eq!(t.resolve(BufferAge::Frames(1)), &[Rect::new(10, 10, 5, 5)]);
    }

    #[test]
    fn double_buffer_unions_previous_frame() {
        let mut t = tracker();
        t.add(Rect::new(0, 0, 5, 5));
        t.resolve(BufferAge::Frames(1));
        t.end_frame();

        t.add(Rect::new(100, 50, 5, 5));
        let damage = t.resolve(BufferAge::Frames(2)).to_vec();

        // The buffer we were handed never saw the first rectangle. Both must repaint.
        assert_eq!(damage.len(), 2);
        assert!(damage.contains(&Rect::new(0, 0, 5, 5)));
        assert!(damage.contains(&Rect::new(100, 50, 5, 5)));
    }

    #[test]
    fn age_deeper_than_history_is_full_repaint() {
        let mut t = tracker();
        t.add(Rect::new(1, 1, 2, 2));
        assert_eq!(
            t.resolve(BufferAge::Frames(MAX_TRACKED_FRAMES as u32 + 1)),
            &[Rect::from_size(SURFACE)]
        );
    }

    #[test]
    fn damage_is_clipped_to_surface() {
        let mut t = tracker();
        t.add(Rect::new(-50, -50, 100, 100));
        assert_eq!(t.resolve(BufferAge::Frames(1)), &[Rect::new(0, 0, 50, 50)]);
    }

    #[test]
    fn offscreen_damage_is_dropped() {
        let mut t = tracker();
        t.add(Rect::new(500, 500, 10, 10));
        assert!(t.is_clean());
    }

    #[test]
    fn resize_invalidates_history() {
        let mut t = tracker();
        t.add(Rect::new(0, 0, 5, 5));
        t.resize(Size::new(320, 240));
        assert_eq!(
            t.resolve(BufferAge::Frames(1)),
            &[Rect::from_size(Size::new(320, 240))]
        );
    }

    #[test]
    fn touching_rects_merge() {
        let mut list = RectList::new();
        list.insert(Rect::new(0, 0, 10, 10));
        list.insert(Rect::new(10, 0, 10, 10));
        assert_eq!(list.as_slice(), &[Rect::new(0, 0, 20, 10)]);
    }

    #[test]
    fn contained_rect_is_absorbed() {
        let mut list = RectList::new();
        list.insert(Rect::new(0, 0, 100, 100));
        list.insert(Rect::new(10, 10, 10, 10));
        assert_eq!(list.as_slice(), &[Rect::new(0, 0, 100, 100)]);
    }

    #[test]
    fn overflow_collapses_to_bounds() {
        let mut list = RectList::new();
        let spaced = |i: i32| Rect::new(i * 20, i * 20, 4, 4);

        // Spaced apart so nothing merges, right up to capacity.
        for i in 0..MAX_DAMAGE_RECTS as i32 {
            list.insert(spaced(i));
        }
        assert_eq!(list.len(), MAX_DAMAGE_RECTS);

        // One more has nowhere to go, so the whole list becomes its bounds.
        list.insert(spaced(MAX_DAMAGE_RECTS as i32));
        assert_eq!(list.len(), 1);
        for i in 0..=MAX_DAMAGE_RECTS as i32 {
            assert!(list.as_slice()[0].contains_rect(&spaced(i)));
        }
    }

    #[test]
    fn coalescing_never_loses_coverage() {
        let mut list = RectList::new();
        let spaced = |i: i32| Rect::new(i * 20, i * 20, 4, 4);
        let n = MAX_DAMAGE_RECTS as i32 * 3;

        for i in 0..n {
            list.insert(spaced(i));
        }

        // Whatever the list collapsed into, every rectangle ever inserted must
        // still be covered. Damage that is dropped is damage that is not repainted.
        assert!(list.len() <= MAX_DAMAGE_RECTS);
        for i in 0..n {
            let r = spaced(i);
            assert!(
                list.as_slice().iter().any(|e| e.contains_rect(&r)),
                "{r:?} was lost by coalescing"
            );
        }
    }

    #[test]
    fn mostly_covered_surface_collapses() {
        let mut t = tracker();
        t.add(Rect::new(0, 0, 200, 40));
        t.add(Rect::new(0, 60, 200, 40));
        // 80% covered by two disjoint bands: one full-surface blit is cheaper.
        assert_eq!(t.resolve(BufferAge::Frames(1)), &[Rect::from_size(SURFACE)]);
    }

    #[test]
    fn full_repaint_is_recorded_in_history() {
        let mut t = tracker();
        t.add_full();
        t.resolve(BufferAge::Frames(1));
        t.end_frame();

        // The next frame's back buffer is one behind and still holds pre-full-repaint
        // content, so last frame's full damage must be replayed.
        t.add(Rect::new(0, 0, 1, 1));
        assert_eq!(t.resolve(BufferAge::Frames(2)), &[Rect::from_size(SURFACE)]);
    }
}
