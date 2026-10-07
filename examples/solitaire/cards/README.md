# The cards' faces

Fifty-two pictures, `c01.png` to `s13.png`: the suit's first letter and the
rank, ace low.

They are Byron Knoll's Vector Playing Cards, which he released into the
public domain (<https://code.google.com/archive/p/vector-playing-cards/>),
taken as the SVGs in <https://github.com/hayeah/playing-cards-assets>
(`svg-cards/`), which carries them unchanged under the same terms. Nothing
of that repository's own code is here.

Each was drawn at 300 by 436 with `rsvg-convert -w 300 -h 436 -b white`,
on Alpine 3.24 with `ttf-liberation` and `ttf-dejavu` installed for the
corner letters, and cut to 96 colours with `pngquant --strip --speed 1 96`.
The example scales them to the size it draws at; `src/faces.rs` says how
and why.

# The cards' backs

Three pictures, `back-01.png`, `back-03.png` and `back-12.png`, in the order
`src/faces.rs` names them. The numbers have gaps because these are three of
a set of twelve made for another program, and are kept as they were named
there. Each was made on 2026-10-07 by an image model, FLUX.1 [schnell]
(Apache-2.0), run on a machine of our own with kvad 0.14.1:

```
kvad images make "PROMPT, edge to edge, full-bleed, no border, no frame,
no text, no letters, no numbers" --model black-forest-labs/FLUX.1-schnell
--size 832x1216 --seed SEED
```

then cut to a card's shape, scaled to 300 by 436 and cut to 128 colours.
Nobody else's picture went into any of them, and nothing restricts their
use. The last is in the manner of the home computers of the 1980s and shows
no maker's name or mark.

| | Name | Seed | What was asked for |
|---|---|---|---|
| 01 | Mist | 21 | Layered mountain ridges fading into mist, deep teal and midnight blue, a small crescent moon |
| 03 | Crimson | 7 | A classic playing card back, symmetrical filigree and scrollwork in crimson and white |
| 12 | 16-bit | 7 | 1987 demoscene art: a red and white checkered sphere over a magenta grid, copper bars behind |

They do not follow the theme: a card is a picture of a thing, and is the
same under a dark theme and a light one. The plain back painted where one
cannot be decoded is a single colour.
