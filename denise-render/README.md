# denise-render

[![crates.io](https://img.shields.io/crates/v/denise-render?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-render)
[![docs.rs](https://img.shields.io/docsrs/denise-render?color=94E2D5&label=docs.rs)](https://docs.rs/denise-render)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The software rasteriser for **[Denise]**, a direct-rendering UI toolkit in Rust for
embedded Linux and systems without a desktop environment.

Rectangles, rounded rectangles, lines, rectangular clipping and source-over alpha
blending, straight into a [`denise::Frame`]. No GPU, no path builder, no allocator:
this crate needs neither `std` nor `alloc`, and every operation writes through a
borrowed slice the caller already owns.

```rust
use denise::{Color, Frame, Point, Rect};
use denise_render::Canvas;

# fn draw(frame: &mut Frame<'_>, damage: &[Rect]) {
let mut canvas = Canvas::new(frame);
for region in damage {
    // Everything inside this borrow is confined to `region`.
    let mut c = canvas.with_clip(*region);
    c.clear(Color::from_rgb888(0x1E1E2E));
    c.fill_rounded_rect(Rect::new(40, 40, 200, 64), 12, Color::rgba(255, 255, 255, 32));
    c.draw_line(Point::new(40, 120), Point::new(240, 121), Color::WHITE);
}
# }
```

## No floating point

Integer throughout, **including anti-aliasing coverage**. That avoids a `libm`
dependency on `no_std` targets and keeps output bit-identical between x86 and ARM
— so a reference test means the same thing on a developer's laptop and on the Pi.

## No `unsafe`

`#![forbid(unsafe_code)]`, deliberately. Bounds checks are hoisted by working
through row slices rather than indexing pixel by pixel. If the benches ever show
that costing real time, that is the evidence needed to justify unchecked access —
not before.

## The built-in font

An 8×8 bitmap face covering Latin plus `æøå`, compiled in at no cost, scaled by
whole numbers. It is what makes a panel with no font files still able to show a
number. Anything more — real TrueType, proportional metrics, wrapping — is
[`denise-text`](https://crates.io/crates/denise-text), which is a separate crate
precisely so this one stays a rasteriser.

## Features

| Feature | Default | What it does |
|---|:---:|---|
| `std` | ✅ | Passes through to `denise/std`. Off gives `no_std`, with no `alloc` either. |

## Where this sits

Depends only on [`denise`](https://crates.io/crates/denise). Used by
[`denise-text`](https://crates.io/crates/denise-text) and
[`denise-ui`](https://crates.io/crates/denise-ui). An application that draws its
own scene can use this directly and link no widget code at all.

## Status

**M5 complete, M6 in progress.** Benchmarked with criterion in CI; the numbers and
the reasoning are in [docs/design.md]. Part of [Denise][Denise] — see the
[repository README][Denise] for the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
[docs/design.md]: https://github.com/bisand/denise/blob/main/docs/design.md
[`denise::Frame`]: https://docs.rs/denise/latest/denise/struct.Frame.html
