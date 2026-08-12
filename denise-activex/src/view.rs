//! Where a design-time picture goes, worked out before any of it reaches GDI.
//!
//! `IViewObject::Draw` hands over a device context and a rectangle and asks for
//! the control's appearance in it. The rectangle comes from a container, which
//! means it can be anything: inverted, empty, or large enough that rendering it
//! at full size would allocate gigabytes. All of that is arithmetic, and
//! arithmetic is the part that goes wrong — so it lives here, outside
//! `cfg(windows)`, where it is tested on every machine rather than only on a CI
//! runner. The same split [`himetric`](crate::himetric) makes, for the same
//! reason.
//!
//! Plain integers rather than `RECTL` and [`denise::Size`] on purpose: both of
//! those crates are Windows-only dependencies of this one, and naming either
//! would put this module back behind the gate it is here to avoid.

/// The largest surface [`plan`] will ask for along either axis.
///
/// A design-time view is drawn at screen scale, where a control is a couple of
/// hundred pixels across; this bound exists for the cases that are not that. A
/// metafile or printer DC measures in its own units and can hand over a
/// rectangle hundreds of thousands wide, and `width * height * 4` bytes of that
/// is an allocation failure inside a form editor rather than a picture.
pub const MAX_EDGE: i32 = 4096;

/// A destination rectangle, and the surface to render for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Left edge of the destination, in the device context's units.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Destination width, always positive.
    pub width: i32,
    /// Destination height, always positive.
    pub height: i32,
    /// Width of the surface to render, at most [`MAX_EDGE`].
    pub source_width: u32,
    /// Height of the surface to render, at most [`MAX_EDGE`].
    pub source_height: u32,
}

impl Plan {
    /// Whether the blit scales rather than copies.
    ///
    /// False in the ordinary case, which is the point: rendering the tree at the
    /// destination's own size means the design-time picture is laid out for the
    /// size the container asked for rather than resampled from another one.
    pub const fn stretches(&self) -> bool {
        self.source_width as i32 != self.width || self.source_height as i32 != self.height
    }
}

/// Works out what to render and where to put it, or `None` for nothing to draw.
///
/// The four arguments are a `RECTL` as the container passed it. Nothing promises
/// they are the right way round — a mirrored or bottom-up device context hands
/// over an inverted one — so they are normalised rather than trusted.
pub fn plan(left: i32, top: i32, right: i32, bottom: i32) -> Option<Plan> {
    let x = left.min(right);
    let y = top.min(bottom);
    // Widened because `right - left` overflows an `i32` for a rectangle spanning
    // the whole range, and this argument comes from somebody else's code.
    let width = i64::from(left.max(right)) - i64::from(x);
    let height = i64::from(top.max(bottom)) - i64::from(y);
    if width == 0 || height == 0 {
        return None;
    }

    // One divisor for both axes, so an absurd rectangle produces a small picture
    // of the right shape rather than a squashed one. Integer, and rounded up, so
    // neither edge can come out above the bound.
    let longest = width.max(height);
    let bound = i64::from(MAX_EDGE);
    let divisor = ((longest + bound - 1) / bound).max(1);

    Some(Plan {
        x,
        y,
        width: width.min(i64::from(i32::MAX)) as i32,
        height: height.min(i64::from(i32::MAX)) as i32,
        source_width: (width / divisor).max(1) as u32,
        source_height: (height / divisor).max(1) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case that happens: a form editor asking for the control at its own
    /// size, which is rendered rather than resampled.
    #[test]
    fn an_ordinary_rectangle_is_rendered_at_its_own_size() {
        let plan = plan(10, 20, 210, 140).expect("something to draw");
        assert_eq!(
            (plan.x, plan.y, plan.width, plan.height),
            (10, 20, 200, 120)
        );
        assert_eq!((plan.source_width, plan.source_height), (200, 120));
        assert!(
            !plan.stretches(),
            "a 1:1 blit must not go through a resample"
        );
    }

    /// A mirrored or bottom-up device context hands over an inverted rectangle,
    /// and a negative width passed to `StretchBlt` mirrors the picture rather
    /// than failing — so this is a wrong drawing, not an error.
    #[test]
    fn an_inverted_rectangle_is_normalised_rather_than_trusted() {
        let plan = plan(210, 140, 10, 20).expect("something to draw");
        assert_eq!(
            (plan.x, plan.y, plan.width, plan.height),
            (10, 20, 200, 120)
        );
        assert_eq!((plan.source_width, plan.source_height), (200, 120));
    }

    /// Zero area is not an error: a container that asks for nothing gets nothing
    /// and an `S_OK`. Refusing it would put a failure in a form editor's log for
    /// a control that did as it was told.
    #[test]
    fn an_empty_rectangle_is_nothing_to_draw() {
        assert_eq!(plan(10, 20, 10, 140), None);
        assert_eq!(plan(10, 20, 210, 20), None);
        assert_eq!(plan(0, 0, 0, 0), None);
    }

    /// A metafile DC measures in its own units, so the rectangle can be enormous.
    /// The destination stays as asked; only the surface is bounded.
    #[test]
    fn an_enormous_rectangle_is_drawn_from_a_bounded_surface() {
        let plan = plan(0, 0, 100_000, 50_000).expect("something to draw");
        assert_eq!((plan.width, plan.height), (100_000, 50_000));
        assert!(plan.source_width <= MAX_EDGE as u32);
        assert!(plan.source_height <= MAX_EDGE as u32);
        assert!(plan.stretches());

        // One divisor for both axes: 2:1 in, 2:1 out. Clamping each edge
        // separately would have made this 4096x4096 and squashed the picture.
        let ratio = f64::from(plan.source_width) / f64::from(plan.source_height);
        assert!((ratio - 2.0).abs() < 0.01, "the aspect ratio was not kept");
    }

    /// The arguments come from a container, so they can be the extremes. A panic
    /// here unwinds out of a COM method into a host's message loop.
    #[test]
    fn the_widest_possible_rectangle_neither_overflows_nor_allocates() {
        let plan = plan(i32::MIN, i32::MIN, i32::MAX, i32::MAX).expect("something to draw");
        assert_eq!((plan.x, plan.y), (i32::MIN, i32::MIN));
        assert_eq!((plan.width, plan.height), (i32::MAX, i32::MAX));
        assert!(plan.source_width <= MAX_EDGE as u32);
        assert!(plan.source_height <= MAX_EDGE as u32);
    }

    /// A rectangle a pixel across is still a rectangle, and a surface zero
    /// pixels across is one `CreateDIBSection` refuses.
    #[test]
    fn a_sliver_still_produces_a_surface_with_pixels_in_it() {
        let plan = plan(0, 0, 1, 200_000).expect("something to draw");
        assert_eq!(
            plan.source_width, 1,
            "an edge may not be scaled away to zero"
        );
        assert!(plan.source_height >= 1);
    }
}
