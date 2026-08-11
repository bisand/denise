# Running Denise on a Raspberry Pi

Everything here was established on a **Raspberry Pi 3 Model A+ running Alpine
Linux** (kernel 6.18, aarch64), driving an HDMI display with no desktop
environment. The numbers are measured, not estimated, and the mistakes described
are ones that were actually made.

## The symptom: no `/dev/dri`

A fresh Pi very often has no DRM device at all:

```console
$ ls /dev/dri/
ls: /dev/dri/: No such file or directory

$ cat /sys/class/graphics/fb0/name
BCM2708 FB
```

`BCM2708 FB` is the **firmware framebuffer**: the GPU's boot-time display, handed
to Linux as a plain memory region. It has no page flip, no vblank and no vsync.
Denise will still run — [`denise-fbdev`](../denise-fbdev) exists precisely for
this — but every frame can tear, and the display cannot pace the render loop.

This is *not* the same thing as `/dev/fb0` on a modern desktop, which is usually
DRM's fbdev emulation. On a Pi without the overlay below, there is no DRM
underneath at all.

## The fix: enable the vc4 KMS driver

The Pi's real display driver is `vc4`, and it is not enabled by default. It needs
a device-tree overlay.

### Check the prerequisites first

If the module or the overlay is missing, enabling it can leave the machine with
no display. Check before rebooting:

```bash
find /lib/modules/$(uname -r) -name 'vc4*' -o -name 'v3d*'
ls -l /boot/overlays/vc4-kms-v3d.dtbo
grep -E 'MemTotal|CmaTotal' /proc/meminfo
```

You want `vc4.ko` present, the `.dtbo` present, and at least ~64 MB of CMA. A
1920×1080 32-bit framebuffer is 8 MB, and double buffering needs two, so 64 MB is
ample even on a 512 MB board.

### Where the line goes

**Alpine Linux** ships a `config.txt` ending in `include usercfg.txt`. Put changes
in `usercfg.txt` so a package update to `config.txt` cannot wipe them:

```bash
doas sh -c 'echo dtoverlay=vc4-kms-v3d >> /boot/usercfg.txt'
doas reboot
```

**Raspberry Pi OS** has no `usercfg.txt`; add the line to `/boot/firmware/config.txt`
directly (older releases: `/boot/config.txt`).

Do not use `vc4-fkms-v3d`. The "fake KMS" variant is deprecated and keeps the
firmware in the display path.

### Verify

```console
$ ls /dev/dri/
card0  renderD128

$ cat /sys/class/graphics/fb0/name
vc4drmfb
```

`card0` present and the framebuffer renamed to `vc4drmfb` means KMS is live.
Denise's backend selection needs no configuration — the demos try DRM first and
fall back to fbdev, so they switch over on their own.

### If the screen goes black

**SSH keeps working**, because none of this affects networking. Remove the line
and reboot:

```bash
doas sed -i '/vc4-kms-v3d/d' /boot/usercfg.txt
doas reboot
```

On a 512 MB board that will not come up, try reserving CMA explicitly:
`dtoverlay=vc4-kms-v3d,cma-64`.

## What changes when you enable it

### The resolution will change

The firmware framebuffer applies overscan compensation; KMS negotiates the
display's preferred mode from its EDID. On the test machine 1824×984 became
1920×1080. That is the panel's real native mode, and the earlier figure was the
firmware trimming the edges.

### Page flips become real, and block until vblank

This is the important one, and it is easy to verify. Denise's DRM smoke test runs
a loop with *no* frame limiter of any kind:

```console
$ cargo run -p denise-drm --example smoke -- 4
mode 1920x1080@60 — 2 buffers, stride 1920 px for 1920 px of width
241 frames in 4.00s = 60.2 fps
```

**60.2 fps on a 60 Hz mode with no limiter** means the flip genuinely blocks until
vblank. The same binary against `virtio-gpu` in a VM reports **5839 fps**, because
virtualised GPUs retire flips the instant the host acknowledges them and never
pace the caller at all.

That difference matters when you are testing: **a VM cannot tell you anything
about frame pacing.** Correctness, mode selection, buffer ages and input all check
out in a VM. Timing does not.

## Async page flips: keep KMS, drop the vblank wait

vc4 advertises `DRM_CAP_ASYNC_PAGE_FLIP`, so Denise can flip **immediately**
instead of at the next vblank. That removes the latency described below while
keeping everything KMS gives you — proper mode setting, real buffer ages, a
restored console on exit. It is [`PresentMode::Immediate`], and it is the default.

```bash
cargo run -p kiosk               # immediate: the default
cargo run -p kiosk -- 20 250 vsync   # tear-free, for comparison
```

On the test machine `immediate` felt clearly better than `vsync` and about the
same as the old tearing fbdev path, which is the point: the latency was the vblank
wait, not anything in the software.

The cost is a horizontal seam where the panel switched buffers mid-scan. With
damage tracking a typical update is a few thousand pixels, so the seam is small
and brief. Reconsider for signage or large fast-moving content.

Drivers without the capability fall back to vsync silently;
`DrmSurface::present_mode` reports what was actually obtained, so log it rather
than assume.

### Immediate mode does not pace your loop

Under vsync, `acquire` blocks until the previous flip retires, so
`loop { acquire; draw; present }` runs at exactly the refresh rate for free.
Under immediate, **nothing waits**, and that same loop will use a whole core
drawing frames nobody sees.

Draw only when something changed — damage tracking makes that natural — or keep a
frame deadline of your own. `examples/kiosk` does both. This is not a defect in
async flips; it is what removing the wait means.

## Tear-free costs latency, and it is not subtle

Measured on the same machine, same demo, same movements, with a Logitech K400:

| | fbdev (no vsync) | DRM/KMS (vsync) |
|---|---|---|
| queued, hardware → read | 0.04 ms | 6.20 ms |
| input → pixels in buffer | 0.17 ms | 16.64 ms |
| tearing | yes | none |

The vsync DRM path was **visibly draggier** to the person operating it. That is
not a defect: a flip queued after a vblank cannot land before the next one, so
double-buffered tear-free presentation costs on the order of one refresh period.
Every system that does this pays it.

Which is why [`PresentMode::Immediate`] is the default — it keeps KMS and drops
the wait. Choose `Vsync` deliberately, for content where a seam would show.

### Read input *after* waiting for the display

Half of the latency above was self-inflicted, and it is worth avoiding in any loop
you write. `Surface::acquire` blocks until the previous flip retires. A loop that
reads input *before* calling it draws a position that has aged by however long the
wait took — 6.2 ms at p50, 15 ms at p95 on this hardware.

Wrong:

```text
poll for input → read input → acquire (blocks to vblank) → render → present
```

Right:

```text
poll for input → acquire (blocks to vblank) → read input → render → present
```

`examples/kiosk` does the latter. It costs nothing and removes up to a full
refresh period of staleness.

## Do not cap the frame rate near the input rate

A frame cap set close to the pointer's report rate is worse than no cap. The K400
reports every 16 ms; a 60 Hz cap runs at almost exactly that rate, so the two
drift in and out of phase and each event waits a uniformly random slice of a frame
for nothing. Measured: 6.8 ms median latency at a 60 Hz cap against 0.16 ms at
250 Hz, with **identical frame counts**, because frames were bound by the input
rate rather than by the cap.

Treat any cap as a runaway guard set far above every plausible input rate, not as
a schedule. On DRM it is redundant anyway — vblank is the schedule.

## Performance on a Pi 3 A+

From `cargo run -p denise-benches --bin on-target`, at 1824×984:

```text
clear                     455 Mpx/s   23.7% of a 60 Hz frame
fill_rect opaque          457 Mpx/s   23.6%
fill_rect alpha           170 Mpx/s   63.4%
rounded_rect fill r=12    449 Mpx/s   24.0%
rounded_rect stroke        31 Mpx/s    2.2%
line antialiased           31 Mpx/s    0.7%
scene, full repaint       187 Mpx/s   57.6%
scene, damage only        147 Mpx/s    0.6%
```

Three things follow:

- Solid fills run at 455 Mpx/s, which is 1.8 GB/s of writes — roughly what this
  board's LPDDR2 can sustain. The fill path is **memory bound, not compute bound**,
  so SIMD would buy nothing.
- The per-pixel path is **fifteen times slower** than the span path (31 vs 457
  Mpx/s). Strokes and anti-aliased lines go through it. That is the obvious place
  to optimise when text arrives, since glyph coverage uses the same path.
- Damage costs **98× less** than redrawing everything: 0.6% of a core against
  57.6%. That is the premise of the project, measured on the target.

Full-screen alpha is 63% of a frame budget, so a dimmed modal backdrop must be
damage-clipped rather than painted over the whole screen.

## Permissions

Neither DRM nor evdev needs root:

- `/dev/dri/card0` is `root:video` — add the user to **`video`**.
- `/dev/input/event*` is `root:input` — add the user to **`input`**.

DRM master works over SSH provided nothing else holds it. A compositor running on
the machine will take it and Denise will fail with `EBUSY`; stop the compositor or
use a bare VT.

## Known gaps

- **The VT keyboard is not muted** while Denise holds DRM master, so keystrokes
  still reach the shell behind the UI. Real kiosks mute it with `KDSKBMODE`/`K_OFF`.
- **Touch is unverified on hardware.** The multitouch slot path is unit tested but
  no physical touchscreen has driven it.

## Cross-compiling to the Pi from another machine

Rust ships musl's `libc.a` and startup objects, so a fully static binary links with
rustc's bundled lld and needs no cross toolchain at all — including from macOS:

```bash
rustup target add aarch64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl -p kiosk
scp target/aarch64-unknown-linux-musl/release/kiosk pi:/tmp/
```

`.cargo/config.toml` in this repository already sets `linker = "rust-lld"` for the
musl targets. A musl-linked static binary runs on glibc distributions such as
Raspberry Pi OS as well; nothing on the target needs installing.

Note that `/tmp` is usually tmpfs, so binaries copied there do not survive a
reboot.
