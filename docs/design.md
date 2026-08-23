# DeniseUI — design notes

How Denise is built and why, written milestone by milestone as it was built. This
is the long half of what used to be the README: the architecture, the reasoning,
the measurements, and the things that were tried and abandoned.

Start with the [README](../README.md) if you want to *use* the toolkit. Read this
if you want to know why it is shaped the way it is — or if you are about to change
something and would like to know what it was traded against.

---

## Architecture

A Cargo workspace: a platform-agnostic core, and thin backends behind two traits.

| Crate | Purpose | Status |
|---|---|---|
| `denise` | Geometry, colour, pixel buffer contract, input, damage tracking, theming | ✅ M0, M1.1 |
| `denise-render` | Software rasteriser, coverage blitting, the built-in bitmap font | ✅ M1, M3, M4 |
| `denise-text` | Glyph sources, a bounded glyph atlas, line layout, word wrapping | ✅ M4, M6 |
| `denise-ui` | Scene graph, scene stack, widgets, cursor sprite | ✅ M3 |
| `denise-image` | PNG, JPEG, GIF and BMP decoding into premultiplied pixels | ✅ M6 |
| `denise-video` | V4L2 M2M hardware decode onto a DRM plane, zero-copy | ◐ M7 |
| `denise-winit` | Desktop development and preview backend | ✅ M0 |
| `denise-drm` | Linux DRM/KMS backend — the primary target | ✅ M2 |
| `denise-fbdev` | Linux fbdev fallback | ✅ M2 |
| `denise-evdev` | Linux input, keyboard layouts, dead keys, console muting | ✅ M2, M4, M5 |
| `denise-ffi` | Stable C ABI, `cdylib`, hand-written header | ✅ M5 |
| `denise-macos` | Embeddable `NSView` over a CoreGraphics bitmap context | ✅ M5 |
| `denise-win32` | Windows child-`HWND` control over a DIB section | ✅ M5 |
| `denise-activex` | COM/ActiveX shim for legacy Windows hosts, scriptable over `IDispatch` | ✅ M5 |

Everything through M5 has run on real hardware. On Windows 11 ARM64, `denise-win32`
puts a window on screen where Tab reaches the control, AltGr composes `@` and the
dead keys produce `é` and `ö`; and `denise-activex` registers with `regsvr32`,
instantiates through `CoCreateInstance`, sites, activates in place and renders
inside a container that knows nothing about it — which then sets its properties by
name over `IDispatch`, sinks its `Change` and `Click` events, and assigns back to
it from inside its own event handlers.

`denise-ui` is a crate of its own rather than part of the core because widgets
need both the platform contract and the rasteriser, and the rasteriser already
depends on the contract — putting them together would be a dependency cycle. It
also means a signage application that draws its own scene links no arena, no tree
and no widget code at all.

### The two traits

Everything a backend has to provide:

```rust
pub trait Surface {
    fn size(&self) -> Size;
    fn scale_factor(&self) -> f32;
    fn format(&self) -> PixelFormat;
    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError>;
    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError>;
}

pub trait InputSource {
    fn poll(&mut self, out: &mut Vec<InputEvent>);
}
```

`Frame` carries the pixel slice, its **stride**, its format, and its **age**. Those
last two are the whole reason `acquire` exists rather than a bare
`buffer_mut() -> &mut [u32]`:

- **Stride is not width.** DRM framebuffers are pitch-aligned — 64 bytes on vc4,
  more on other ARM drivers — and fbdev has its own `line_length`. Code that
  assumes rows are contiguous works perfectly on a desktop and shears diagonally on
  the panel you actually shipped.
- **Buffers are stale.** With double buffering the buffer you are handed holds the
  frame *before* last. Repainting only this frame's damage leaves the older
  content visible in alternating frames. `BufferAge` is modelled on
  `EGL_EXT_buffer_age`, and `DamageTracker::resolve` widens this frame's damage to
  cover everything that buffer missed.

Both failure modes are covered by
[`denise-render/tests/damage_pipeline.rs`](../denise-render/tests/damage_pipeline.rs),
which runs an anti-aliased scene through 1-, 2-, 3- and 6-buffered swapchains with
a padded stride and asserts every presented frame is pixel-identical to a full
repaint.

### Rendering pipeline

Ported from CoreCanvas, and what `Ui::paint` does, in this order, inside each
damage rectangle:

1. Clear the back buffer — clean UI, no cursor.
2. Render the base scene.
3. Render modal scenes over a dimmed backdrop.
4. Composite the cursor sprite onto the clean buffer.
5. Present damaged rectangles only.

Every step happens inside the damage clip, which is what makes step 3 affordable:
a full-screen alpha fill costs 63% of a 60 Hz frame on a Pi 3, so a modal that
repaints its own blinking caret must not drag one along with it.

On DRM, step 4 uses the **hardware cursor plane** instead. vc4 has one, and the
display controller composites it during scanout, so moving the pointer is a single
ioctl — no repaint, no page flip, and the new position takes effect at the next
scanout of those lines rather than the next frame. `CursorPlane` in the core is the
seam; `Ui::show_cursor(false)` tells the tree to stop drawing its own, and the
software composite stays as the fallback for every backend without a plane.

### What damage actually buys

Worth being precise about, because it differs per backend:

| Backend | Effect of `present(damage)` |
|---|---|
| Win32 `BitBlt`, X11, Wayland | Real. Only the listed regions are uploaded. |
| DRM/KMS page flip | Little to none. A flip swaps whole buffers; `FB_DAMAGE_CLIPS` is atomic-only and widely ignored. |

On DRM the win is entirely upstream: not rasterising the untouched pixels in the
first place. That is where the CPU goes, so that is where the tracking pays.

## The component model

Widgets live in a generational arena; the tree stores ids, not references. Event
handling returns messages, not callbacks. No `Rc<RefCell<_>>` anywhere in the
path.

```rust
let mut ui: Ui<Msg> = Ui::new(surface.size(), theme::DARK);
let card = ui.add(ui.root(), Panel::default(), Rect::new(40, 40, 400, 260))?;
ui.add(card, Button::new("Lagre", Msg::Save), Rect::new(20, 180, 160, 46))?;

ui.handle(&events);
ui.render(&mut surface)?;              // draws nothing when nothing changed
for message in ui.drain_messages() { /* the application decides */ }
```

A stale `NodeId` resolves to `None` rather than to whoever was allocated next,
which is the entire reason for the generation in the key.

### The widget set, and what a widget has to earn

Twenty-five of them: `Panel`, `Label`, `Button`, `TextInput`, `Checkbox`,
`Toggle`, `RadioGroup`, `Progress`, `Slider`, `Divider`, `Badge`, `Alert`,
`Tabs`, `List`, `RadialProgress`, `Spinner`, `Select`, `Image`, `Rating`,
`Avatar`, `Table`, `Timeline`, `Carousel`, `Collapse`, `Video`. The first four are CoreCanvas 0.4
parity; the rest are being added one at a time against
[issue #6](https://github.com/bisand/denise/issues/6), which triages the DaisyUI
component list against what a toolkit with no layout engine can honestly support.

The bar is not "is it useful". It is **would several panels otherwise each get
this subtly wrong** — focus handling, keyboard semantics, hit areas, disabled
states. A widget that saves a caller three `fill_rect` calls has not earned a
place; one that stops three applications each inventing their own tab-stop rule
has.

`RadioGroup` is the clearest case. It is the *group*, not the button, which makes
it one node and therefore **one tab stop** — the thing an application assembling
radios out of separate widgets gets wrong every time — and it holds one index, so
"two chosen" is unrepresentable rather than merely avoided. `Slider` is the next
clearest: a drag has to keep the pointer after it leaves the widget, and the tree
clears `VisualState::PRESSED` at exactly that boundary because that is what makes
a *button's* drag-off cancel. Two widgets, opposite requirements, one signal.

Three rules hold across all of them:

- **A setter is silent.** `set_checked`, `set_value`, `set_selected` emit nothing.
  The message reports what a person did, and an application that assigned and got
  its own message back would either loop or guard against itself.
- **A message carries the new value**, as a `fn(T) -> M` rather than a fixed
  message. An enum's tuple variant already is such a function, so
  `Checkbox::new("Mute", Message::Muted)` reads as intended, and no `M: Clone`
  bound is needed.
- **A widget that has a natural size offers `preferred_width` and
  `preferred_height`, and the tree never calls them.** This is the line between
  the toolkit as it is and a layout engine, and it is worth being explicit about
  because it looks like the same thing. An intrinsic-size *protocol* is one where
  the tree asks every widget how big it wants to be and then places it. Here the
  *application* asks, does its own arithmetic, and passes a rectangle — exactly as
  it does for a node with no natural size at all. `Button`, `Checkbox`, `Toggle`,
  `RadioGroup`, `Badge`, `Alert`, `Tabs` and `List` all offer the query; nothing in
  `denise-ui` consumes it.

Two widgets share their geometry rather than each inventing it. `Spinner` is
`RadialProgress`'s ring with the value replaced by a clock, so the centre,
radius, thickness and colour rules live in one place and a spinner and a ring of
the same size *are* the same ring. Sharing the colours also fixed the same bug
twice at once: `interactive_pair` recesses every role to `Base200` when
disabled, so a disabled ring drew its arc in its track's colour and lost its
value — the third time that has bitten, after `RadioGroup`'s disc and `List`'s
selection, and the third time it was found by looking at the rendered showcase
rather than by a test.

`Select` is where the caller-owns-content line got its clearest statement. A
dropdown that opened its own list would have to create nodes from inside
`on_event`, and `EventCtx` deliberately cannot — it emits, asks for focus, asks
for frames, asks to be revealed, and nothing else. That is the same line `Tabs`
drew against owning pages, `List` against owning a viewport, and `push_popup`
against owning content; a select that owned its list would have been the first
widget to own nodes and the exception would have been permanent. So the widget
is the *closed* control, and `widgets::open_select` is a free function that
composes `push_popup` with a `List` — nothing invented, and the open list stays
ordinary nodes the application can inspect or replace.

Getting that helper right needed one distinction `List` already had: it wires
the list's **activation** message, not its selection. A dropdown whose arrow
keys reported every row they passed over would have an application applying
three values on the way to the fourth.

### Animated relayout: the tween, and the stack that moves the siblings

The last foundation the widget tracker waited on, and it split into two pieces
the moment it was designed honestly. `Ui::animate_layout(id, to, duration)` is
the obvious half: the tree carries a node's layout to a target through the
same `set_layout` path the application uses, so damage — the rectangles left
behind and the ones now occupied — and reflow come along on every frame, at
`Spinner`'s 20 fps, landing *exactly* on the target and going silent.

The rules around the tween are where its design lives. A second call
mid-flight retargets **from the current mid-flight rectangle**, so a section
told to close while opening turns around instead of teleporting. A plain
`set_layout` cancels the journey — the application wrote state, and state
written is state shown, the silent-setter rule applied to the tree itself.
Hiding the node **completes the journey instantly**: a hidden node must not
keep the device awake, and half-moved is the one dishonest place to stop.
Tweens are counted by `Ui::animating()`, so the idle-cost evidence covers the
tree's own motion as well as the widgets'.

The second piece is the one hiding in the feature's name: animating a
collapse's height does nothing useful while the ten sections below it sit
still, and per-frame sibling bookkeeping in the application is exactly the
scattered-invalidation disease this toolkit exists to prevent. So the tree
owns one placement rule — `Ui::set_stack(id, spacing)` makes a node a
vertical stack whose visible children are placed top-to-bottom at the running
y, keeping their own x, width and height. It is applied in `reflow`, beside
the scroll offset and for the same reason: one place turns layouts into
bounds, so paint, damage, clipping and hit testing cannot disagree about
where a moved sibling is. It is not a layout engine and not the
intrinsic-size protocol — the tree still asks widgets nothing.

The consequence that generated most of the tests: anything that changes what
a stack should place — a child's size, its visibility, its z, adding,
removing — has to reflow and damage the *stack*, not just the child.
`set_visible` had never reflowed anything, because until now visibility never
moved anyone else. The two pieces compose into the whole feature: put
sections in a stack, tween one section's height, and the stack re-places the
rest on every frame. That is the accordion mechanism, and the widgets over it
can now be thin.

`Collapse` held #34's promise that these widgets would be thin. The widget is
the header alone — title, chevron, the toggle — and the node it sits on hosts
the body as ordinary children, the way `Panel` does. `widgets::set_open` is
the application's whole answer to the message: it drives `animate_layout` on
the node's height, and the expanded height is **remembered at the moment of
folding** rather than configured, so a section that grew a row while open
comes back at its grown height. The body needs no hiding — the node's own
clip crops it, mid-animation included, for free. `Accordion` is a controller
struct, not a `Widget`: exclusivity over N sections the application owns is
application policy, and it lives once, like `ClickPair`. A drawer is
`push_scene` plus `animate_layout` composed by the tree (`Ui::push_drawer`),
and its one new mechanism is closing: the scene pops **when the exit slide
lands** — the first thing to happen *because* a tween arrived, kept internal
rather than growing a public completion hook nobody has asked for yet.

`Carousel` is the first widget to compose two foundations that had not met:
#19's requested animation and #22's pictures. The tracker filed it under
"needed scrolling", wrongly — nothing in it scrolls. Its pages are pictures
rather than nodes (`EventCtx` cannot create nodes, the `Tabs`/`Select` line;
mixed content composes the other way round, as `Tabs` without visible tabs),
and its cost story is the design: idle without an advance clock, nothing;
holding on a page with one, **one wake per interval**, the toast arrangement;
frames only during the quarter-second slide. Like `Spinner` it does not start
itself. One representational decision carries the widget: a slide's
displacement is stored as a **fraction of the width**, because `animate` has
no geometry — the advance clock starts slides without ever having seen the
rectangle, and a fraction lets paint, which has the rectangle, do the
multiply. And one rule got its sharpest statement yet: the advance clock
never emits the settle message, because a message reports what a *person*
did — the clock is the machine talking to itself, the same reasoning as every
silent setter.

`Timeline` is display, and its whole value is one alignment: the disc column
is placed by the widest time in the list, so the discs form a straight line
whatever each row's time says — `List`'s leading-column answer again. The
connector is drawn per gap rather than as one stripe, so it stops at the last
disc instead of running to the rectangle's edge. Mutation testing was the
editor again on both widgets: it deleted two dead clock-reset assignments in
the carousel (every slide ends in `animate`'s landing, which restarts the
hold — the resets on the way in were overwritten before anyone read them),
and it demanded the mid-hold re-ask test, the case where a spinner elsewhere
on screen makes the tree ask a holding carousel every frame — which is
exactly when the landing's restart is load-bearing.

`Table` is the one widget that scrolls itself, and the reason is structural
rather than taste: the **header**. `List` deliberately owns no scrolling — it
cooperates with a `set_scrollable` viewport — but a header inside a viewport
scrolls away with the rows, and a header outside it is a second widget whose
column layout has to be kept in agreement with the first, which is exactly the
drift a table widget exists to prevent. So the table windows its *data*, the
way `examples/table-editor` already did by hand: it owns the index of the
first visible row, draws the header pinned, and scrolling changes which
records are drawn. That is also the answer to the virtualisation question the
widget tracker left open — rows outside the window are never even iterated, so
ten thousand rows paint at the price of nine. It consumes the wheel (the first
widget to; it declines when it has nothing to scroll, so a page it sits on can
have it), and one `Column` definition places both the header title and every
cell — pinned by a pixel test that measures where the ink starts, because that
is where drift would appear. The selection contract, the double-click pairing
and the row colours are `List`'s, as shared code rather than as convention:
`ClickPair` and `row_colors` moved into the widgets' common module so the
disabled-selection answer and the pairing rules live once.

`Rating` is where the rasteriser ran out. Arcs unblocked the rings and the
spinner and do nothing at all for a five-pointed star, which is a ten-vertex
polygon — so the choice was a public polygon API, rating with discs instead of
stars, or one more *shape*. The shape won: `Canvas::fill_star` computes its own
vertices from the same Q16 sine table the arcs use, over a scanline polygon
filler kept `pub(crate)`. The no-path-builder promise survives intact, and a
heart or an arrow can be added the day one is wanted without having committed to
a public path API today. The filler keeps its crossings in a fixed-size stack
array, because this crate has no allocator.

Two things fell out of that. `TURN` is a power of two and does not divide by
five, so a five-pointed star is only approximately five-fold symmetric — the
vertex angles round to the nearest of 65536, which is under a hundredth of a
pixel and invisible, but it means a test comparing rotations has to compare
against a tolerance rather than against bytes. And the widget's value is an
`f32` while its gesture is not: an average of 4.3 has to draw four stars and a
bit, but a person tapping the fourth star means four. Partial fill is the same
star drawn again through a narrower clip, so there is no second shape to keep in
step with the first.

`Rating` also took the fifth outing of the `interactive_pair` trap, and the
first one that was known about in advance and happened anyway — because it
arrived wearing a different face. The four before it were all *one part of a
widget becoming the same colour as another part*, and the guard written for this
one checked exactly that. What actually broke was the empty stars becoming the
same colour as the **panel behind them**: `Panel` fills with `Base200` and a
disabled rating recessed its track to `Base200`, so a disabled "two of five"
rendered as a plain "two". Not a muted control — a wrong one, because the empty
stars are the denominator. Found by rendering the showcase and looking, for the
fifth time.

The test that now covers it is worth more than the fix. A fixed contrast floor
could not do the job: `AA_LARGE` is WCAG's bar for *text* and no pair of base
surfaces in any built-in theme comes within half of it, while no single ratio
separates the good cases from the bad — `Base300` on `Base200` is 1.18 in the
light theme and reads fine, and `Base200` on `Base100` is 1.23 in the dark one
and was the bug. So the floor asks the theme what *it* considers a visible step,
and demands at least that. It adapts to a theme nobody has written yet.

`Avatar` is the opposite lesson: almost all of it was already built. A circular
crop is `Image` with `Fit::Cover` and a full corner radius, which #22 had
already made exact. What earns it a widget is the fallback — initials on a disc
when there is no picture, which on a real panel is most of the time — and the
colour of that disc, derived from the initials into the theme's own accent
roles so that the same person is the same colour every session and a theme swap
still carries a contrast-checked pair. A picture whose buffer does not match the
size it claims falls back too, because a broken asset on a kiosk should still
say who it is.

`Image` draws somebody else's pixels and refuses to be the one who loads them:
the application does I/O and decoding and hands over premultiplied `0xAARRGGBB`,
because a widget that opens files is wrong in an embedded toolkit and unusable
over the FFI. Its four fit modes are pure integer geometry over the rasteriser's
blit, and the one decision with a bug hiding in it is the rounded-corner mask:
the mask follows what is *visible* — the picture's rectangle under `Contain`,
the box's under `Cover` — and must never be derived from the clip, because the
clip is damage, and a damage-restricted repaint of half an avatar has to round
the avatar's corners, not the damage rectangle's. A full radius is a circle, so
the avatar crop is not a special case anywhere.

`Spinner` is also the toolkit's only **unbounded** animation, and it is
deliberately awkward about it: it does not start itself. A spinner receives no
events, so it cannot request frames from a handler, and the application calls
`Ui::request_animation` — which puts the decision to keep a CPU awake at the
line where somebody made it. Its frame interval is 50 ms rather than 16: a
rotating object is the animation least forgiving of a low rate, so it cannot go
as slow as the caret, but the expense is the wake rather than the drawing (the
arc bench says three microseconds), and twenty a second is a third of sixty.

The keyboard is where the widgets deliberately *disagree*, and each difference is
about how much of the set a person can see at once. `RadioGroup` and `Tabs` wrap
past the ends, because a handful of options is all visible and coming round from
the last to the first is obvious. `List` stops, because a hundred rows jumping
from the bottom to the top under a held key is disorienting. `RadioGroup` takes
all four arrow keys; `Tabs` leaves Up and Down for the page below it. Nothing here
follows from a general rule, which is exactly why it belongs in the widgets rather
than in the tree.

One rule about colour is worth stating because it has been got wrong twice. **A
role is only guaranteed to contrast with its own content**, not with the surface
a widget sits on. `Toggle` reached for a fixed `Base100` knob, which is 1.38:1 on
a `Base300` track in the light theme; `Tabs` reached for the role colour as the
selected label, and `Secondary` is 2.34:1 against the light panel. Both now take
both colours from one `interactive_pair`, so the guarantee is structural.

The corollary took two more goes to state generally. De-emphasising text — an
unselected tab, a row a list will not let you choose — costs contrast, and **not
every pair has contrast to spend**. The disabled content `interactive_pair`
*derives* is mixed until it just clears the floor, so muting it drops a label to
2.33:1; and a theme's saturated pairs are only guaranteed to *reach* the floor, so
muting the dark theme's `Primary` content leaves 2.94:1. Both were found by a test,
one per widget, before the rule was. It now lives in one function, `style::muted`,
which mutes a pair that can afford it and returns one that cannot unchanged:
legible and undifferentiated beats differentiated and illegible.

**A tear is not one cost, it is two.** `PresentMode::Immediate` asks for async
page flips, and its documentation had already named the case it would fail —
"reconsider for signage or anything with large fast-moving content, where a tear
crosses something worth looking at". The gallery grew a scrolling viewport, and
the flicker was reported from a Pi within a day. What the note had not said is
that the seam is the smaller half. An async flip never blocks, so nothing paces
the loop: a Pi 3A+ paints a scrolled 1920x1080 viewport in **14.5 ms**, and it
will spend a whole core producing torn frames back to back for as long as a
finger keeps moving. The frame that most needs not to tear is the same frame that
most needs the brakes, so the mode now follows the damage rather than being set
once for everything. `Surface::present` had been handed the damage since the
beginning and DRM had been throwing it away.

**What the damage is measured in took a second go, and a report from the panel.**
The first version compared damaged *area* against the surface, which is the
obvious thing and is wrong: a tear is a horizontal seam, so what decides whether
it is visible is how many **scanlines** the damage spans. The gallery's sidebar
is 300 by 1016 on a 1920x1080 screen — 14.7% of the pixels and 94% of the rows —
so it slipped under an area threshold and tore down almost the full height of the
display, which is exactly the "just a flash from time to time" that came back
after the first fix shipped. Counting rows keeps the case async flips exist for,
a wide shallow band whose seam is as short as the band, and catches the narrow
column an area test cannot see.

**And a third go, from the same panel: rows *covered*, not rows *spanned*.** The
row count started as the damage's bounding box, justified by the observation that
one flip changes the buffer for every rectangle at once, so the beam can seam
anywhere between them. True, and beside the point — the beam can seam there,
nobody can see it there. Untouched rows are identical in both buffers, because
that is precisely what repainting to the buffer's age guarantees, and a seam
between two identical images is not a seam. What the bounding box cost was the
frame nobody had thought to check: the gallery's spinner sits at the top of the
screen and re-damages itself every motion tick, so hovering anything in the lower
two thirds produced a 48-row spinner, a 24-row cursor, and a box spanning the
eight hundred rows between them. Every such frame took a vblank it did not need,
which put a 16 ms motion rate against a 16.7 ms refresh and read as flicker on
the control under the pointer — a regression introduced by the fix above, and
reported within a day of it, the same way the fix above was. Coverage puts that
frame back at 72 rows. Nothing that mattered changes: a scroll and a sidebar are
each *one* rectangle, and one rectangle covers exactly what it spans.

The general lesson is worth more than the fix. Both wrong versions were wrong in
the same way — they measured a proxy (pixels, then the extent between rectangles)
instead of the thing that decides whether a human sees an artefact, which is how
much of what the beam draws differs between the two buffers. The third version
measures that directly, and it is the first one that has no obvious frame it
mishandles.

The measurement around it is worth keeping for the next person, because two
plausible explanations died on the way. Scrolling is **not** over-damaging: one
scroll produces exactly one rectangle, the viewport. It is **not** input
handling: sixteen scroll events cost 6 us. And it is **not** `Ui::paint`'s clear
being redundant under an opaque panel, which looked like a clean 2x on a
synthetic tree and moved 14.5 ms to 14.3 on the real one — the gallery's viewport
is a `Void`, so the clear *is* the background and there is nothing to skip. A
synthetic benchmark can only measure the tree you built for it. What is left is
fill bandwidth, and the only cure for that is not repainting the whole viewport:
[#46](https://github.com/bisand/denise/issues/46).

Three limits were found by building against them rather than by design review,
and all three are now resolved:

- **`Ui::tick` animated only the focused widget** — resolved by
  [#19](https://github.com/bisand/denise/issues/19). Animation is now *requested*:
  a widget asks for frames at the moment it starts needing them
  (`EventCtx::request_animation`, or `Ui::request_animation` for a widget nobody
  touches), and drops out by answering `Wake::Never` — the widget keeps itself
  animating and must stop asking. A toast that appears, waits and fades without
  ever being focused is now expressible; `Toggle` finishes its crossing when
  focus moves away instead of settling defensively. What survived the change is
  the thing the old rule protected: **a tree at rest holds nobody awake**, and
  that is now asserted rather than trusted — `Ui::animating()` reports the set's
  size, a test pins it at zero for a populated idle panel, and hiding or removing
  a node stops its animation so an invisible spinner cannot hold the CPU.
  Unbounded animation is deliberately expressible, because a spinner genuinely is
  one; the accountability for it is the count and the tests, not a prohibition.
- **`Metrics::scaled` was called from nowhere** — resolved by
  [#20](https://github.com/bisand/denise/issues/20), by deciding *who* calls it:
  **the application scales, once, at construction.** It already knows the scale
  factor and already computes every rectangle, so it is the one place that can
  multiply everything consistently — the same argument as compile-time backend
  selection. The pattern is three calls in one place:
  `theme.scaled(factor)` for the widgets' furniture, `Rect::scaled(factor)` for
  every layout rectangle, and a scaled text size wherever one is named.
  Coordinates stay physical everywhere; there is no logical coordinate space and
  no per-widget conversion. `Rect::scaled` scales **edges**, not extents —
  rectangles that touch in the logical layout still touch at fractional scales,
  which is the seam a naive width-times-scale opens. `examples/hello` makes it
  executable: `cargo run -p hello -- --snapshot out.ppm 2` renders the same
  layout at 2×, and the C ABI's `denise_ui_new_scaled` gives embedded hosts —
  the ones that actually receive `WM_DPICHANGED` — the same lever.

  What the decision left dangling for a while was *where a window gets the
  factor from*, since an application built before its window exists cannot know
  it: every windowed example passed 1.0 and came out half size on a Retina Mac,
  correct on the Pi, and nobody could tell whether the mechanism worked because
  nothing on a desktop used it. Closed by `denise_winit::run_with`, which takes
  a builder instead of an application and calls it with the surface size and the
  display's scale factor at the one moment both are known — plus the size in
  `WindowConfig` becoming **logical**, so 1280×800 is the same amount of desk on
  a Pi and on a 2× display, where the surface behind it is 2560×1600. `gallery`,
  `hello` and `table-editor` all build through it; the gallery is the honest
  test, because it does the multiplication once inside its own `add` helper and
  seventy-odd rectangles come out right without one of them mentioning DPI. The
  rough
  edge left open on purpose: widgets default their text to 16 px, so a
  scale-aware application sets text sizes explicitly; theme-driven typography
  would be its own design conversation.
- **The desktop backends cost what a desktop costs, not what a panel costs** —
  found by looking at Activity Monitor rather than by any test. One spinner on a
  Retina Mac burned 48.8% of a core; the same tree on a Pi 3A+ moved the needle
  by 1.37%, and on Windows by 0.5%.

  Three causes, in increasing order of how much they taught. The loop presented
  **every** frame whether or not anything had changed, which is a damage tracker
  the backend then ignores. It also free-ran at 60 Hz while the tree was asking,
  through `Ui::next_wake_ms`, to be woken every 50 ms — the kiosk loops had
  honoured that from the start, so a window was the odd one out;
  `DeniseApp::next_frame_in` is how it stops being. Together those took 48.8% to
  22%.

  The rest was macOS, and it was structural. softbuffer's CoreGraphics backend
  allocates and zeroes a fresh buffer per `buffer_mut`, reports a buffer age of
  0 so the shadow is copied in full, and drops the damage rectangles on the
  floor in `present_with_damage` — three passes over the whole surface, per
  frame, where win32 blits the damage into a persistent DIB. So macOS does not
  use softbuffer here: the pixels live in a pair of `IOSurface`s that
  CoreAnimation reads in place, which is what `denise-macos` was already doing
  for an embedded view. 3.5% at 60 fps, on the same window that started at 48.8%.

  **The pair is the part worth remembering.** One surface is enough for the
  copying and not enough for the compositor: assigning the same object to
  `contents` changes no property, so CoreAnimation never looks at the buffer
  again and the window freezes on its first frame while the application draws
  contentedly into memory nobody reads. Two surfaces make every present a
  visible change. Getting there also cost a permanently held `IOSurfaceLock`
  (which stalls the compositor outright), an implicit 250 ms cross-fade on every
  `contents` assignment (fifteen of them overlapping at 60 fps, which looks like
  two frames a second), and a transaction nobody flushed because this loop
  blocks in `WaitUntil` rather than in AppKit's own cycle.

  The methodological lesson is sharper than the technical one: **a CPU
  measurement cannot tell "efficient" from "not drawing"**, and three of those
  four bugs produced *lower* CPU while being more broken. The check that
  actually settled it reads the layer's `contents` back and asks the surface
  whether anyone is using it — `DENISE_MACOS_DEBUG=1` still prints it — and it
  should have existed before the first measurement was quoted, not after the
  fourth wrong one.

- **How fast animation runs was four constants and nobody's decision** —
  resolved by [#45](https://github.com/bisand/denise/issues/45), immediately
  after the CPU work above made the number matter. `Spinner`, `Carousel` and
  `Toggle` each held a private `FRAME_MS = 16` and `Ui` a private
  `TWEEN_FRAME_MS = 50`: one decision copied four times, reachable from nowhere,
  and wrong in two directions at once. Sixty frames a second is what stops a
  turning arc reading as a stutter on a desktop, and it is 4.20% of a Pi 3A+'s
  core for as long as a spinner is on screen, against 1.37% at fifty.

  The setting is `Ui::set_motion`, and the interesting part is what had to
  change before it could exist. A widget used to answer `next_ms: Option<u64>`,
  which spelled two unrelated things identically: a spinner asking for "another
  frame in 16 ms" and a carousel asking for "the next page in eight seconds". A
  setting that halved one would silently have halved the other, so `Wake` splits
  them — **`Wake::Animating` is a rate the tree owns, `Wake::At` is a deadline it
  must not touch.** Halving the rate now makes an animation coarser and never
  slower, and `Toggle`'s 120 ms crossing, `Carousel`'s advance and the caret
  blink are all unaffected by it, which is a test rather than a claim.

  `Motion::None` is the other half, and is not merely a very slow rate:
  transitions land at their end state, the animating set empties, and the tree
  asks for no wake at all. That needed a second trait method — `Widget::snap`,
  "land what is in flight, now" — because a widget derives its position from a
  clock the tree cannot fast-forward. It is the `prefers-reduced-motion` answer
  and the right setting on hardware where any animation is a bad trade, and it
  stops **motion** rather than **schedules**: the carousel cuts between pictures
  instead of sliding, and still rotates.

  Two placement decisions worth recording. It is on `Ui` rather than `Theme`,
  although the theme already carries `metrics` and `depth`: a theme is an
  identity, and swapping dark for light should not change the power budget,
  while the rate is a deployment decision that differs for the same panel on a
  bench and on a battery. And it is on the *tree* rather than on the widgets,
  which means a custom widget gets the setting for free by answering
  `Wake::Animating` — a widget with its own frame-rate constant would opt out of
  a tree-wide policy without meaning to. `Spinner::with_frame_ms` is the escape
  hatch for the one that genuinely differs, and `Motion::None` overrides even
  that, because reduced motion is a person's decision.

  The gallery on a Pi 3A+ over DRM, one spinner turning, `--motion` the only
  thing changed between runs:

  | | CPU |
  |---|---|
  | 16 ms, the default | 4.20% |
  | 33 ms | 2.06% |
  | 50 ms | 1.26% |
  | off | **0.00%** |

  The 16 ms figure is the same 4.20% measured before any of this existed, which
  is the check that the mechanism costs nothing when it is not used. The last
  row is the one worth having: with no motion the tree asks for no wake at all,
  the backend blocks on input, and the device idles like a panel with nothing
  animating on it — because that is now what it is.

- **Nothing looked for bugs that tests do not look for** — resolved by a review
  pass across security, memory and CPU ([#37](https://github.com/bisand/denise/issues/37)–[#44](https://github.com/bisand/denise/issues/44)), whose finding was
  less about what it found than about what nothing was watching. Four crates in
  the tree parse untrusted bytes (`png`, `zune-jpeg`, `gif`, `fontdue`), seven
  contain `unsafe`, and no job checked either. Now `cargo deny` gates advisories
  and licences, Miri runs over `denise-ffi` — where the raw pointers are — and
  both the image decoders and the C ABI are fuzzed for a minute per push.

  The ABI target justified itself immediately: within a minute of first
  existing it produced `attempt to multiply with overflow` in the caret blink,
  reachable from `Ui::tick` with any large clock, because every animated widget
  computed its next wake with plain `+` and `*` on a number the *application*
  owns. All of them saturate now. Two size validations had the same shape —
  `stride * (height - 1)` in `usize`, which wraps on `armv7` and lets an
  undersized buffer pass the check that exists to catch exactly that — and
  `denise::required_words` is now the one expression all three call sites use,
  in `u64`, where two `u32`s cannot overflow.

  The other half was invariants nobody stated. `Picture` promised
  `pixels.len() == width * height` in prose and enforced it nowhere, so an APNG
  with a first frame smaller than its canvas decoded to `Ok` and then drew
  nothing at all — `PixelView` checks the length and declines, silently. Every
  decoder goes through a checked constructor now. And the windowed examples
  marked the whole surface dirty on every change while their own kiosk arms
  presented exact rectangles: a caret blink copying sixteen megabytes a second
  at 2×, in the examples that exist to demonstrate damage tracking.
  `Ui::pending_damage` is what they were missing — `Ui::damage` reports what
  `paint` last resolved, which during `update` is still the previous frame.

- **Nothing scrolled** — resolved by
  [#21](https://github.com/bisand/denise/issues/21), in the place the issue
  demanded: the tree, not a widget. A node marked `Ui::set_scrollable` becomes a
  viewport; its scroll offset is applied in **`reflow`, the one loop that turns
  layouts into absolute bounds** — so paint, clip and hit testing cannot
  disagree about where a scrolled child is, because they all read the fields
  that loop wrote. Scrolling damages the whole viewport (the honest first
  version), the wheel scrolls the innermost viewport under the pointer after
  the hovered widget declines it, PageUp/Down page the scrollable holding
  focus, and a touch that lands on a viewport's *background* drags it — a touch
  that lands on a widget belongs to the widget, because stealing an in-progress
  press is gesture disambiguation, deliberately not attempted yet. Focus
  reveals: tabbing below the fold scrolls the target into view, which required
  teaching `is_focusable` that an empty clip is reachable when a scrollable
  ancestor can fix it — demanding visibility first was a catch-22 that made
  everything below the fold keyboard-unreachable, and the tests caught it on
  their first run. Widgets whose *interior* moves get `EventCtx::reveal`: `List`
  reveals its selected row, so a keyboard selection below the fold pulls the
  viewport along. Still deliberately out: smooth and inertial scrolling — a
  kiosk redrawing a viewport at 60 Hz to animate a fling is the opposite of
  what the animation contract protects.

### One surface, and what that rules out

A Denise **tree** is a single `Surface`, and on every backend that ships that is
also the whole of what the process has: `denise-drm` owns the display and there is
no window system to open a second one in, and `denise-win32`, `denise-macos` and
`denise-activex` are *embedded* — the host owns the window and Denise owns one
rectangle inside it. A control that spawned a top-level window would escape its
host's modality, land on the wrong monitor and outlive the dialog that owns it,
which is the behaviour that makes embedded controls hated. None of that has
changed and none of it is going to.

So a modal is `Ui::push_scene`: another root over a dimmed backdrop, inside the
same buffer. An `Alert` is an inline banner, not a message box.

**The desktop backend is the exception, since [#83].** `denise-winit` can run
several trees at once, one per window — `DeniseApp::take_windows` hands back a
`WindowRequest` and gets a window with its own surface, damage tracker and frame
deadline. It is there because a desktop application asked for a settings form that
is a *window*, and on a desktop that is the native answer rather than a scene.

What that does *not* do is make Denise a multi-window toolkit:

- **The tree never learns about it.** `denise-ui` is untouched by the feature and
  has no idea another tree exists. A window is a `DeniseApp`, which is a backend
  concept, so a form is composed the same way the main window is and shares
  nothing but whatever the application chose to put in an `Rc`.
- **It cannot be portable, so it is not pretending to be.** The capability lives
  in the desktop backend and nowhere else; an application built for the kiosk
  links `denise-drm` and never sees it. This is the same compile-time split
  everything else about the display already uses.
- **Modality is ours, not the platform's.** Only Windows has a real modal dialog
  (`with_owner_window` plus `set_enable(false)`); macOS gives z-order through
  `addChildWindow:ordered:` and nothing more; X11 and Wayland are not reachable
  through winit at all. So the runner blocks a modal's owner itself and the
  platform calls are appearance. `denise-winit::owner` says which is which.
- **There is no handle to somebody else's window.** Nothing can close, invalidate
  or reach into a window it did not build. A form closes *itself* through
  `exit_requested`, and a window that wants another one's state watches for it.
  This is deliberate: a handle would be the seam through which the tree stops
  being the only thing that owns a tree.

The same stack carries popups, since [#18]. What was already there did most of
the work: input only ever reaches the topmost scene, so a scene *is* input
capture, and Tab was always confined to it — the modal focus trap was
structural before it had a name. `Ui::push_popup(anchor, size, side)` adds the
dismissal rules on top: the container is placed beside its anchor by
`overlay::anchored` — flipping to the other side when the surface runs out — a
press outside it closes the popup *and is swallowed* (the dropdown that closes
and also activates the button behind it is the classic bug, and it is pinned by
a test), Escape closes before the focused widget sees the key, and focus
returns to the anchor however the popup closes. Each layer keeps its honest
tool: a modal is `push_scene` with a dim, a popup is `push_popup` without one,
and a tooltip is no scene at all — a non-interactive node placed with
`anchored` at a high z, invisible to hit testing like any `Label`.

A tooltip turned out to want none of that machinery, and the reasoning is the
same one applied one step further. Everything hard about a tooltip happens
*before* it would exist: the dwell timer has nothing to belong to yet, the
placement needs another node's bounds, and it goes when the pointer leaves the
**anchor** rather than the bubble — which the pointer must never be able to
enter. All three are things the tree already tracks, so `Ui::set_tooltip(id,
text)` stores a string on the node and the tree owns the rest, drawing it beside
the cursor sprite: over everything, because it is not part of the tree at all.

The coupling that makes it work is the one worth remembering: **`next_wake_ms`
has to report the dwell deadline.** A kiosk blocks on input until the tree says
it wants waking, so a deadline left out of that answer is a bubble that appears
the next time something unrelated happens. And a tooltip is a *pointer*
affordance — a touchscreen has no hover, so on a touch-only panel it does
nothing at all, which the documentation says rather than leaving somebody to
find out on a kiosk.

`Ui::toast` is the same idea a second time, and the pair is worth reading
together because they came out of different arguments to the same place. A
toast is not a node because **removing itself is the whole point** and only the
tree can remove things, because two of them must not land on top of each other
and no widget can see its siblings, and because not being a node is what makes
it invisible to Tab and to hit testing without anybody remembering to make it
so. `Alert` stays as the *inline* banner; a toast is the same message when
there is nowhere in the layout to put it.

Two things about it are worth keeping. It **swallows a press inside itself and
dismisses** — a toast that let a tap through would have somebody clearing a
notification and pressing the button it was covering, which is the dropdown bug
in a new hat. And it is **mostly idle**: it fades in, holds, fades out, and only
the fades need frames, so during the hold the tree asks for exactly one wake, at
the instant the fade-out starts. A holding toast costs one wake rather than two
hundred and forty — the opposite of `Spinner`, which looks like the same kind of
feature and costs a wake per frame throughout.

Both of them had the same bug during development, and it is the one to expect
from anything drawn outside the tree: **damage measured after the state that
knew where the pixels were had already changed.** The tooltip cleared its
remembered position before the damage could read it; the toast stack could not
say where an expired toast had been, because the layout no longer included it.
The tooltip damages before every state change; the toast remembers what its last
paint covered. Neither is visible in a pixel test, because those repaint
everything.

One paint rule came with it: **only the topmost veil paints.** Two dimmed
scenes would otherwise double-darken everything under both, and a popup inside
a modal must not darken the modal it serves.

[#18]: https://github.com/bisand/denise/issues/18
[#83]: https://github.com/bisand/denise/pull/83

An application that wants a *native* dialog on a desktop build should call the
platform for one — `MessageBoxW`, `NSAlert` — behind the same `cfg` seam it
already uses to pick a backend. It knows which build it is; the toolkit would
have to guess, and guessing wrong means a kiosk trying to open a window on a
machine with no compositor.

### Damage is the toolkit's job

This is the part that changed as a result of M2. A bug on the Pi produced a
stale colour on a card because the application's idea of "what changed" left out
the one field that decided the pixels. The fix is not care; it is that the
application no longer has that job:

- `Ui::widget_mut` invalidates **on access**, before you have changed anything.
  Taking `&mut` to a widget is the declaration that it will look different.
- Hover, press, focus and enabled are tracked by the tree, so a widget cannot
  forget to invalidate a state it does not own.
- Moving, resizing, showing, hiding, adding or removing a node damages both the
  rectangles it vacated and the ones it now occupies.

[`denise-ui/tests/scene.rs`](../denise-ui/tests/scene.rs) asserts the property that
subsumes all of it: after any poke at the tree, an incremental repaint through a
double-buffered swapchain with a padded stride is **pixel-identical to a full
repaint**. A missed invalidation fails there rather than three weeks later on
somebody's panel.

### Scenes, not dialog widgets

A modal is a scene pushed on the stack, not a widget inside the page. Input goes
to the topmost scene only, so nothing underneath is hittable, focusable or
reachable by Tab — a property of the stack rather than something each dialog has
to enforce. The backdrop is painted per damage region, never over the whole
surface.

### Hit testing and paint order

Siblings are kept sorted by z as they are added, so flattening the tree is a
plain depth-first walk, cached and rebuilt only on a structural or z-order
change. Non-interactive widgets are invisible to hit testing, which is why a
`Label` inside a `Button` does not swallow the click.

### Numbers, at five hundred nodes

`cargo bench -p denise-benches --bench ui`, on an Apple M-series at 1920×1080
with a 500-node tree — twenty panels of controls, which is a busy HMI:

| Benchmark | Time |
|---|---|
| Hit test, topmost node | 4.0 ns |
| Hit test, deepest node | 297 ns |
| Hit test, miss (walks everything) | 265 ns |
| Rebuild the paint order after a z change | 754 ns |
| Full repaint | 497 µs |
| One button hovered, then unhovered | 5.3 µs per frame |
| One button pressed, then released | 4.1 µs per frame |

A frame that changed one button costs **about a hundredth of a percent** of a
full repaint of the same tree. That ratio is the whole design, measured against
the size the architecture was specified for.

## The rasteriser

`denise-render` draws rectangles, rounded rectangles, circles, arcs, stars,
lines, borrowed pixel blocks (blitted 1:1 or nearest-neighbour scaled) and
source-over alpha straight into a `Frame`. It needs neither `std` nor `alloc`, contains no `unsafe`,
and uses **no floating point at all** — anti-aliasing coverage included.

That last one is a deliberate trade. Integer coverage means no `libm` on `no_std`
targets, no FPU traffic where that costs, and output that is bit-identical between
x86 and ARM — which is what makes a pixel-exact reference test meaningful on a
developer's laptop *and* on the Pi. Rounded corners are evaluated analytically per
scanline with an integer square root, at four sub-rows per scanline.

The clip is the only damage-awareness the drawing code has. Widget code paints as
though it owned the whole window; restricting the clip to a damage region turns
that into an incremental repaint, so there is never a second draw path to keep in
step with the first.

### Numbers

Apple M-series, `--release`, so read the *ratios*, not the absolute times — a Pi 4
is an order of magnitude slower on memory-bound work.

| Benchmark | Time |
|---|---|
| 1080p scene, full repaint | 211 µs |
| 1080p scene, typical damage (0.4% of the surface) | **5.7 µs** |
| 1080p blit, whole buffer | 101 µs |
| 1080p blit, same damage | **0.88 µs** |
| 800×480 scene, full repaint | 54 µs |
| Rounded rect fill 1600×900, r=8 / r=32 | 77 µs / 78 µs |
| 800×480 image blit, opaque / per-pixel alpha | 136 µs / 398 µs |
| 800×480 image blit, nearest-neighbour 2× upscale | 190 µs |

A damaged frame costs **37× less** than a full one, and the rounded-rect cost is
flat in the radius — anti-aliasing is paid per perimeter pixel, not per area. Both
are the properties the design was aiming at.

Two results worth keeping in view:

- **A padded stride costs 3× on a full clear** (59 µs → 185 µs at 1080p). DRM hands
  out pitch-aligned buffers, so the padded number is the one that will matter on
  hardware. Whether that gap is stride handling or simply the larger buffer falling
  out of cache is not yet established.
- **`fill_rect` currently measures slower than `fill_rounded_rect` on the same
  rectangle**, which cannot be right — the rounded path does strictly more work.
  Unexplained, and flagged rather than papered over.

### The built-in font

Five by seven in an eight-row cell, monospace, integer-scaled. Glyphs are ASCII
art in the source, packed into bits by a `const fn` at compile time, because a
table of hex bytes cannot be reviewed and a picture of a `Ø` can. It is the first
of the three [text tiers](#text), and the only one that needs no feature flag, no
file and no allocator.

```bash
cargo run -p denise-render --example fontdump -- "Kjøre på Æ"
```

Anti-aliased glyphs from the other tiers arrive as 8-bit coverage masks. The
blitter walks each row in runs: solid runs — the inside of a glyph — go through
the span blend, empty runs are skipped, and the per-pixel path is paid only on the
rim. That matters because the M1 benches put the per-pixel path at 31 Mpx/s
against 457 for spans on a Pi 3, and predicted glyphs would be where the gap got
paid.

```bash
cargo bench --workspace
```

CI compiles the benches but does not gate on their timings: wall-clock variance on
a shared runner is far wider than any threshold worth setting. The regression gate
belongs on a self-hosted Pi, or on instruction counts.

## Text

Three tiers, chosen by what a panel actually has to draw. The cost column is the
increase in a stripped, statically linked `aarch64-unknown-linux-musl` binary,
measured rather than estimated.

| Tier | Feature | Cost | Buys |
|---|---|---|---|
| Built-in bitmap | none | 0 | Latin plus `ÆØÅ æøå ÄÖÜ äöü Éé ß °`, whole-number scales |
| TrueType | `truetype` | **+145 KB** | Real faces, anti-aliased, proportionally spaced, any size |
| Shaped | `shaping` | **+3.1 MB** | Ligatures, bidirectional text, font fallback, complex scripts |

For scale, the whole of Denise, DRM, evdev and the widgets is **848 KB**, so the
shaping tier is four times the rest of the toolkit put together.

The numbers that decide between them are stark in both directions. On a Norwegian
pangram, `truetype` and `shaping` produce lines **two pixels** different in total
width — three megabytes for two pixels. On Arabic they are not comparable at all:

- the built-in font draws **boxes** — obviously missing, obviously a defect;
- `truetype` draws **the right glyphs, unjoined and in logical order**, which is
  fluent nonsense: it looks like text, it is wrong, and nobody who cannot read the
  script will notice;
- only `shaping` joins them and runs them right to left.

That middle result is the one to be careful about. `examples/specimen` takes a
sample string as its third argument precisely so it can be checked before a device
ships:

```bash
cargo run -p denise-text --features truetype,shaping \
  --example specimen -- specimen.ppm MyFont.ttf "sample text"
```

**No font ships with Denise, and none will.** Type designers' licences differ, and
embedding somebody's typeface in a toolkit is a decision for whoever ships the
device. There is also no font discovery: nothing is read from a system font
directory, because a device that boots from flash with a read-only root very often
has none, and a UI whose text depends on that is a UI that fails in the field.
`cosmic-text`'s own `new_with_fonts` turned out to load the host's fonts anyway —
812 of them on the machine this was written on — which is why the database is
built by hand.

### The glyph cache

One buffer of coverage bytes, shelf-packed, with a size fixed at construction:
64 KB by default. A panel with a twenty-year service life wants "the glyph cache
is exactly 64 KB", not "however many glyphs the user has typed since Tuesday".

When it fills it resets wholesale rather than freeing rectangles, and counts the
reset — so a cache that is genuinely too small shows up as a rising number rather
than as a mystery. Measurement goes through it as well as drawing, which is what
stops a label being re-outlined on every layout pass.

| Benchmark | Time |
|---|---|
| Cache hit | 2.8 ns |
| Cache miss, 16 px | 100 ns |
| Cache miss, 24 px | 213 ns |
| Measure a 16-character label | 252 ns |
| Draw a 16-character label, 16 px | 1.39 µs |
| Draw the same label, 48 px | 6.91 µs |

## Keyboards

`KeyCode` names a *position*; what it types is a property of the user's layout.
`denise-evdev` ships US and Norwegian as static tables, with dead-key composition,
AltGr as a third level, and a Caps Lock that reaches `æøå` without turning `1`
into `!`.

The layout is read from the system: `DENISE_KEYMAP`, then `XKB_DEFAULT_LAYOUT`,
then the console keyboard configuration files distributions actually write. On the
Pi this was developed against, `/etc/conf.d/loadkmap` says Norwegian and the panel
picks it up with nothing set by hand.

```console
$ /tmp/panel
keymap  no (from /etc/conf.d/loadkmap)
```

The composition table is generated from Unicode's own canonical composition data
rather than typed out — a hand-written table of a hundred accented letters is a
list of a hundred chances to be subtly wrong about one of them.

### Why the tables are ours, when the choice is the system's

Reading which layout a system wants is easy. Reading the layout *itself* is the
part that would remove these tables, and both ways of doing it cost more than they
save:

| | What it gives | What it costs |
|---|---|---|
| `KDGKBENT` on a VT | The kernel's real keymap, dead-key table included | `/dev/tty0` is `root:root` mode 600 everywhere checked |
| libxkbcommon | Every layout in xkeyboard-config | A C library and a runtime data directory |

Denise otherwise runs unprivileged, needing only the `video` and `input` groups,
and a static binary needs no data directory. Giving up either to read a keymap is
a poor trade. So the choice comes from the system and the data comes from here.

The cost is real and stated: a system configured for a layout Denise has no table
for falls back to US, **visibly**, through the reported source — rather than by
typing the wrong thing. Adding a table is about thirty lines, because a layout
lists only what differs from the Latin alphabet. Needing root is forever.

Control characters are never text. Enter, Tab and Backspace produce `Key` events
and nothing else, so a field can insert everything it receives without filtering
and a key binding cannot be shadowed by a stray control character.

### Muting the console

Reading evdev does not stop anyone *else* reading it. On a console-booted kiosk
the login shell behind the UI receives every keystroke as well, so typing into a
Denise text field also types at the shell — and a form field that happens to
contain `reboot` followed by Enter does what it says. Holding DRM master stops the
console drawing; it does nothing about the keyboard.

`Console::mute_keyboard` sets `KDSKBMODE` to `K_OFF`. evdev sits below the console
layer, so Denise still sees everything and the shell sees nothing. It is paired
with `KDSETMODE`/`KD_GRAPHICS`, which stops console blanking on an idle panel and
stops the kernel repainting text after an oops.

Two things make this safe to ship rather than a footgun:

- **The guard restores on drop**, including while a panic unwinds, and it puts
  back the mode it *read* rather than a guess at the default.
- **A pty is refused.** `/dev/tty` over SSH is not a console, and `KDGKBTYPE` is
  the ioctl that says so. Without that check, `open` would hand back the first
  thing that opened and the developer's own terminal would be the one muted.

`K_OFF` also swallows `Ctrl+Alt+F2`, so a muted console cannot be escaped from at
the keyboard, and nothing restores it after `SIGKILL`. The escape hatch, over SSH,
is `kbd_mode -u -C /dev/tty1`.

## Embedding

M5 is the other direction. Everything before it is Denise owning the display; this
is Denise owning one rectangle inside an application that already exists — an MFC
dialog, a Cocoa window, a C or C# or Python host.

The shape is the same in all of them, and it is the one thing worth getting right:
**the host owns the window, the event loop and the pixel buffer; Denise owns the
widget tree and draws into whatever it is handed.** There is no `run` function in
any of these backends, and no `Surface` in the C ABI. A library that owned either
would be unembeddable in exactly the places this exists for.

| | Backing store | Present | Verified |
|---|---|---|---|
| `denise-ffi` | the caller's, described by `DeniseFrame` | the caller's problem | C and C++ example built and run in CI |
| `denise-macos` | `CGBitmapContext`, CoreGraphics owns the pixels | `setNeedsDisplayInRect:` then `CGContextDrawImage` | rendered through AppKit's own `cacheDisplayInRect:` |
| `denise-win32` | 32-bit top-down DIB section | `InvalidateRect` then `BitBlt` | compiles, unit tests pass on a Windows runner |

Three things fell out of doing it three times:

- **Damage means different things.** On DRM a page flip swaps whole buffers, so
  damage saves rasterisation and no bandwidth. On Win32 and on AppKit it saves
  both — `BitBlt` moves only what it is given. The same rectangles, worth
  measurably more.
- **Row zero is not agreed on.** A `CGImage` is bottom-up; a DIB section is
  bottom-up unless you ask for a negative height; Denise's row zero is the top.
  Neither platform reports a mistake here. It renders upside down and looks like
  somebody laid the widgets out wrong.
- **There is already a cursor.** Both hosts draw one, so the composited sprite has
  to stay off — and it did not, because the tree revealed it on every pointer move.
  `Ui::show_cursor` is now a decision that sticks.

The Windows CI job caught the first thing it was pointed at: two virtual key codes
naming the same position, because the unsided `VK_CONTROL` deliberately aliases
`VK_LCONTROL` and the test excluded only the shift case. The mapping was right; the
test's exclusion was too narrow. The fix worth having was not the one-line one —
`denise-win32`'s keymap is now platform-independent and its tests run everywhere,
the same split `denise-drm` and `denise-evdev` already made. A table of a hundred
numbers is exactly the thing that breaks, and a CI runner is a slow place to find
out.

### Scripting the ActiveX control

Embedding a control and *driving* one are different problems, and the second is
`IDispatch`. The surface is four members and two events:

| Member | Dispid | |
|---|---|---|
| `Text` | 1 | property, read/write — the field's contents |
| `Caption` | 2 | property, read/write — the heading |
| `Enabled` | 3 | property, read/write |
| `Refresh` | 4 | method |
| `Change` | 1 | event — somebody typed |
| `Click` | -600 | event — the button, at OLE's standard `DISPID_CLICK` |

```vbscript
Set p = CreateObject("Denise.Panel")
p.Caption = "Hei"
```

There is no type library, so a host is late-bound: it asks for a name and invokes
it. VBScript, JScript, VB6 through an `Object` variable, MFC's
`COleDispatchDriver` and every OLE container work that way and need nothing else.

**PowerShell is the exception, and chasing it was the most instructive part of
this.** It builds its member table from `ITypeInfo` and will not ask for a name it
has not been told about, so `$panel.Caption` fails with "cannot be found on this
object" before a single COM call is made — nothing is wrong with the control, it
has simply never been asked anything.

`CreateDispTypeInfo` looked like a cheap way out: hand it a method table and it
builds an `ITypeInfo` in memory, no `.tlb`, no `LIBID`, nothing to keep in step.
Two rounds of that produced two better errors and no fix. The first was mine —
every put claimed to return `VT_EMPTY`, which is a variant that *holds* nothing
rather than a call that *returns* nothing, and PowerShell duly unwrapped a null.
The second was the API's: `CreateDispTypeInfo` builds a vtable-shaped description,
`TKIND_INTERFACE` and not `TKIND_DISPATCH`, so PowerShell looked for a
dispinterface, did not find one, and produced an object with no members and no
complaint at all. Nothing in the method table changes the kind.

So it was removed. `GetTypeInfoCount` answers zero, which is honest, and PowerShell
reaches the control through `[System.__ComObject].InvokeMember` — which goes
straight to `Invoke` and works. The real fix is a registered type library, and it
buys a form designer's property sheet and early binding at the same time; it is on
the outstanding list rather than half-built.

There was a third thing wrong, and CI found it first: reading that description
crashed the Windows runner outright, `STATUS_ACCESS_VIOLATION`. The module holding
the method table freed its own name buffers when the builder returned, on the
strength of a comment asserting that `CreateDispTypeInfo` copies what it is given
— written as a fact and never checked. So the description outlived its own strings.
It was unsound before it was useless.

The lesson worth keeping is not about COM. Each round of this was a rebuild on a
VM, a screenshot, and a guess about an adapter that cannot be run from the machine
the code is written on — while the answer sat in a CI log nobody opened, on the
very commit that added the tests meant to answer it. The cheap diagnostic already
existed. Writing one and then not reading it is worse than not writing it, because
it buys the feeling of having checked.

Two more things are worth naming. The first is that **a host is not tidy about
`wFlags`** — VBScript sends `METHOD | PROPERTYGET` for anything whose result it
uses, because at the call site it does not know which the object has. So the flags
are a set of things the host would accept, and the control picks the one the member
offers. That decision is a pure function of a table, so it lives outside
`cfg(windows)` with tests, next to the HIMETRIC arithmetic and for the same reason.

The second is **re-entrancy**, which is where a control like this actually breaks.
A click handler assigning to `Caption` is an ordinary thing for a script to do, and
it arrives while the tree that raised the click is still running — with the
control's own `RefCell` borrowed around it. Pushing it straight back in would
panic, unwinding out through a COM method into somebody's script engine. So a
property put made while the tree is running records the change and stops, and the
tree applies whatever a handler left behind in a second pass before it returns. One
extra pass, deliberately not a loop: a handler that assigns on every event would
otherwise never hand control back.

On the ARM64 machine that is a few hundred round trips — a `Change` handler reading
`Text` on every keystroke of a sentence, and eighteen clicks each assigning
`Caption` — with the borrow held around all of them.

### The header is the contract

`denise-ffi`'s header is written by hand and the Rust is checked against it, not
generated from it. A generated header follows whatever the implementation says this
week, which is the opposite of what a stable ABI means.

[`tests/header.rs`](../denise-ffi/tests/header.rs) does the checking, and it earns its
keep on the parts a linker cannot. A missing declaration is a link error the first
time anybody tries. A key number that differs between the two sides is not: the
host presses Enter, the field receives Home, and nothing anywhere says so.

The numbering is not arbitrary either. A key position is *named* after the US
layout, so positions carrying an ASCII character there are numbered with it —
`DENISE_KEY_A` is `0x41`, `DENISE_KEY_SEMICOLON` is `0x3B`. Half the table needs no
lookup and a key log is readable in hex.

## Theming

The role vocabulary is borrowed from [daisyUI](https://daisyui.com), which got the
important part right: a widget never names a colour, it names a **role**, and every
surface role has a **content** partner. Swapping a theme cannot produce unreadable
text, because readability is a property of the pair rather than of the widget.

```rust
let (background, foreground) = theme.pair(Role::Primary);
let corner = theme.radius(Radius::Box);
```

Twenty roles — `base-100/200/300` plus `base-content`, then `primary`,
`secondary`, `accent`, `neutral`, `info`, `success`, `warning` and `error`, each
with a content partner. Three radius tokens by widget class (`Selector`, `Field`,
`Box`) rather than one constant per widget, which is what stops the set drifting.

A theme is built from nine seed colours; the two recessed base surfaces and all
nine content colours are derived by walking towards black or white until the mix
clears **WCAG 4.5:1**, so a derived theme keeps its hue instead of collapsing to
black on white. `Theme::from_seeds` is a `const fn`, so the built-in themes cost
nothing at runtime and cannot drift out of step with the derivation rules.

`Theme::validate` checks every pair, and it earns its place: it caught that pure
magenta and `#FF5555` both top out near 6.7:1 against black and cannot reach AAA,
which is why the high-contrast palette uses lightened variants.

Three themes ship — `LIGHT`, `DARK`, and `HIGH_CONTRAST` for panels read in glare
or through a visor. On a device booting from flash, an unused theme is bytes
somebody paid for.

### What was not borrowed

| | |
|---|---|
| **OKLCH storage** | Cube roots mean floats, which mean `libm` on `no_std` and output that is no longer bit-identical across architectures. Colours are sRGB, derived with integers. |
| **`--noise`** | A per-pixel texture makes every pixel differ from its neighbour, so no damaged region can be repainted without a seam against the region beside it. It turns every frame into a full repaint. |
| **35 built-in themes** | Three. |
| **`--depth` as a shadow** | Kept as a number. A real blur is expensive in software and spills outside the widget's bounds, so every damage rectangle would have to be inflated by the blur radius. |

## The browser example

`examples/browser` exists to answer one question with a workload nobody would
design a panel toolkit for: can real web pages be rendered *with Denise
widgets*? Hacker News, Wikipedia and DuckDuckGo Lite say yes — fetched over
rustls, parsed by html5ever, and every visible thing on screen a widget: page
text through the shared engine, images through `denise-image`, form controls
as the toolkit's own `TextInput`, `Checkbox`, `RadioGroup` and `Select`,
submitting real GET and POST. No JavaScript, which is the line between an
example and a decade.

The toolkit lacks three things a browser needs, and the point of the example
is that all three were built *on top*, from public API, without touching a
crate:

- **A layout engine.** Block boxes and a run-aware line breaker over
  `measure_line`, each line committed at the tallest ascent on it.
  `draw_line` taking a baseline origin — a decision made back in M4 for no
  reason this ambitious — is exactly what mixed sizes on one line require.
- **Rich text.** A widget owning precomputed styled fragments. Paint measures
  nothing; links are rectangles recorded at layout time. Bold is a second
  font, because a `TextStyle` is honestly a font and a size.
- **A waker.** There is none, deliberately, so the fetch thread posts to a
  channel and the loop polls at 40 ms *only while something is in flight*.
  Idle still costs nothing with a network attached, which was the property
  worth defending.

What the workload found, recorded for whenever these earn fixing: `Ui::new`
hardcodes the 64 KB glyph atlas, and a style-heavy page would like to hand it
a bigger one; there is no multiline text editing for `<textarea>` to map
onto; `blit_scaled`'s nearest-neighbour shows on photographs; and a font's
character map can lie — macOS Arial claims U+21BB and draws the `.notdef`
box, which is why the reload button became an arc the rasteriser draws
itself. A browser is also mostly scrolling, so
[#46](https://github.com/bisand/denise/issues/46) matters more with this
example in the tree than it did before.

## Milestones

| | | |
|---|---|---|
| **M0** | Workspace, `Surface`/`InputSource`, winit backend, damage tracking, CI | ✅ |
| **M1** | Software rasteriser: rects, rounded rects, lines, clipping, alpha blend. Benches. | ✅ |
| **M1.1** | Theming: semantic colour roles, guaranteed-contrast content pairing, geometry tokens. | ✅ |
| **M2** | DRM/KMS with legacy modesetting and page flip; fbdev fallback; evdev input. Runs with no X. | ✅ |
| **M3** | Scene stack, z-index, modal dialogs, cursor sprite. Label, Button, TextInput. CoreCanvas 0.4 parity. | ✅ |
| **M4** | Text: three font tiers behind feature flags, a bounded glyph atlas, keyboard layouts with dead keys. | ✅ |
| **M5** | C ABI, macOS `NSView`, Windows child-HWND control, ActiveX shim. | ✅ |
| **M6** | Widening the widget set, one at a time, against [#6](https://github.com/bisand/denise/issues/6). | ◐ |

M2 does not start until M1 is benchmarked. M5 does not start until the Pi story is
solid — that is the entire point of the project.

M2 shipped legacy modesetting rather than atomic, which reverses the original
plan. What atomic buys is `FB_DAMAGE_CLIPS`, plane composition and tear-free
guarantees; a page flip swaps whole buffers, so damage saves rasterisation rather
than bandwidth, and the one plane worth having — the hardware cursor — has a
legacy equivalent. Atomic slots in behind the same seam when planes earn it.

M3 pulled the built-in bitmap font forward from M4, because a milestone shipping
Label, Button and TextInput without glyphs would have been a milestone in name
only. M4 then found the other half of that gap: `denise-evdev` reported key
*positions* and never turned them into text, so M3's text fields could not receive
a single character from real hardware. Tab and Enter worked, which is why it
looked fine on the Pi.

M4 also added a tier the bootstrap did not name. It listed `cosmic-text` and
`fontdue`; measuring them showed 3.1 MB against 145 KB, and a middle tier with
real fonts but no shaper is what most panels actually want.

M5 was gated on the Pi story being solid, which it was not quite: the console
keyboard was still unmuted, so every character typed into a Denise text field was
also typed at the login shell behind it. That is fixed first — `Console` in
`denise-evdev`, restoring on drop — and then the milestone starts.

M5's ActiveX shim was written twice, and the first attempt was abandoned on
purpose. It would have sat entirely on top of `denise-win32`, which at the time had
never run, and nothing available could have checked it beyond "it compiles" — a
long way from "a container can host it". So the registration table shipped alone,
and the COM object waited until the control underneath it had put a window on a
screen. It then took one sitting, and the container found no bugs in it at all.

What did find one was a test: `2540 / 96` as an integer constant is 26 rather than
26.458, so every extent the control reported was 1.7% short. A container would have
drawn it slightly too small forever and nothing would have pointed at a constant.

Still outstanding, and deliberately not hidden:

- **No form editor has hosted it.** The two things one would need are now there —
  a type library, so a property sheet and an object browser have something to
  read, and `IViewObject2::Draw`, so a control dropped on a form is a picture
  rather than a blank rectangle. Both are exercised on every push, the library by
  building one and reading it back and the view by drawing into a memory DC and
  counting pixels. Neither has been in front of the thing they are for.
- **`denise-win32`'s edges are unverified.** It runs, and the input path is
  confirmed on Windows 11 ARM64 — Tab, AltGr, dead keys, hover and mouse-leave.
  Not yet exercised: `SetCapture` on a drag off a pressed button, the wheel's
  screen-to-client conversion, and DPI changes, which is the one I trust least
  because `WM_DPICHANGED` reaches top-level windows only. It has never been
  hosted inside a dialog, which is what `WM_GETDLGCODE` exists for.
  [docs/windows.md](windows.md) is the checklist.
- **Touch is unverified on hardware.** The multitouch slot path is unit tested and
  a single touch is routed to widgets as a pointer would be, but no physical
  touchscreen has driven it.
- **No text selection, clipboard or word motion** in `TextInput`. The measurement
  it needs now exists; the editing model does not.
- **The Norwegian layout is a reconstruction.** `æøå` and the `¨^~` dead key are
  certain; the AltGr assignments on the `+?` and `´` positions are less so, and
  want checking against a physical keyboard.
- **Only two layouts.** US and Norwegian. Adding one is about thirty lines,
  because a layout table lists only what differs from the Latin alphabet.
- **No layout engine.** Nodes are positioned with explicit rectangles relative to
  their parent, which is what a fixed-resolution panel wants; the opt-in vertical
  stack (`Ui::set_stack`) is the one placement rule the tree owns, and a
  constraint solver can still be added over all of this without changing anything
  below it.
