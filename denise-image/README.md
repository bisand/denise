# denise-image

[![crates.io](https://img.shields.io/crates/v/denise-image?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-image)
[![docs.rs](https://img.shields.io/docsrs/denise-image?color=94E2D5&label=docs.rs)](https://docs.rs/denise-image)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

Image decoding for **[Denise]**, a direct-rendering UI toolkit in Rust for
embedded Linux and systems without a desktop environment.

Bytes in, premultiplied pixels out — the shape `Canvas::blit` and the `Image`
widget consume, with the multiply by alpha paid here, once, instead of at every
frame:

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let bytes: &[u8] = include_bytes!("../tests/fixtures/two-halves.jpg");
let picture = denise_image::decode(bytes)?; // sniffs the format itself
let (pixels, size) = picture.into_parts();  // ready for Image::new
# Ok(())
# }
```

## Formats, and what each one costs

Each decoder is a cargo feature, so a panel pays binary size only for the
formats it actually ships — measured on a minimal ARM64 release binary,
`lto = "thin"`:

| Format | Decoder | Feature | Adds |
|---|---|:---:|---:|
| BMP, uncompressed 24/32-bit | this crate, ~100 lines | always on | ~0 KB |
| GIF (first frame) | [`gif`](https://crates.io/crates/gif) | `gif` | 43 KB |
| PNG, incl. APNG first frame | [`png`](https://crates.io/crates/png) | `png` | 115 KB |
| JPEG | [`zune-jpeg`](https://crates.io/crates/zune-jpeg) | `jpeg` | 154 KB |

All three defaults together add about 330 KB. All pure Rust — the same
decoders the `image` crate uses internally, without the umbrella.

## What it refuses to do

No file I/O: the application reads bytes and hands them over, because a
decoder that opens paths is unusable over the FFI and wrong in an embedded
toolkit. No scaling: the rasteriser does that at draw time. And nothing
decodes past 32 megapixels, so a file header that lies about its dimensions
cannot make a kiosk allocate a third of its RAM.

Animated GIFs decode to their first frame, deliberately — playback is an
animation-clock question for a later issue, and the streaming decoder
underneath does not foreclose it.

## Where this sits

Depends on [`denise`](https://crates.io/crates/denise) and
[`denise-render`](https://crates.io/crates/denise-render) plus the decoders
chosen by feature. Nothing in the toolkit depends on *it*: bringing your own
pixels remains the zero-cost tier, and this crate is one way of bringing them.

## Status

**M6.** Part of [Denise][Denise] — see the [repository README][Denise] for the
whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
