# denise-text

[![crates.io](https://img.shields.io/crates/v/denise-text?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-text)
[![docs.rs](https://img.shields.io/docsrs/denise-text?color=94E2D5&label=docs.rs)](https://docs.rs/denise-text)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

Fonts, a bounded glyph cache, line layout and word wrapping for **[Denise]**, a
direct-rendering UI toolkit in Rust for embedded Linux and systems without a
desktop environment.

One `TextEngine` holds every font an application uses and one `GlyphAtlas` that
caches what has been rasterised. Measurement and drawing both go through it, so a
label measured during layout and drawn a moment later rasterises its glyphs
exactly once.

```rust
use denise::{Color, Point};
use denise_render::Canvas;
use denise_text::{TextEngine, TextStyle};

# fn demo(canvas: &mut Canvas<'_>) {
let mut text = TextEngine::new();
let style = TextStyle::built_in(16);

let extent = text.measure(style, "Kjærlighet på Øy");
text.draw(canvas, style, Point::new(20, 20), "Kjærlighet på Øy", Color::WHITE);

// And the lines a paragraph becomes at a given width, borrowing the input.
let lines: Vec<&str> = text.wrap(style, "en to tre fire fem seks", 80);
# let _ = (extent, lines);
# }
```

## Three tiers, and what each costs

Measured as the increase in a stripped, statically linked
`aarch64-unknown-linux-musl` binary:

| Tier | Feature | Cost | What it buys |
|---|---|---:|---|
| Built-in bitmap | *none* | 0 | Latin plus `æøå`, whole-number scales |
| TrueType | `truetype` | +145 KB | Real fonts, proportional metrics, anti-aliasing |
| Shaped | `shaping` | +3.1 MB | Ligatures, bidi, complex scripts, font fallback |

For scale: the whole of Denise, DRM, evdev and the widgets is about **840 KB**, so
the shaping tier is four times the rest of the toolkit put together. It is there
because some panels genuinely need it, and off by default because most do not — a
temperature readout and a Norwegian name do not need a shaper.

## Wrapping

Greedy, returning slices that borrow the input, and explicit `\n` always breaks. A
word wider than the line overflows on a line of its own rather than being cut
between characters: breaking mid-word means knowing where a grapheme ends, and half
of an `æ` is nothing.

## Where this sits

Depends on [`denise`](https://crates.io/crates/denise) and
[`denise-render`](https://crates.io/crates/denise-render).
[`denise-ui`](https://crates.io/crates/denise-ui) re-exports `TextStyle` and passes
the tier features through, so an application picks its text tier once.

`#![forbid(unsafe_code)]`.

## Status

**M5 complete, M6 in progress.** The glyph cache is benchmarked in CI. Part of
[Denise][Denise] — see the [repository README][Denise] and [docs/design.md] for
the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
[docs/design.md]: https://github.com/bisand/denise/blob/main/docs/design.md
