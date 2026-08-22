# browser

A small web browser, every visible pixel of it a Denise widget. The URL bar
is `TextInput`, the buttons are `Button` (and one arc the rasteriser draws
itself), page text is a custom widget over the shared text engine, `<img>`
goes through `denise-image`, and a form's controls are the toolkit's own
`TextInput`, `Checkbox`, `RadioGroup` and `Select` — reading a page's
`<select>` opens the same popup the gallery's does.

```bash
cargo run -p browser                                    # the welcome page
cargo run -p browser -- https://news.ycombinator.com
cargo run -p browser -- examples/browser/fixtures/form.html
cargo run -p browser -- --snapshot shot.ppm https://example.com
cargo run -p browser -- --keyboard                      # with the keyboard up
cargo run -p browser --no-default-features --features kiosk    # the display itself
```

Alt with an arrow is history. Escape puts the on-screen keyboard away, and
then quits when there is nothing left to dismiss. On a kiosk, F12 writes a
screenshot to `/tmp`.

## What it is for

The toolkit's composability, proved on a workload nobody designed it for.
Denise deliberately has no layout engine, no rich text, and no waker — and a
browser needs all three. Each gap closed with public API, from the outside:

- **Layout** is this example's own: block boxes stacked down the page, and a
  run-aware line breaker over `TextEngine::measure_line`, committing each
  line at the tallest ascent on it — which is what `draw_line` taking a
  *baseline* origin was waiting for. The tree gets explicit rectangles, the
  way it always does; `set_scrollable` on one node is the whole viewport.
- **Rich text** is the `TextFlow` widget: precomputed styled fragments,
  painted with zero measuring, links hit-tested against rectangles recorded
  at layout time. Bold and italic are separately loaded faces, because a
  `TextStyle` is honestly just a font and a size.
- **The network** is a thread and two channels. Neither backend can be woken
  by another thread, so while a fetch is in flight the loop polls at 40 ms —
  and when nothing is in flight, nothing polls, which keeps the toolkit's
  idle-costs-nothing rule intact with a network attached.

## Typing on a panel

A Pi wired to a display has no keyboard, and until it had one this demo was
a browser you could look at and not use: the URL bar is the only way to
anywhere, and the welcome page's search box is the first thing on screen.

Tapping either brings `denise-keyboard` up from the bottom — no wiring, the
keyboard follows focus and asks the tree whether what took the caret is a
`TextInput`. It types what a keyboard plugged into the machine would type,
in the layout the machine is configured for: verified on an Alpine Pi whose
`/etc/conf.d/loadkmap` says `no`, where the home row comes up `ø æ` and the
layout key says so.

Two things this example has to do itself, because they are policy rather
than mechanism:

- **The page grows by the keyboard's height while it is up.** A page ends
  where its last line ends, so a field in that final screenful has nothing
  below it to scroll into and cannot be brought clear of the keys. Adding
  exactly what the keyboard covers gives it somewhere to go; the room leaves
  with the keyboard. Every phone browser does this, and the toolkit cannot,
  because only the application knows its content may grow.
- **Escape.** A shelf pushes no scene — that is what lets the field keep its
  caret — so nothing in the tree claims the key on the keyboard's behalf.

## What it does

HTML through html5ever into an arena DOM; a hardcoded user-agent style plus
a working CSS subset (colours, font size and voice, margins, padding,
`display`, underlines, alignment) cascaded from `<style>` blocks, linked
stylesheets, `style=""` attributes, and the pre-CSS `bgcolor` that still
holds up Hacker News; GET and POST forms serialised
`application/x-www-form-urlencoded` from the live widgets at the moment of
the click; redirects, an in-memory cookie jar, gzip, and charset
transcoding via ureq with rustls.

TLS is the `tls` feature, on by default. It is separable because rustls's
`ring` contains C, and the workspace's toolchain-free cross story ships a
linker, not a compiler — so a kiosk cross-build has to find one somewhere.
It does not need a cross toolchain, though: ring compiles its C with
`-nostdlibinc` and reads no libc headers, so any clang aimed at the target
plus rustup's `llvm-ar` produces the same one static `aarch64-musl` binary,
`https:` included. `scripts/deploy-pi.sh` sets that up by itself, and
`docs/raspberry-pi.md` writes it out for building by hand.

`file:` URLs work, and so does a bare path — typed into the URL bar, one is
read as a path whether or not the file is there, so a mistyped filename says
"no such file" instead of going looking for a host by that name. Anything that
is not plainly a path and has no scheme gets `https://` guessed at it. A
`file:` form submits into a preview page showing exactly what would have been
sent — the fixtures in `fixtures/` exercise
the whole pipeline with no network at all, and `--snapshot` renders any
page headlessly into a PPM for eyes or tests.

## What it deliberately does not do

**No JavaScript**, ever — that is the line between an example and a decade.
Server-rendered pages (Wikipedia, Hacker News, DuckDuckGo Lite, docs, the
old web) read well; an SPA renders the empty shell it serves. No
positioning, no flex or grid — content flows in document order: *readable
is the promise; fidelity is not.* Two exceptions earned their keep: tables
get real columns, widths negotiated from `width` attributes, spacer images
and short cells, up to eight across; and floats stand aside while the text
runs past them, `clear` honoured, the old `align` attribute included.
Media queries are answered for `screen` and the real viewport width;
absolutely positioned overlays are dropped the way reader modes drop them;
`#fragment` links scroll instead of refetching; Back and Forward come from
an in-memory page cache, instantly; downscaled images are box-filtered
smooth before the blit. No mid-word
breaking, no bidi or shaping. Upscaled images stay nearest-neighbour
(downscales are filtered). No WebP or
SVG. No multiline `textarea` editing, no file inputs, no tabs, no cache,
no selection, no find-in-page. Cookies last as long as the process.

## Where this sits

An example, not a crate: `publish = false`, like the gallery it borrows its
two backends from — a window on any desktop, or DRM/KMS and evdev on a bare
Linux panel, chosen at compile time because no runtime probe can tell a
kiosk Pi from a desktop Pi.
