//! A picture in a rectangle.

use alloc::vec::Vec;

use denise::{Pen, PixelView};
use denise::{Rect, Size};

use crate::widget::{PaintCtx, Widget};
use crate::widgets::describe::{
    Describe, DynDescribe, FITS, Group, Mismatch, Property, PropertyKind, Value,
};

/// How a picture whose size disagrees with its box gets reconciled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Fit {
    /// Stretch to the box, ignoring aspect ratio.
    Fill,
    /// Largest size that fits entirely inside the box, aspect kept, centred.
    /// Never crops and never distorts, which is why it is the default.
    #[default]
    Contain,
    /// Smallest size that covers the whole box, aspect kept, centred. The
    /// overflow on one axis is cropped by the box.
    Cover,
    /// No scaling at all: the picture at its own size, centred, cropped if the
    /// box is smaller. The right mode for pre-sized assets, QR codes and pixel
    /// art — and the cheapest, since nothing is resampled.
    Center,
}

/// The pixels an [`Image`] draws: decoded at runtime and owned, or compiled in
/// and borrowed forever.
#[derive(Clone, Debug)]
enum Pixels {
    Owned(Vec<u32>),
    Static(&'static [u32]),
}

impl Pixels {
    fn as_slice(&self) -> &[u32] {
        match self {
            Pixels::Owned(v) => v,
            Pixels::Static(s) => s,
        }
    }
}

/// A picture, drawn with a [`Fit`] inside its rectangle.
///
/// Not interactive and not focusable, like [`Label`](crate::widgets::Label):
/// an image inside a button never intercepts the click.
///
/// # The pixels are premultiplied `0xAARRGGBB`
///
/// The same contract as [`Pen::blit`]: colour channels already multiplied
/// by alpha, converted once at load time —
/// [`denise_render::blend::premultiply`] does it in place. Fully opaque pixels
/// are identical in both conventions, so an image with no transparency needs
/// no conversion. Where the pixels come from is deliberately not this
/// widget's business: the application does I/O and decoding and hands the
/// result over — the `denise-image` crate decodes PNG, JPEG, GIF and BMP
/// into exactly this shape.
///
/// # Sizing is honest about what it costs
///
/// Scaling is nearest-neighbour — exact for the flat-colour assets an embedded
/// panel mostly shows, visibly blocky for a photo at a non-integer factor. An
/// asset pre-sized to its box (`Fit::Center`) is blitted without resampling at
/// all, which is both the fastest path and the best-looking one.
#[derive(Clone, Debug)]
pub struct Image {
    pixels: Pixels,
    size: Size,
    fit: Fit,
    radius: i32,
}

impl Image {
    /// An image over decoded pixels, drawn with [`Fit::Contain`].
    ///
    /// `pixels` is rows of premultiplied `0xAARRGGBB`, tightly packed at
    /// `size.width` words per row. A buffer smaller than `size` requires draws
    /// nothing rather than panicking — the tree keeps running with a blank
    /// rectangle where the picture would be.
    pub fn new(pixels: Vec<u32>, size: Size) -> Self {
        Self {
            pixels: Pixels::Owned(pixels),
            size,
            fit: Fit::Contain,
            radius: 0,
        }
    }

    /// An image over pixels compiled into the binary.
    ///
    /// For the logo case: `include_bytes!` plus a build-time conversion gives a
    /// panel its branding with no file I/O and no allocation.
    pub fn from_static(pixels: &'static [u32], size: Size) -> Self {
        Self {
            pixels: Pixels::Static(pixels),
            size,
            fit: Fit::Contain,
            radius: 0,
        }
    }

    /// Sets how the picture is reconciled with its box.
    pub fn with_fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
        self
    }

    /// Rounds the corners of the painted area.
    ///
    /// The radius applies to what is visible: the picture's own rectangle under
    /// [`Fit::Contain`], the box itself under [`Fit::Cover`]. A radius of half
    /// the shorter side is a circle, which is the avatar crop.
    pub fn with_corner_radius(mut self, radius: i32) -> Self {
        self.radius = radius.max(0);
        self
    }

    /// The picture's own size, in pixels.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Replaces the pixels.
    ///
    /// Reach this through [`Ui::widget_mut`](crate::Ui::widget_mut), which
    /// marks the node dirty on the way in — this is how a periodically
    /// refreshed picture (a camera still, a chart) updates in place.
    pub fn set_pixels(&mut self, pixels: Vec<u32>, size: Size) {
        self.pixels = Pixels::Owned(pixels);
        self.size = size;
    }
}

/// The rectangle `image` maps onto when drawn into `bounds` with `fit`.
///
/// Integer throughout; the aspect-preserving modes round to the nearest pixel
/// and never round a visible dimension to zero.
fn fit_rect(bounds: Rect, image: Size, fit: Fit) -> Rect {
    let (bw, bh) = (bounds.width as i64, bounds.height as i64);
    let (iw, ih) = (image.width as i64, image.height as i64);
    let centred = |w: i64, h: i64| {
        Rect::new(
            bounds.x + ((bw - w) / 2) as i32,
            bounds.y + ((bh - h) / 2) as i32,
            w as i32,
            h as i32,
        )
    };
    match fit {
        Fit::Fill => bounds,
        Fit::Center => centred(iw, ih),
        Fit::Contain | Fit::Cover => {
            // Width-limited when the image is proportionally wider than the
            // box; Cover picks the opposite axis to Contain.
            let width_limited = iw * bh >= ih * bw;
            if width_limited == matches!(fit, Fit::Contain) {
                let h = ((ih * bw + iw / 2) / iw.max(1)).max(1);
                centred(bw, h)
            } else {
                let w = ((iw * bh + ih / 2) / ih.max(1)).max(1);
                centred(w, bh)
            }
        }
    }
}

impl Image {
    /// Whether there are enough pixels to draw the size this image claims.
    ///
    /// `false` for a buffer shorter than `width × height`, which is what
    /// [`Avatar`](super::Avatar) asks before deciding whether it can show a
    /// picture or has to fall back to initials.
    pub fn is_drawable(&self) -> bool {
        PixelView::new(self.pixels.as_slice(), self.size, self.size.width).is_some()
    }

    /// Draws into `bounds` with `radius`, ignoring the image's own.
    ///
    /// The whole of painting, so [`Widget::paint`] is one call to it. Taking
    /// the rectangle and radius as arguments is what lets `Avatar` reuse the
    /// fit and mask rules for a shape it decides at paint time, without
    /// cloning a pixel buffer to change one number.
    pub(crate) fn paint_at(&self, bounds: Rect, radius: i32, canvas: &mut Pen<'_>) {
        let Some(view) = PixelView::new(self.pixels.as_slice(), self.size, self.size.width) else {
            return;
        };
        let dest = fit_rect(bounds, self.size, self.fit);
        if dest.is_empty() {
            return;
        }
        if radius > 0 {
            // The mask is what is visible: the picture where it fits inside
            // the box, the box where the picture overflows it.
            let Some(shape) = dest.intersect(&bounds) else {
                return;
            };
            canvas.blit_rounded(&view, dest, shape, radius);
        } else {
            canvas.blit_scaled(&view, dest);
        }
    }
}

impl<M: 'static> Widget<M> for Image {
    fn describe(&self) -> Option<&dyn DynDescribe> {
        Some(self)
    }

    fn describe_mut(&mut self) -> Option<&mut dyn DynDescribe> {
        Some(self)
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        self.paint_at(ctx.bounds, self.radius, canvas);
    }
}

impl Describe for Image {
    const KIND: &'static str = "image";
    const DOC: &'static str = "A picture, fitted to a rectangle.";
    const GROUP: Group = Group::Media;
    const ICON: &'static denise::icon::Icon = &super::icons::IMAGE;

    const PROPERTIES: &'static [Property] = &[
        Property::new(
            "src",
            PropertyKind::Asset,
            "The picture, as a path relative to the form file.",
        ),
        Property::new(
            "fit",
            PropertyKind::Enum(FITS),
            "How a picture whose size disagrees with its box is reconciled.",
        ),
        Property::new(
            "radius",
            PropertyKind::Int { min: 0, max: 256 },
            "Corner radius in pixels — a picture is cropped to a shape rather than themed into one.",
        ).in_pixels(),
    ];

    fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            // The widget holds decoded pixels and never saw the path they came
            // from, so there is nothing to report. See the `describe` module.
            "src" => return None,
            "fit" => Value::fit(self.fit),
            "radius" => Value::Int(self.radius),
            _ => return None,
        })
    }

    fn apply(&mut self, name: &str, value: Value) -> Result<(), Mismatch> {
        match name {
            "src" => return Err(Mismatch::Supplied),
            "fit" => self.fit = value.as_fit()?,
            // Clamped exactly as `with_corner_radius` clamps it: a negative
            // radius would reach `blit_rounded` as a shape it cannot mask.
            "radius" => self.radius = value.as_int()?.max(0),
            _ => return Err(Mismatch::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: Rect = Rect::new(10, 10, 100, 50);

    #[test]
    fn contain_touches_the_box_on_exactly_the_limited_axis() {
        // A tall image in a wide box: height limited, centred horizontally.
        let r = fit_rect(BOX, Size::new(50, 100), Fit::Contain);
        assert_eq!(r.height, 50);
        assert_eq!(r.width, 25);
        assert_eq!(r.y, 10);
        assert_eq!(r.x, 10 + (100 - 25) / 2);

        // A wide image in the same box: width limited.
        let r = fit_rect(BOX, Size::new(400, 100), Fit::Contain);
        assert_eq!(r.width, 100);
        assert_eq!(r.height, 25);
    }

    #[test]
    fn cover_overflows_the_box_on_exactly_the_other_axis() {
        let r = fit_rect(BOX, Size::new(50, 100), Fit::Cover);
        assert_eq!(r.width, 100, "must span the full width");
        assert_eq!(r.height, 200, "and overflow the height");
        assert_eq!(r.y, 10 + (50 - 200) / 2, "overflow split evenly");
    }

    #[test]
    fn contain_and_cover_preserve_aspect_within_rounding() {
        // Sizes chosen not to trip the never-to-zero sliver clamp, which
        // deliberately trades aspect for visibility and has its own test.
        for size in [Size::new(37, 91), Size::new(640, 480), Size::new(30, 80)] {
            for fit in [Fit::Contain, Fit::Cover] {
                let r = fit_rect(BOX, size, fit);
                // Cross-multiplied aspect check, allowing one pixel of rounding
                // on the derived axis.
                let derived = (size.height as i64 * r.width as i64 + size.width as i64 / 2)
                    / size.width as i64;
                assert!(
                    (derived - r.height as i64).abs() <= 1,
                    "{size:?} {fit:?} gave {r:?}"
                );
            }
        }
    }

    #[test]
    fn center_never_scales() {
        let r = fit_rect(BOX, Size::new(30, 20), Fit::Center);
        assert_eq!((r.width, r.height), (30, 20));
        assert_eq!((r.x, r.y), (10 + 35, 10 + 15));

        // Larger than the box: still unscaled, overflow split evenly.
        let r = fit_rect(BOX, Size::new(300, 200), Fit::Center);
        assert_eq!((r.width, r.height), (300, 200));
        assert_eq!(r.x, 10 + (100 - 300) / 2);
    }

    #[test]
    fn fill_is_the_box() {
        assert_eq!(fit_rect(BOX, Size::new(1, 1000), Fit::Fill), BOX);
    }

    #[test]
    fn an_absurdly_thin_image_still_shows_a_sliver() {
        let r = fit_rect(BOX, Size::new(1, 10_000), Fit::Contain);
        assert_eq!(r.width, 1, "rounded to a sliver, not to nothing");
        assert_eq!(r.height, 50);
    }
}
