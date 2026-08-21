# denise-evdev

[![crates.io](https://img.shields.io/crates/v/denise-evdev?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-evdev)
[![docs.rs](https://img.shields.io/docsrs/denise-evdev?color=94E2D5&label=docs.rs)](https://docs.rs/denise-evdev)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

Linux evdev input for **[Denise]**, a direct-rendering UI toolkit in Rust for
embedded Linux and systems without a desktop environment.

Reads mice, touchscreens and keyboards straight from `/dev/input/event*`, with **no
display server in the way**, and turns them into `denise::InputEvent`s.

```rust
# #[cfg(target_os = "linux")]
# fn demo() -> Result<(), Box<dyn std::error::Error>> {
use denise::{InputSource, Size};
use denise_evdev::InputBackend;

// The surface size is what absolute touch coordinates get scaled into.
let mut input = InputBackend::open_all(Size::new(1280, 800))?;

let mut events = Vec::new();
input.poll(&mut events);      // never blocks
# Ok(())
# }
```

## Blocking, and how not to spin

`poll` drains whatever is ready and returns. A frame loop that wants to sleep
should wait on `InputBackend::raw_fds` together with the DRM device's descriptor,
so the process idles in the kernel until either input arrives or the display
retires a flip — rather than waking up to ask.

That list changes under you. A wireless mouse that was asleep at startup has no
device node until somebody moves it, and `poll` opens it when it appears — so ask
`devices_changed()` each pass and take `raw_fds` again when it says yes. Holding
the first list forever is how a panel ends up with a mouse it can see in
`/dev/input` and cannot read.

## Keyboards

Key positions are translated to `KeyCode`, then composed into text: dead keys,
AltGr, and the modifier state that decides both. Two layouts ship, **US** and
**Norwegian** — the Norwegian AltGr assignments are a careful reconstruction and
want checking against real hardware.

There is also a console guard: on a bare VT the kernel is still echoing every
keystroke behind the panel, so `console` puts the tty into a raw, graphics mode for
the life of the process and restores it afterwards — including on panic, which is
the case that leaves a machine unusable if it is forgotten.

## Testing

`translate` and `keymap` are platform-independent and unit tested everywhere. That
is not tidiness: multitouch slot tracking, frame batching and modifier state are
the parts that break, and each is far easier to pin down as a table of raw event
codes than by dragging a finger across a panel and guessing. Only device discovery
and reading are gated to Linux.

Three examples help on a new board: `input` prints translated events, `keys` shows
what a keyboard produces layout by layout, and `pointer` tracks a cursor.

## Permissions

Reading `/dev/input/event*` needs membership in the `input` group, or root. Being
able to read every keystroke on the machine is exactly as sensitive as it sounds,
which is why the group exists.

## Platform

Linux only; elsewhere the crate compiles to almost nothing. `unsafe` is permitted
here and every block carries a `// SAFETY:` comment.

## Where this sits

Implements `denise::InputSource`. Pair it with
[`denise-drm`](https://crates.io/crates/denise-drm) or
[`denise-fbdev`](https://crates.io/crates/denise-fbdev) for output.

## Status

**M2 complete**, with touch routing exercised in unit tests and on a VM; no
physical touchscreen has confirmed it yet. Part of [Denise][Denise] — see the
[repository README][Denise] for the whole picture.

MIT licensed.

[Denise]: https://github.com/bisand/denise
