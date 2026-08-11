//! Buffer rotation and buffer-age bookkeeping.
//!
//! Deliberately knows nothing about DRM. Which buffer to draw into next, and how
//! stale it is, is pure arithmetic — and it is the arithmetic that goes wrong. An
//! off-by-one here reports a two-frame-old buffer as one frame old, the renderer
//! repaints too little, and the display flickers at half the refresh rate on
//! hardware you cannot attach a debugger to.
//!
//! Keeping it separate means it is tested on a laptop instead of guessed at.

use denise::BufferAge;

/// Most buffers a swapchain may hold.
///
/// Two is the milestone default and what a Pi wants: triple buffering trades
/// latency for smoothness, which is the wrong trade for a control panel where a
/// touch should register now.
pub const MAX_BUFFERS: usize = 4;

/// Fewest buffers that can be flipped.
pub const MIN_BUFFERS: usize = 2;

/// Which buffer is next and how old its contents are.
#[derive(Clone, Debug)]
pub struct Swapchain {
    count: usize,
    current: usize,
    /// Frame number at which each buffer was last presented; `None` if never.
    presented_at: [Option<u64>; MAX_BUFFERS],
    /// Number of frames presented so far.
    frame: u64,
}

impl Swapchain {
    /// Creates a swapchain over `count` buffers, clamped to a sane range.
    pub fn new(count: usize) -> Self {
        Self {
            count: count.clamp(MIN_BUFFERS, MAX_BUFFERS),
            current: 0,
            presented_at: [None; MAX_BUFFERS],
            frame: 0,
        }
    }

    /// Number of buffers in rotation.
    #[inline]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Index of the buffer to draw into next.
    #[inline]
    pub const fn current(&self) -> usize {
        self.current
    }

    /// Total frames presented.
    #[inline]
    pub const fn frames_presented(&self) -> u64 {
        self.frame
    }

    /// How stale the current buffer's contents are.
    ///
    /// We are rendering what will become frame `frame + 1`. A buffer last presented
    /// as frame `then` therefore holds contents from `frame + 1 - then` frames ago.
    /// Getting that off by one makes single buffering report age zero, which reads
    /// as [`BufferAge::Undefined`] and silently turns every frame into a full
    /// repaint — correct on screen, and a total loss of the thing damage bought.
    pub fn age(&self) -> BufferAge {
        match self.presented_at[self.current] {
            Some(then) => BufferAge::Frames((self.frame + 1 - then) as u32),
            None => BufferAge::Undefined,
        }
    }

    /// Records that the current buffer has been presented, and advances.
    pub fn presented(&mut self) {
        self.frame += 1;
        self.presented_at[self.current] = Some(self.frame);
        self.current = (self.current + 1) % self.count;
    }

    /// Discards all history, so every buffer reports [`BufferAge::Undefined`].
    ///
    /// Call after a modeset or a resolution change: every buffer in flight is now
    /// the wrong shape or holds the previous mode's pixels.
    pub fn invalidate(&mut self) {
        self.presented_at = [None; MAX_BUFFERS];
        self.current = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_count_is_clamped() {
        assert_eq!(Swapchain::new(0).count(), MIN_BUFFERS);
        assert_eq!(Swapchain::new(1).count(), MIN_BUFFERS);
        assert_eq!(Swapchain::new(3).count(), 3);
        assert_eq!(Swapchain::new(99).count(), MAX_BUFFERS);
    }

    #[test]
    fn first_pass_over_every_buffer_is_undefined() {
        let mut sc = Swapchain::new(3);
        for _ in 0..3 {
            assert_eq!(sc.age(), BufferAge::Undefined);
            sc.presented();
        }
        // Now every buffer has been presented at least once.
        assert_ne!(sc.age(), BufferAge::Undefined);
    }

    #[test]
    fn double_buffering_reports_age_two() {
        let mut sc = Swapchain::new(2);
        sc.presented(); // buffer 0 presented as frame 1
        sc.presented(); // buffer 1 presented as frame 2
        // Back to buffer 0, which holds frame 1's contents while we render frame 3.
        assert_eq!(sc.age(), BufferAge::Frames(2));
    }

    #[test]
    fn triple_buffering_reports_age_three() {
        let mut sc = Swapchain::new(3);
        for _ in 0..3 {
            sc.presented();
        }
        assert_eq!(sc.age(), BufferAge::Frames(3));
    }

    #[test]
    fn age_equals_buffer_count_in_the_steady_state() {
        // The invariant the whole design rests on. If this ever drifts, the damage
        // tracker is being told to repaint the wrong amount.
        for count in MIN_BUFFERS..=MAX_BUFFERS {
            let mut sc = Swapchain::new(count);
            for _ in 0..count * 5 {
                sc.presented();
            }
            assert_eq!(
                sc.age(),
                BufferAge::Frames(count as u32),
                "{count} buffers settled to the wrong age"
            );
        }
    }

    #[test]
    fn buffers_are_visited_round_robin() {
        let mut sc = Swapchain::new(3);
        let visited: Vec<usize> = (0..7)
            .map(|_| {
                let i = sc.current();
                sc.presented();
                i
            })
            .collect();
        assert_eq!(visited, vec![0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn invalidate_forces_a_full_repaint_everywhere() {
        let mut sc = Swapchain::new(2);
        for _ in 0..10 {
            sc.presented();
        }
        sc.invalidate();
        assert_eq!(sc.current(), 0);
        for _ in 0..2 {
            assert_eq!(
                sc.age(),
                BufferAge::Undefined,
                "stale age survived a modeset"
            );
            sc.presented();
        }
    }

    #[test]
    fn age_never_exceeds_the_buffer_count() {
        // A larger age would mean claiming contents are older than any buffer can
        // be, which would make the tracker discard damage history it still needs.
        let mut sc = Swapchain::new(4);
        for _ in 0..50 {
            if let BufferAge::Frames(n) = sc.age() {
                assert!(
                    n as usize <= sc.count(),
                    "age {n} exceeds {} buffers",
                    sc.count()
                );
            }
            sc.presented();
        }
    }
}
