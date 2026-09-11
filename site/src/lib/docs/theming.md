---
title: Theming
description: Widgets name roles rather than colours, a theme is derived from nine seeds with contrast checked by construction, and scaling for density is one multiply in one place.
---

A widget in DeniseUI never names a colour. It names a **role** — `Primary`, `Base100`, `Error` — and the theme says what that role is today. Every surface role has a **content** partner, so readability is a property of the pair rather than of the widget, and swapping a theme cannot produce unreadable text. The vocabulary is borrowed from [daisyUI](https://daisyui.com); the arithmetic is Denise's own.

## Roles

There are twenty, defined by `denise::Role`:

| Surfaces | Content |
|---|---|
| `Base100`, `Base200`, `Base300` | `BaseContent`, shared by all three |
| `Primary`, `Secondary`, `Accent`, `Neutral` | `PrimaryContent`, `SecondaryContent`, `AccentContent`, `NeutralContent` |
| `Info`, `Success`, `Warning`, `Error` | `InfoContent`, `SuccessContent`, `WarningContent`, `ErrorContent` |

`Base100` is the page and panel background; `Base200` and `Base300` are recessed steps of the same surface, for wells, stripes, borders and dividers.

```rust
use denise::theme::{Radius, Role, Theme};

let theme = Theme::DARK;
let (background, foreground) = theme.pair(Role::Primary);
let corner = theme.radius(Radius::Field);
```

`Theme::pair` returns a surface and the foreground guaranteed to contrast with it. One rule is worth stating because it has been got wrong more than once inside the toolkit itself: **a role is only guaranteed to contrast with its own content**, not with whatever surface a widget happens to sit on. Take both colours from one pair.

A `Theme` is plain data with no global state. It travels explicitly, so two displays on one device can run different themes, and nothing in the render path reaches for shared mutable state. Changing it at run time is `Ui::set_theme`; every widget follows, because every widget asked for roles.

## Nine seeds

A theme is built from nine seed colours: base, primary, secondary, accent, neutral, info, success, warning and error. The rest are derived.

```rust
use denise::{Color, theme::{ColorScheme, Theme}};

const PANEL: Theme = Theme::from_seeds(
    "panel",
    ColorScheme::Dark,
    Color::from_rgb888(0x1E1E2E), // base
    Color::from_rgb888(0x89B4FA), // primary
    Color::from_rgb888(0xF5C2E7), // secondary
    Color::from_rgb888(0x94E2D5), // accent
    Color::from_rgb888(0x585B70), // neutral
    Color::from_rgb888(0x89DCEB), // info
    Color::from_rgb888(0xA6E3A1), // success
    Color::from_rgb888(0xF9E2AF), // warning
    Color::from_rgb888(0xF38BA8), // error
);
```

Those are the seeds of the built-in dark theme. `Theme::from_seeds` is a `const fn`, so the built-in themes cost nothing at run time and cannot drift out of step with the derivation rules the way a hand-written table would.

### What is derived, and how

- **The recessed surfaces.** `Base200` and `Base300` are the base mixed 18 and 36 steps towards black — or towards white, when the base is so dark there is no darker left. That is the whole of the dark derivation: an OLED-black theme's recessed surfaces step lighter, and everything else's step darker, which is what reads as depth.
- **Every content colour.** Each is found by walking from the surface towards black or white until the mix clears **WCAG AA, 4.5:1**. Stopping at the first mix that clears means a derived theme keeps its hue rather than collapsing to black on white. `BaseContent` is derived against all three base surfaces, not only the main one.

The contrast arithmetic is WCAG relative luminance in integers, so it gives the same answer on x86 and ARM. The thresholds are public as ratios times a hundred: `AA` is 450, `AA_LARGE` is 300 and `AAA` is 700.

### Contrast by construction, and by check

A theme derived from seeds meets AA by construction. `Theme::from_seeds_at` takes a different target for a theme that must be legible in glare. Once you start overriding roles by hand with `Theme::with_color` — which does **not** re-derive the content partner — check the result:

```rust
use denise::theme::{AA, Theme};

let theme = Theme::DARK.with_color(Role::Primary, Color::from_rgb888(0x3050F0));
if let Err(failure) = theme.validate(AA) {
    // failure.surface, failure.content, failure.ratio_x100, failure.required
}
```

`Theme::validate` reports the worst offender. It has earned its place: it caught that pure magenta and `#FF5555` both top out near 6.7:1 against black and cannot reach AAA, which is why the high-contrast theme uses lightened variants of both. In `denise-ui`, every widget's surface and foreground pair is contrast-checked by a test in all three built-in themes.

## The built-in themes

Three ship, listed in `Theme::BUILT_IN`:

| Theme | Built from |
|---|---|
| `Theme::LIGHT` | Catppuccin Latte, near enough |
| `Theme::DARK` | Catppuccin Mocha, near enough |
| `Theme::HIGH_CONTRAST` | Saturated primaries on black, derived at AAA, with touch metrics |

Three rather than thirty-five, because on a device that boots from flash an unused theme is bytes somebody paid for. A `.dform` file chooses one by name with `theme=dark`, `light` or `high-contrast`.

## Geometry: radius, metrics and depth

Corners come from three tokens by widget class, not one constant per widget:

| `Radius` | Used by |
|---|---|
| `Selector` | checkboxes, radios, toggles, badges |
| `Field` | buttons, inputs, selects, tabs |
| `Box` | cards, dialogs, alerts, panels |

A theme's `Metrics` holds those radii plus the height of a field, the size of a selector and the border width. Two sets ship: `Metrics::DEFAULT` for a pointer (36-pixel fields) and `Metrics::TOUCH` for a finger, possibly gloved (48-pixel fields). `Theme::with_metrics` swaps them.

`depth` is a number, not a shadow. A widget honours it by lightening its top edge and darkening its bottom edge, which stays inside its own bounds. A real blur would spill outside them and inflate every damage rectangle by its radius. `Theme::with_depth` sets it.

## The gallery's theme editor

[`examples/gallery`](https://github.com/bisand/denise/tree/main/examples/gallery) is every widget live, with an editor beside them: the nine seeds, a light/dark switch, touch metrics, roundness, depth and a Surprise button. The editor does not talk to the widgets. It rebuilds a theme with `Theme::from_seeds` and hands it to `Ui::set_theme`, and that is the entire mechanism. A badge in the corner shows the worst surface/content contrast in whatever you have built.

```bash
cargo run -p gallery
```

## Scale and DPI

**The application scales, once, at construction.** It already knows the scale factor and already computes every rectangle, so it is the one place that can multiply everything consistently. There is no logical coordinate space inside the tree and no per-widget conversion. The pattern is three calls, and `examples/hello` shows all of them:

```rust
let s = |r: Rect| r.scaled(scale);
let px = |v: f32| (v * scale + 0.5) as u16;

let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale));

ui.add(
    card,
    Label::new("Hello, Denise").with_size(px(22.0)),
    s(Rect::new(20, 18, 388, 28)),
);
```

- `Theme::scaled` scales the metrics — radii, field heights, borders. Colours have no size.
- `Rect::scaled` scales **edges**, not width and height. Two rectangles that touch in the logical layout still touch at 1.5×, where rounding each width independently would open one-pixel seams. `Rect::scaled_by` does the same with a factor per axis.
- Text sizes are the application's numbers like any other. Widgets default to 16 px, so a scale-aware application names its sizes; whether the theme should carry a type scale is an open design question.

To see it without a display:

```bash
cargo run -p hello -- --snapshot out.ppm 2
```

Where the factor comes from depends on the backend. On the desktop, `denise_winit::run_with` builds the application once the window exists and hands the builder the surface size and the display's scale factor; `WindowConfig::size` is logical, so the same number is the same amount of desk on a Pi and on a 2× display. A C host calls `denise_ui_new_scaled`, which takes the factor in hundredths. A `.dform` file declares with `scaling=` whether it consents to being drawn at another size — see [Forms and the designer](../forms-and-designer/).

One edge is still unproven: `denise-win32` has never been hosted inside a real dialog while the DPI changes, which is what would prove the Windows end. See [Embedding](../embedding/).

## What was not borrowed from daisyUI

| | |
|---|---|
| OKLCH storage | Cube roots mean floats, floats mean `libm` on `no_std` and output that is no longer bit-identical across architectures. Colours are sRGB, derived with integers. |
| Noise textures | A per-pixel texture means no damaged region can be repainted without a seam, which turns every frame into a full repaint. |
| Thirty-five themes | Three. |
| Depth as a shadow | Kept as a number, for the damage reason above. |

API reference: [`denise::theme` on docs.rs](https://docs.rs/denise). Source: [`denise/src/theme/mod.rs`](https://github.com/bisand/denise/blob/main/denise/src/theme/mod.rs).
