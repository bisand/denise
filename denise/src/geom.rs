//! Integer geometry.
//!
//! [`Size`] is unsigned because buffers cannot have negative extent. [`Rect`] is
//! signed throughout because clipping arithmetic routinely goes negative before it
//! is clamped, and doing that in unsigned space is a bug farm.

/// A point in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: i32,
    /// Vertical coordinate.
    pub y: i32,
}

impl Point {
    /// The origin.
    pub const ZERO: Self = Self { x: 0, y: 0 };

    /// Creates a point.
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A width/height pair in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Size {
    /// A zero-area size.
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
    };

    /// Creates a size.
    #[inline]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Returns `true` if either dimension is zero.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Pixel count, widened so large surfaces cannot overflow.
    #[inline]
    pub const fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// An axis-aligned rectangle in physical pixels.
///
/// A rectangle with a non-positive `width` or `height` is empty. Constructors clamp
/// negative extents to zero so that an empty rectangle is always well-formed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Extent along x. Never negative.
    pub width: i32,
    /// Extent along y. Never negative.
    pub height: i32,
}

impl Rect {
    /// An empty rectangle at the origin.
    pub const ZERO: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    /// Creates a rectangle, clamping negative extents to zero.
    #[inline]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: if width > 0 { width } else { 0 },
            height: if height > 0 { height } else { 0 },
        }
    }

    /// Creates a rectangle from edges. `right`/`bottom` are exclusive.
    #[inline]
    pub const fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self::new(
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        )
    }

    /// This rectangle at `scale` — **by its edges**, which is the part that
    /// matters.
    ///
    /// Scaling a rectangle's width and height directly looks equivalent and is
    /// not: two rectangles that share an edge in a logical layout would round
    /// their widths independently, and at a fractional scale that opens
    /// one-pixel seams between panels that were designed to touch. Scaling each
    /// *edge* and deriving the extent keeps shared edges shared at every scale,
    /// which is what lets an application design in logical units and multiply
    /// once — the DPI answer this toolkit gives; see `docs/design.md`.
    ///
    /// Rounds half away from zero, without `f32::round`, which lives in `std`.
    pub fn scaled(self, scale: f32) -> Rect {
        #[inline]
        fn round(v: f32) -> i32 {
            // `as i32` truncates towards zero, so the negative side needs its
            // half added in the other direction.
            if v >= 0.0 {
                (v + 0.5) as i32
            } else {
                -((0.5 - v) as i32)
            }
        }
        Rect::from_edges(
            round(self.x as f32 * scale),
            round(self.y as f32 * scale),
            round(self.right() as f32 * scale),
            round(self.bottom() as f32 * scale),
        )
    }

    /// A rectangle covering a whole surface, anchored at the origin.
    #[inline]
    pub const fn from_size(size: Size) -> Self {
        Self::new(0, 0, size.width as i32, size.height as i32)
    }

    /// Exclusive right edge.
    #[inline]
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    /// Exclusive bottom edge.
    #[inline]
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    /// Returns `true` if the rectangle covers no pixels.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// Pixel count.
    #[inline]
    pub const fn area(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            self.width as u64 * self.height as u64
        }
    }

    /// Returns `true` if `p` lies inside the rectangle.
    #[inline]
    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }

    /// Returns `true` if `other` lies entirely inside `self`. Empty rectangles are
    /// contained by everything.
    #[inline]
    pub const fn contains_rect(&self, other: &Rect) -> bool {
        other.is_empty()
            || (other.x >= self.x
                && other.y >= self.y
                && other.right() <= self.right()
                && other.bottom() <= self.bottom())
    }

    /// Returns `true` if the two rectangles share at least one pixel.
    #[inline]
    pub const fn intersects(&self, other: &Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Returns `true` if the rectangles intersect or share an edge. Used by damage
    /// coalescing, where two abutting rectangles are worth merging.
    #[inline]
    pub const fn touches(&self, other: &Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    /// Intersection, or `None` when the rectangles are disjoint.
    #[inline]
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let r = Rect::from_edges(
            self.x.max(other.x),
            self.y.max(other.y),
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        (!r.is_empty()).then_some(r)
    }

    /// Smallest rectangle containing both. An empty operand is ignored.
    #[inline]
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::from_edges(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// Moves the rectangle without changing its extent.
    #[inline]
    pub const fn translate(&self, dx: i32, dy: i32) -> Rect {
        Self {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
            width: self.width,
            height: self.height,
        }
    }

    /// Grows the rectangle by `d` on every side. Negative `d` shrinks it.
    #[inline]
    pub fn inflate(&self, d: i32) -> Rect {
        Rect::from_edges(
            self.x.saturating_sub(d),
            self.y.saturating_sub(d),
            self.right().saturating_add(d),
            self.bottom().saturating_add(d),
        )
    }

    /// Clips to a surface of `size`, returning `None` if nothing remains.
    #[inline]
    pub fn clip_to_size(&self, size: Size) -> Option<Rect> {
        self.intersect(&Rect::from_size(size))
    }
}

#[cfg(test)]
mod tests {

    /// The reason `Rect::scaled` works by edges: two rectangles that touch in
    /// the logical layout must still touch at every scale. Independent
    /// width-rounding is what opens the one-pixel seam.
    #[test]
    fn scaling_keeps_shared_edges_shared() {
        for scale in [0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let left = Rect::new(20, 40, 155, 60);
            let right = Rect::new(left.right(), 40, 133, 60);
            let (a, b) = (left.scaled(scale), right.scaled(scale));
            assert_eq!(
                a.right(),
                b.x,
                "scale {scale}: a seam opened between adjacent rectangles"
            );
        }
    }

    /// Identity at 1.0, exact doubling at 2.0 — the cases an application can
    /// check with its own eyes.
    #[test]
    fn scaling_is_exact_at_whole_factors() {
        let r = Rect::new(20, 44, 388, 34);
        assert_eq!(r.scaled(1.0), r);
        assert_eq!(r.scaled(2.0), Rect::new(40, 88, 776, 68));
    }

    /// Negative coordinates round like positive ones — towards the nearest
    /// pixel, not towards zero. A rect partially off-screen is ordinary.
    #[test]
    fn scaling_rounds_negative_edges_to_nearest() {
        let r = Rect::new(-10, -10, 20, 20).scaled(1.5);
        assert_eq!(r, Rect::new(-15, -15, 30, 30));
        // The half-pixel case, both signs, same distance moved.
        let r = Rect::new(-3, 3, 6, 6).scaled(1.5);
        assert_eq!(r.x, -5, "-4.5 rounds away from zero");
        assert_eq!(r.bottom(), 14, "13.5 rounds away from zero");
    }

    use super::*;

    #[test]
    fn negative_extent_is_clamped() {
        let r = Rect::new(10, 10, -5, -5);
        assert!(r.is_empty());
        assert_eq!(r.area(), 0);
    }

    #[test]
    fn from_edges_handles_inverted_input() {
        assert!(Rect::from_edges(20, 20, 10, 10).is_empty());
        assert_eq!(Rect::from_edges(1, 2, 5, 9), Rect::new(1, 2, 4, 7));
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&Rect::new(50, 50, 1, 1)), None);
    }

    #[test]
    fn union_ignores_empty_operands() {
        let a = Rect::new(3, 4, 5, 6);
        assert_eq!(a.union(&Rect::ZERO), a);
        assert_eq!(Rect::ZERO.union(&a), a);
    }

    #[test]
    fn touching_but_not_intersecting() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(10, 0, 10, 10);
        assert!(!a.intersects(&b));
        assert!(a.touches(&b));
    }

    #[test]
    fn containment() {
        let outer = Rect::new(0, 0, 100, 100);
        assert!(outer.contains_rect(&Rect::new(10, 10, 10, 10)));
        assert!(!outer.contains_rect(&Rect::new(95, 95, 10, 10)));
        assert!(outer.contains(Point::new(99, 99)));
        assert!(!outer.contains(Point::new(100, 0)));
    }
}
