# denise

Core of [Denise](https://github.com/bisand/denise), a direct-rendering UI toolkit
for embedded Linux and systems without a desktop environment — kiosks, digital
signage, industrial HMIs, Raspberry Pi panels.

This crate is platform-agnostic: geometry, colour, the pixel buffer contract, input
events and dirty-rectangle tracking. It contains no platform code and no `unsafe`,
and builds `no_std + alloc`. Backends live in separate `denise-*` crates and are
selected at build time, so an embedded target never compiles desktop code.

```rust
use denise::{DamageTracker, Rect, Surface};

# fn demo(surface: &mut impl Surface, tracker: &mut DamageTracker) -> Result<(), denise::SurfaceError> {
let mut frame = surface.acquire()?;
// The buffer just acquired may be several frames old; widen the damage to match.
let damage: &[Rect] = tracker.resolve(frame.age());
// ... draw, clipped to `damage` ...
drop(frame);
surface.present(damage)?;
tracker.end_frame();
# Ok(())
# }
```

**Status: 0.0.0, M0.** The `Surface` and `InputSource` traits and damage tracking
work and are tested. There is no scene graph, component model, rasteriser, text
support or hardware backend yet. Not usable for applications. See the
[repository README](https://github.com/bisand/denise) for the roadmap.

MIT licensed.
