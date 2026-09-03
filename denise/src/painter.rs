//! The drawing contract widgets paint through.
//!
//! a rasteriser is one implementation of it — the software rasteriser this crate
//! exists for. The point of the trait is that it need not be the only one: a
//! backend that hands work to a GPU implements the same operations, and every
//! widget already written keeps working against it unchanged.
//!
//! # Why the colours are `Paint` and not `impl Into<Paint>`
//!
//! Widgets are trait objects, so `Widget::paint` can only ever be handed a
//! `&mut dyn Painter`, and a trait with generic methods is not object-safe.
//! a rasteriser's inherent methods keep taking `impl Into<Paint>` — code holding a
//! concrete canvas is unaffected — but the trait itself takes the premultiplied
//! form, and widget code converts at the call site.

use crate::Color;
use crate::geom::{Point, Rect, Size};
use crate::icon::Icon;
use crate::paint::Paint;
use crate::pixels::{AtlasPage, ImageRef, Mask, PixelView};
use crate::surface::PixelFormat;

/// Proof that a clip was narrowed, and the only way to widen one back.
///
/// Held by [`Clipped`] and consumed by [`Painter::pop_clip`]. The field is
/// private, so the sole way for widget code to obtain one is to have narrowed
/// the clip first: a child still cannot draw outside the region its parent gave
/// it. Backends implementing [`Painter`] by hand build tokens with
/// [`ClipToken::restoring`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipToken(Rect);

impl ClipToken {
    /// The clip to restore. Backend plumbing; widget code wants
    /// [`PainterExt::with_clip`].
    #[doc(hidden)]
    #[inline]
    pub const fn restoring(previous: Rect) -> Self {
        Self(previous)
    }

    /// The rectangle this token restores.
    #[inline]
    pub const fn previous(self) -> Rect {
        self.0
    }
}

/// A clipped drawing target.
///
/// Coordinates are physical pixels relative to the target origin, never relative
/// to the clip. Angles are binary turns, as everywhere else in this crate: `0`
/// is twelve o'clock, clockwise positive, [`TURN`](crate::angle::TURN) is a full circle.
///
/// Clipping is rectangular. That covers scrolling regions, damage-restricted
/// repaint and nested panels, which is all a UI actually needs; arbitrary clip
/// shapes are not planned, and their absence is what keeps this trait small
/// enough for a backend to implement in an afternoon.
pub trait Painter {
    // ---- state ------------------------------------------------------------

    /// Full extent of the target.
    fn size(&self) -> Size;

    /// Word layout of the target.
    fn format(&self) -> PixelFormat;

    /// The region operations are currently restricted to.
    fn clip(&self) -> Rect;

    /// Narrows the clip to its intersection with `rect`, returning the old one.
    ///
    /// Never widens. Prefer [`PainterExt::with_clip`], which pairs this with its
    /// [`pop_clip`](Painter::pop_clip) for you.
    fn push_clip(&mut self, rect: Rect) -> ClipToken;

    /// Restores the clip a [`push_clip`](Painter::push_clip) replaced.
    fn pop_clip(&mut self, token: ClipToken);

    // ---- primitives -------------------------------------------------------

    /// Fills the entire clip with an opaque colour.
    fn clear(&mut self, color: Color);

    /// Fills a rectangle.
    fn fill_rect(&mut self, rect: Rect, paint: Paint);

    /// Fills a rectangle with rounded corners.
    fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, paint: Paint);

    /// Strokes a rounded rectangle inside its bounds.
    fn stroke_rounded_rect(&mut self, rect: Rect, radius: i32, thickness: i32, paint: Paint);

    /// Fills a circle.
    fn fill_circle(&mut self, centre: Point, radius: i32, paint: Paint);

    /// Strokes a circle inside its bounds.
    fn stroke_circle(&mut self, centre: Point, radius: i32, thickness: i32, paint: Paint);

    /// Strokes part of a circle, from `start` through `sweep` binary turns.
    fn stroke_arc(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        start: i32,
        sweep: i32,
        paint: Paint,
    );

    /// Draws a one-pixel line.
    fn draw_line(&mut self, a: Point, b: Point, paint: Paint);

    /// Fills a polygon whose vertices are in the rasteriser's 8.8 fixed point.
    ///
    /// The primitive icons and stars are made of, which is why it is on the
    /// trait rather than private to the software backend: a GPU answers it with
    /// a triangle fan and gets both for free.
    fn fill_polygon_fx(&mut self, points: &[(i32, i32)], paint: Paint);

    /// Composites an 8-bit coverage mask. How glyphs arrive.
    fn blit_mask(&mut self, at: Point, mask: &Mask<'_>, paint: Paint);

    /// Composites the glyph at `rect` of an atlas `page`.
    ///
    /// Provided as "cut the rectangle out and [`blit_mask`](Painter::blit_mask)
    /// it", which is all the software rasteriser wants. A backend that keeps
    /// textures overrides it to upload the page once per
    /// [`version`](AtlasPage::version) and draw rectangles of it from then on,
    /// which is the difference between a glyph costing an upload and costing
    /// six vertices.
    fn blit_glyph(&mut self, at: Point, page: &AtlasPage<'_>, rect: Rect, paint: Paint) {
        if let Some(mask) = page.mask.sub(rect) {
            self.blit_mask(at, &mask, paint);
        }
    }

    /// Copies a premultiplied source over the target at `at`.
    fn blit(&mut self, src: &PixelView<'_>, at: Point);

    /// Copies a premultiplied source, scaled to `dest`.
    fn blit_scaled(&mut self, src: &PixelView<'_>, dest: Rect);

    /// Copies a premultiplied source, scaled to `dest` and masked to a rounded
    /// `shape`.
    fn blit_rounded(&mut self, src: &PixelView<'_>, dest: Rect, shape: Rect, radius: i32);

    /// Draws an image, scaled to `dest`.
    ///
    /// Provided as [`blit_scaled`](Painter::blit_scaled) of the pixels, which
    /// is all the software rasteriser wants. A backend that keeps textures
    /// overrides it to upload the image once per
    /// [`version`](ImageRef::version) and draw a quad from then on.
    fn blit_image(&mut self, src: &ImageRef<'_>, dest: Rect) {
        self.blit_scaled(&src.view, dest);
    }

    /// Draws an image, scaled to `dest` and masked to a rounded `shape`.
    ///
    /// The rounded counterpart of [`blit_image`](Painter::blit_image), provided
    /// as [`blit_rounded`](Painter::blit_rounded) of the pixels.
    fn blit_image_rounded(&mut self, src: &ImageRef<'_>, dest: Rect, shape: Rect, radius: i32) {
        self.blit_rounded(&src.view, dest, shape, radius);
    }

    // ---- derived ----------------------------------------------------------
    // Written in terms of the primitives above, so a new backend need not
    // supply them -- but left overridable, because a batching backend wants
    // `fill_rects` to become one instanced draw rather than N.

    /// Returns `true` if the clip admits no pixels, so drawing can be skipped.
    fn is_clipped_out(&self) -> bool {
        self.clip().is_empty()
    }

    /// The clipped, visible part of `rect`.
    fn visible(&self, rect: Rect) -> Option<Rect> {
        self.clip().intersect(&rect)
    }

    /// Fills several rectangles in one colour.
    fn fill_rects(&mut self, rects: &[Rect], paint: Paint) {
        for rect in rects {
            self.fill_rect(*rect, paint);
        }
    }

    /// Strokes a rectangle inside its bounds.
    fn stroke_rect(&mut self, rect: Rect, thickness: i32, paint: Paint) {
        let t = thickness.max(0);
        if t == 0 || rect.is_empty() || paint.is_invisible() {
            return;
        }
        if t * 2 >= rect.width.min(rect.height) {
            self.fill_rect(rect, paint);
            return;
        }
        let inner_height = rect.height - 2 * t;
        self.fill_rect(Rect::new(rect.x, rect.y, rect.width, t), paint);
        self.fill_rect(Rect::new(rect.x, rect.bottom() - t, rect.width, t), paint);
        self.fill_rect(Rect::new(rect.x, rect.y + t, t, inner_height), paint);
        self.fill_rect(
            Rect::new(rect.right() - t, rect.y + t, t, inner_height),
            paint,
        );
    }

    /// Fills a star with `points` points.
    ///
    /// See `denise-render` for what the binary-turn rounding costs.
    fn fill_star(
        &mut self,
        centre: Point,
        outer_radius: i32,
        inner_radius: i32,
        points: u32,
        rotation: i32,
        paint: Paint,
    ) {
        use crate::angle::{COORD_LIMIT, direction, to_fx};
        use crate::icon::MAX_ICON_VERTICES;

        if points < 2 || outer_radius <= 0 || paint.is_invisible() {
            return;
        }
        let count = (points as usize) * 2;
        if count > MAX_ICON_VERTICES {
            return;
        }
        let outer = outer_radius.clamp(0, COORD_LIMIT) as i64;
        let inner = inner_radius.clamp(0, outer_radius) as i64;

        let mut vertices = [(0i32, 0i32); MAX_ICON_VERTICES];
        let (cx, cy) = (to_fx(centre.x), to_fx(centre.y));
        for (i, vertex) in vertices.iter_mut().enumerate().take(count) {
            // Rounded, not truncated: the spacing error is then at most half a
            // unit per vertex rather than a whole one.
            let step = (i as i64 * crate::angle::TURN as i64 + count as i64 / 2) / count as i64;
            let angle = rotation.wrapping_add(step as i32);
            let (dx, dy) = direction(angle);
            let r = if i % 2 == 0 { outer } else { inner };
            // The direction is Q16 and the target is 8.8, so a radius in whole
            // pixels lands sub-pixel after shifting off eight bits.
            *vertex = (
                cx + ((dx as i64 * r) >> 8) as i32,
                cy + ((dy as i64 * r) >> 8) as i32,
            );
        }
        self.fill_polygon_fx(&vertices[..count], paint);
    }

    /// Draws an icon scaled to `rect`, in two inks.
    fn draw_icon(&mut self, icon: &Icon, rect: Rect, fore: Color, back: Color) {
        use crate::icon::{GRID, Ink, MAX_SHAPES, fx_along};

        if rect.is_empty() || GRID <= 0 {
            return;
        }
        for shape in icon.shapes.iter().take(MAX_SHAPES) {
            let mut points = [(0i32, 0i32); crate::icon::MAX_ICON_VERTICES];
            let n = shape.points.len().min(crate::icon::MAX_ICON_VERTICES);
            if n < 3 {
                continue;
            }
            for (slot, &(gx, gy)) in points.iter_mut().zip(shape.points).take(n) {
                *slot = (
                    fx_along(rect.x, rect.width, gx),
                    fx_along(rect.y, rect.height, gy),
                );
            }
            let paint = match shape.ink {
                Ink::Fore => fore,
                Ink::Back => back,
            };
            self.fill_polygon_fx(&points[..n], paint.into());
        }
    }
}

/// The pen widgets draw with: a `&mut dyn Painter` wearing the ergonomics the
/// trait cannot have.
///
/// [`Painter`] must be object-safe, so its methods take a premultiplied
/// [`Paint`] and it cannot hand out a re-borrow of itself for a tighter clip.
/// Those are exactly the two things widget code wants back. `Pen` restores them
/// as *inherent* methods, which need no import and never compete with the
/// trait's for resolution -- so a widget body written against a rasteriser reads
/// identically written against a pen, and a backend still only implements the
/// small trait.
pub struct Pen<'a> {
    painter: &'a mut dyn Painter,
    /// `Some` in a pen produced by [`Pen::with_clip`], which restores the clip
    /// it narrowed when dropped.
    token: Option<ClipToken>,
}

impl<'a> Pen<'a> {
    /// Borrows a painter to draw through.
    #[inline]
    pub fn new(painter: &'a mut dyn Painter) -> Self {
        Self {
            painter,
            token: None,
        }
    }

    /// The painter underneath, for code that wants the raw contract.
    #[inline]
    pub fn painter(&mut self) -> &mut dyn Painter {
        self.painter
    }

    /// A pen on the same target with a tighter clip.
    ///
    /// The clip is restored when the returned pen is dropped, and the borrow
    /// stops the parent drawing until then: a child cannot escape the region it
    /// was given, exactly as with a canvas's own `with_clip`.
    pub fn with_clip(&mut self, rect: Rect) -> Pen<'_> {
        let token = self.painter.push_clip(rect);
        Pen {
            painter: self.painter,
            token: Some(token),
        }
    }

    /// Full extent of the target.
    #[inline]
    pub fn size(&self) -> Size {
        self.painter.size()
    }

    /// Word layout of the target.
    #[inline]
    pub fn format(&self) -> PixelFormat {
        self.painter.format()
    }

    /// The region operations are currently restricted to.
    #[inline]
    pub fn clip(&self) -> Rect {
        self.painter.clip()
    }

    /// Returns `true` if the clip admits no pixels, so drawing can be skipped.
    #[inline]
    pub fn is_clipped_out(&self) -> bool {
        self.painter.is_clipped_out()
    }

    /// The clipped, visible part of `rect`.
    #[inline]
    pub fn visible(&self, rect: Rect) -> Option<Rect> {
        self.painter.visible(rect)
    }

    /// Fills the entire clip with an opaque colour.
    #[inline]
    pub fn clear(&mut self, color: Color) {
        self.painter.clear(color);
    }

    /// Fills `rect`, compositing with the destination if the paint has alpha.
    #[inline]
    pub fn fill_rect(&mut self, rect: Rect, color: impl Into<Paint>) {
        self.painter.fill_rect(rect, color.into());
    }

    /// Fills every rectangle. Overlapping rectangles composite twice.
    #[inline]
    pub fn fill_rects(&mut self, rects: &[Rect], color: impl Into<Paint>) {
        self.painter.fill_rects(rects, color.into());
    }

    /// Draws a border of `thickness` pixels *inside* `rect`.
    #[inline]
    pub fn stroke_rect(&mut self, rect: Rect, thickness: i32, color: impl Into<Paint>) {
        self.painter.stroke_rect(rect, thickness, color.into());
    }

    /// Fills a rectangle with rounded corners.
    #[inline]
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, color: impl Into<Paint>) {
        self.painter.fill_rounded_rect(rect, radius, color.into());
    }

    /// Strokes a rounded rectangle inside its bounds.
    #[inline]
    pub fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radius: i32,
        thickness: i32,
        color: impl Into<Paint>,
    ) {
        self.painter
            .stroke_rounded_rect(rect, radius, thickness, color.into());
    }

    /// Fills a circle.
    #[inline]
    pub fn fill_circle(&mut self, centre: Point, radius: i32, color: impl Into<Paint>) {
        self.painter.fill_circle(centre, radius, color.into());
    }

    /// Strokes a circle inside its bounds.
    #[inline]
    pub fn stroke_circle(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        color: impl Into<Paint>,
    ) {
        self.painter
            .stroke_circle(centre, radius, thickness, color.into());
    }

    /// Strokes part of a circle, from `start` through `sweep` binary turns.
    #[inline]
    pub fn stroke_arc(
        &mut self,
        centre: Point,
        radius: i32,
        thickness: i32,
        start: i32,
        sweep: i32,
        color: impl Into<Paint>,
    ) {
        self.painter
            .stroke_arc(centre, radius, thickness, start, sweep, color.into());
    }

    /// Draws a one-pixel line.
    #[inline]
    pub fn draw_line(&mut self, a: Point, b: Point, color: impl Into<Paint>) {
        self.painter.draw_line(a, b, color.into());
    }

    /// Fills a star with `points` points.
    #[inline]
    pub fn fill_star(
        &mut self,
        centre: Point,
        outer_radius: i32,
        inner_radius: i32,
        points: u32,
        rotation: i32,
        color: impl Into<Paint>,
    ) {
        self.painter.fill_star(
            centre,
            outer_radius,
            inner_radius,
            points,
            rotation,
            color.into(),
        );
    }

    /// Fills a polygon whose vertices are in the rasteriser's 8.8 fixed point.
    #[inline]
    pub fn fill_polygon_fx(&mut self, points: &[(i32, i32)], color: impl Into<Paint>) {
        self.painter.fill_polygon_fx(points, color.into());
    }

    /// Draws an icon scaled to `rect`, in two inks.
    #[inline]
    pub fn draw_icon(&mut self, icon: &Icon, rect: Rect, fore: Color, back: Color) {
        self.painter.draw_icon(icon, rect, fore, back);
    }

    /// Composites an 8-bit coverage mask. How glyphs arrive.
    #[inline]
    pub fn blit_mask(&mut self, at: Point, mask: &Mask<'_>, color: impl Into<Paint>) {
        self.painter.blit_mask(at, mask, color.into());
    }

    /// Composites the glyph at `rect` of an atlas `page`.
    #[inline]
    pub fn blit_glyph(
        &mut self,
        at: Point,
        page: &AtlasPage<'_>,
        rect: Rect,
        color: impl Into<Paint>,
    ) {
        self.painter.blit_glyph(at, page, rect, color.into());
    }

    /// Copies a premultiplied source over the target at `at`.
    #[inline]
    pub fn blit(&mut self, src: &PixelView<'_>, at: Point) {
        self.painter.blit(src, at);
    }

    /// Copies a premultiplied source, scaled to `dest`.
    #[inline]
    pub fn blit_scaled(&mut self, src: &PixelView<'_>, dest: Rect) {
        self.painter.blit_scaled(src, dest);
    }

    /// Copies a premultiplied source, scaled to `dest` and masked to a rounded
    /// `shape`.
    #[inline]
    pub fn blit_rounded(&mut self, src: &PixelView<'_>, dest: Rect, shape: Rect, radius: i32) {
        self.painter.blit_rounded(src, dest, shape, radius);
    }

    /// Draws an image, scaled to `dest`.
    #[inline]
    pub fn blit_image(&mut self, src: &ImageRef<'_>, dest: Rect) {
        self.painter.blit_image(src, dest);
    }

    /// Draws an image, scaled to `dest` and masked to a rounded `shape`.
    #[inline]
    pub fn blit_image_rounded(&mut self, src: &ImageRef<'_>, dest: Rect, shape: Rect, radius: i32) {
        self.painter.blit_image_rounded(src, dest, shape, radius);
    }
}

impl Drop for Pen<'_> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            self.painter.pop_clip(token);
        }
    }
}

impl core::fmt::Debug for Pen<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pen")
            .field("size", &self.size())
            .field("clip", &self.clip())
            .finish_non_exhaustive()
    }
}

/// The part of the painting API that cannot be object-safe.
///
/// Blanket-implemented for every [`Painter`], `dyn Painter` included, so it
/// needs no thought at the call site beyond importing it.
pub trait PainterExt: Painter {
    /// A view of the same target with a tighter clip, restored on drop.
    ///
    /// This is how a parent hands a child a region to draw in without either
    /// being able to escape it: the returned guard borrows the painter, so the
    /// parent cannot draw until it is dropped, and the child cannot widen the
    /// clip because it has no token to widen it with.
    fn with_clip(&mut self, rect: Rect) -> Clipped<'_, Self> {
        let token = self.push_clip(rect);
        Clipped {
            painter: self,
            token,
        }
    }
}

impl<P: Painter + ?Sized> PainterExt for P {}

/// A painter with a narrowed clip, which it restores when dropped.
///
/// Derefs to the painter, so it is used exactly as one.
pub struct Clipped<'p, P: Painter + ?Sized> {
    painter: &'p mut P,
    token: ClipToken,
}

impl<P: Painter + ?Sized> Drop for Clipped<'_, P> {
    fn drop(&mut self) {
        self.painter.pop_clip(self.token);
    }
}

impl<P: Painter + ?Sized> core::ops::Deref for Clipped<'_, P> {
    type Target = P;
    #[inline]
    fn deref(&self) -> &P {
        self.painter
    }
}

impl<P: Painter + ?Sized> core::ops::DerefMut for Clipped<'_, P> {
    #[inline]
    fn deref_mut(&mut self) -> &mut P {
        self.painter
    }
}
