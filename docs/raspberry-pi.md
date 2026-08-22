# Running Denise on a Raspberry Pi

Everything here was established on a **Raspberry Pi 3 Model A+ running Alpine
Linux** (kernel 6.18, aarch64), driving an HDMI display with no desktop
environment. The numbers are measured, not estimated, and the mistakes described
are ones that were actually made.

## Installing the demo panel

Everything below this line is the reasoning. If you just want the panel on a Pi
running Alpine, there is a script:

```bash
scripts/deploy-pi.sh rpi3b
```

It cross-builds every demo for `aarch64-unknown-linux-musl`, copies them over
with the files in `dist/`, and runs `dist/install.sh` on the far end. The Pi
needs no toolchain — musl links statically with the linker rustc already ships,
which is the whole reason a 900 MHz board never has to compile anything.

The installer is safe to run twice, and it does four things beyond copying
binaries:

- **A splash and a panel service.** `denise-splash` in `sysinit`, `denise` in
  `default`. Which demo runs at boot is `demo=` in `/etc/conf.d/denise`; the
  default is the launcher, which offers the rest as a menu.
- **The boot configuration.** The `vc4-kms-v3d` overlay, `gpu_mem=128`,
  `disable_splash`, `disable_overscan`, `bcm2835-codec` in `/etc/modules`, and
  `console=tty8` so kernel and OpenRC chatter goes to a VT nobody is looking at.
  `/boot/cmdline.txt` is backed up first and rejected if the edit would produce
  more than one line — a two-line `cmdline.txt` is a Pi that boots without half
  its parameters, and the recovery is a card reader.
- **tty1 freed for the panel**, so no login prompt prints over it.
- **tty2-6 through `denise-console`**, a getty wrapper that brings the panel back
  when the session ends.

That last one is what makes the launcher's *Exit to console* button a visit
rather than a one-way door: log out and the panel returns by itself. It wraps
getty rather than using a logout script because `~/.bash_logout` only fires for
bash and `~/.zlogout` only for zsh, and neither fires at all if the shell is
killed rather than exited. init is the thing doing the waiting, so init is the
right place to notice.

Skip the boot half with `--no-boot-config`, or pick the boot demo with
`--demo gallery`.

### Uninstalling, or getting back to a plain console

`/boot/cmdline.txt.before-denise` and `/etc/inittab.before-denise` are the
originals. Putting both back and `rc-update del denise default` leaves a normal
Alpine machine.

Worth knowing before you need it: while the panel is running there is **no local
console at all**. It mutes the console keyboard with `K_OFF`, and that mode
discards keystrokes before the kernel looks for Alt+F-key combos, so VT switching
is dead too. The way out is the *Exit to console* button, or ssh.

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

### The hardware decoder is a separate module

KMS gets you `/dev/dri`. It does not get you `/dev/video*`, which is where
`denise-video` looks for the V4L2 memory-to-memory decoder. Raspberry Pi OS binds
`bcm2835-codec` from the device tree; **Alpine does not**, and the `probe` example
reports nothing until the module is loaded by name:

```bash
doas modprobe bcm2835-codec                      # now
echo bcm2835-codec | doas tee -a /etc/modules    # and at every boot
```

Then `probe` should name a decoder rather than ask you where one is:

```console
$ doas cargo run -p denise-video --example probe
driver           kind               H.264  HEVC   path
bcm2835-codec    stateful           yes    -      /dev/video10
```

`HEVC -` is correct on a Pi 3 and a Pi 4; only the Pi 5 has that, and by the
stateless path this crate does not drive yet.

The decoder's buffers come from GPU memory, not from CMA, so `gpu_mem` matters
here in a way it does not for a UI-only panel. `gpu_mem=128` beside the overlay
line is the usual headroom for 1080p H.264 — the 64 MB default is enough for the
console and leaves little for a decoder.

### The handover blanks the screen, and that part is not yours to fix

The firmware puts a framebuffer up within about 1.3 seconds. vc4 replaces it
around seven, and the replacement reprograms the HDMI pipeline: the sink drops
its TMDS lock and takes a second or two to find it again. Filmed on a Pi 3 with
an NEC panel and read back frame by frame, that is **two seconds of black**, and
the picture was there the whole time — the splash logged itself painting 1.5
seconds before anything appeared on the glass.

Nothing in userspace shortens it. What you can do is not straddle it: hold the
first paint until `/dev/dri/card0` exists, so the handover happens over a black
screen and whatever you are showing appears once. `examples/splash` takes
`--after` for exactly this.

You can also make the handover happen sooner. Left alone, `hwdrivers` coldplugs
USB first — a Logitech unifying receiver takes 1.6 seconds by itself — and vc4
does not bind until 6.8s. Loading it by name before that runs:

```bash
modprobe vc4
```

Measured on the test board: vc4 bound at **5.3s** instead of 6.8s, and the first
paint moved from 7.1s to 6.0s.

### Putting vc4 in the initramfs does not help

It is the obvious next idea and it does not work, so here is the measurement
rather than the theory.

Adding a `vc4` feature to `/etc/mkinitfs/features.d` and rebuilding gets the
module and its DRM dependencies into the image, and Alpine's initramfs then does
not load it: `nlplug-findfs` loads what it needs to find root and stops caring.
Naming it in the kernel command line's `modules=` list does not do it either.
Both were tried; vc4 bound at 6.3s and 5.9s respectively — **worse than the plain
`modprobe`**, because the larger image costs about 0.6s to unpack.

If you try it anyway, keep the working image: `cp /boot/initramfs-rpi
/boot/initramfs-rpi.backup` before rebuilding. `/boot` is FAT, so that backup can
be put back from any machine with a card reader, which is the difference between
a five-minute recovery and a reinstall.

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

## A pointer that is missing entirely is a sleeping one

A wireless mouse that was asleep when the panel started does not merely go
unread — it has no `/dev/input/event*` node at all. The receiver enumerates at
boot and the mouse does not, and the node is created whenever somebody first
moves it. On this Pi, with a Logitech unifying receiver, that was **775 seconds
after boot**:

```text
[   8.498] logitech-hidpp-device 0003:046D:402D.0007: hidraw3: ... [Logitech M560]
[ 783.268] logitech-hidpp-device 0003:046D:402D.0007: HID++ 2.0 device connected.
[ 783.501] input: Logitech Wireless Mouse M560 as .../input19
```

Note that the boot line gives it a `hidraw` node and no `input:` line — the
devices that were awake say `input,hidraw4` instead.

`InputBackend` watches `/dev/input` and opens the node when it appears, so the
mouse starts working the moment it wakes. What it cannot do is repair a loop
holding descriptors from startup: see [the crate's
docs](../denise-evdev/src/lib.rs) on `devices_changed`, or just use
`bare_linux::Waits`, which every kiosk example does.

## A laggy pointer is usually the mouse

Pointer smoothness has a hard floor at the device's report rate, and no renderer
can go below it. Before suspecting the toolkit, measure:

```bash
cargo run -p denise-evdev --example pointer -- 30
```

It opens no display and touches no framebuffer, so whatever it reports is input
and nothing else. Two numbers matter. **`age`** is the gap between the kernel
timestamping an event and this process reading it — under a millisecond means the
loop is keeping up. **`gap`** is the interval between reports, which is the
hardware's own rate.

Measured on the Pi 3 A+, over 1849 events:

| Device | `gap` | Rate | `age` p50 / p95 / max |
|---|---|---|---|
| Logitech K400 (wireless keyboard + touchpad) | 16.00 ms | 62.5 Hz | 0.04 / 0.05 / 0.08 ms |

Input arrives in **40 microseconds** — four hundred times faster than a frame —
and the reporting interval is dead steady at 16 ms. The USB side agrees: the
receiver's interrupt endpoint has a `bInterval` of 8 ms, so the bus asks twice as
often as the K400 has anything to say. The 62.5 Hz is the touchpad.

That floor is what a laggy pointer feels like. 16 ms of input granularity plus up
to one frame of page-flip latency is about 33 ms before a movement reaches the
glass — with everything working correctly. Swapping in a wired mouse makes it
visibly snappier, which is the confirmation worth doing before writing any code.

Two things this rules *out*, both worth knowing:

- **CPU throttling.** Four busy cores for 25 seconds held 1400 MHz and 50 °C. The
  undervoltage flag does assert under that load — worth a better supply, since
  undervoltage corrupts SD cards — but the firmware never cut the clock, and no
  undervoltage appears under real rendering at all.
- **The render path.** While the panel ran, the CPU never rose above 900 MHz of
  its 1400 MHz, it drew one frame per input event, and it repainted 1.9% of the
  surface per frame. It was idling, waiting for the pointer.

### The hardware cursor plane, measured

vc4 has a cursor plane, and using it removes the page flip from pointer movement
entirely. The same panel, the same pointer, moved continuously in both runs:

| | Input events | Frames | Frames per pointer report |
|---|---|---|---|
| Sprite composited into the buffer | 355 | 361 | **1.02** |
| Hardware cursor plane | 963 | 68 | **0.07** |

A fourteenfold drop. The software path drew a frame and flipped a page for every
pointer report; the hardware path draws almost nothing.

The remaining 68 are not pointer frames. The caret blinks at 2 Hz, which accounts
for 40 of them over 20 seconds, and the rest are hover states lighting up as the
pointer crosses buttons — repaints that should happen.

The wake-up counts show the mechanism: **1023 wake-ups for 68 frames**. The loop
still wakes on every report, because it has to issue the move ioctl, and then goes
straight back to sleep without touching the framebuffer.

Two things this does not fix, and one trap:

- **The 16 ms report interval is untouched.** The plane removes a frame of flip
  latency, which on a 62.5 Hz device is the smaller half of the delay.
- **Only the pointer is free.** Anything else that animates still costs a normal
  frame, which is correct.
- **Move the plane before the repaint decision.** A pointer move now damages
  nothing, so `needs_paint` is false and the loop is about to sleep. Moving the
  cursor after that check means it only follows the hand when something else
  happens to redraw — which looks exactly like the bug the plane was meant to
  fix.

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

## Keyboard layout

evdev reports key *positions*; what they type is a property of the layout. Denise
reads the system's choice, which on Alpine lives in `/etc/conf.d/loadkmap`:

```console
$ cat /etc/conf.d/loadkmap
KEYMAP=/etc/keymap/no.bmap.gz

$ /tmp/panel
keymap  no (from /etc/conf.d/loadkmap)
```

`DENISE_KEYMAP=no` overrides it, and `F3` cycles layouts at runtime.

If the panel reports a layout you did not expect, `examples/keys` shows exactly
what each key produces — the position on one line, the composed character
indented under it:

```console
$ /tmp/keys 30
key   Semicolon
  --> text 'ø'  U+00F8
```

The position is a fact about the hardware and the character is a fact about the
layout, so a wrongly-labelled keyboard and a wrong keymap stop looking alike.

Two things that look like faults and are not. A dead key prints a position and no
text at all until the next key resolves it. And **the labels on the keys are not
consulted**: with a Norwegian layout on a physically English keyboard, `ø` is on
the key printed `;`, `æ` on `'`, and `å` on `[`.

## The on-screen keyboard

A panel with nothing plugged into it still has to be typed on: the browser's URL
bar is the only way to a page, and the table editor is a form. `denise-keyboard`
slides a keyboard up from the bottom and emits exactly what evdev would have —
`Key` down, the composed `Text`, `Key` up — so nothing downstream can tell the
difference.

It reads the same layout the hardware path reads, from the same files, so a board
configured Norwegian comes up Norwegian without being told. Verified on a Pi 3 A+
running Alpine, whose `/etc/conf.d/loadkmap` names `/etc/keymap/no.bmap.gz`: the
home row came up `ø æ`, and the layout key said `no`.

```console
$ /tmp/denise-browser --keyboard --size 1920x1080 --snapshot /tmp/kbd.ppm
```

`--keyboard` puts the caret in a field at startup, which is what brings the
keyboard up — there is no separate switch, because focus is the trigger. The
browser, the gallery and the table editor all take it.

Three things worth knowing before it is on a wall:

- **It is 330 logical pixels tall**, six rows of 48. That is a third of a 1080p
  panel and two thirds of an 800×480 one, so on a short display expect it to be
  the larger half of the screen.
- **`--scale` applies to it.** The grid is written in logical pixels like every
  other layout in these examples, so a 2× panel gets fingertip-sized keys rather
  than 48 device pixels of them.
- **Escape is the application's.** A shelf pushes no scene — which is what lets
  the field underneath keep its caret — so nothing in the tree closes the
  keyboard for you.
- **Holding Backspace keeps deleting**, and no other key repeats. It costs
  nothing when nobody is touching it: the key asks to be woken only between its
  press and its release, so an idle panel with the keyboard on screen schedules
  no wakes at all.

## Taking a screenshot with no desktop

There is no screenshot tool, because there is no compositor to ask and no window
server to ask it of. Three ways round that, best first.

**Ask the program.** `panel` writes `/tmp/denise-panel.ppm` on **F12**. It captures
the scanout buffer after painting and before presenting, so what lands in the file
is exactly what the display is about to show, cursor sprite included — a capture,
not a re-render that might differ. No root, no extra package, and it works
identically on DRM and on fbdev.

```bash
# on the Pi, with the panel running: press F12, then
pnmtopng /tmp/denise-panel.ppm > shot.png     # netpbm
# or from your own machine
scp pi@raspberrypi:/tmp/denise-panel.ppm . && magick denise-panel.ppm shot.png
```

Twenty lines of any application can do the same thing; `capture` in
[examples/panel](../examples/panel/src/main.rs) is the whole of it. A panel in the
field that can mail you a picture of what the operator is looking at is worth
rather more than a screenshot key on a desktop.

**Render one without a display at all.** Several examples take `--snapshot
out.ppm`, which draws one frame and exits. Deterministic, needs no Pi, and diffs
cleanly between two builds — but it shows what the tree *would* draw, not what is
on the screen right now.

**Grab the scanout from outside.** `ffmpeg`'s `kmsgrab` reads the active KMS plane
directly:

```bash
sudo ffmpeg -f kmsgrab -i - -frames:v 1 -vf hwdownload,format=bgr0 shot.png
```

It needs `CAP_SYS_ADMIN` and a DRM master that is not exclusive, so it will not
always cooperate with an application that has taken the display — which is most of
the point of this toolkit. `cat /dev/fb0` has the same problem in reverse: with the
vc4 KMS driver `/dev/fb0` is an emulation, and while a DRM client holds the CRTC it
often shows the console underneath rather than what is on screen. Try them, but
believe the first method over either.

## Known gaps

- **Touch is still unverified on hardware, and now there is something to verify
  it with.** The multitouch slot path is unit tested, and every key of the
  on-screen keyboard is a `Button`, which handles `TouchDown`/`TouchUp` through
  the same path as pointer buttons — so there is nothing left to *write*, only
  something to try. What remains, precisely: attach a touchscreen, check it
  appears in `/proc/bus/input/devices` with `ABS_MT_SLOT`, run
  `denise-browser --keyboard`, and confirm that a tap on a key types one
  character rather than none or two, and that a tap on the URL bar summons the
  keyboard the way a click does. Neither test board has a touchscreen attached;
  the Pi 3 A+ used for the layout check above had a keyboard and a mouse and
  nothing else.

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

### The one crate that wants a C compiler

Exactly one thing in the demo set breaks that rule: the browser's `tls` feature,
which pulls rustls, which pulls ring, whose build compiles C. rustc ships a
linker, not a compiler, so this is the one build a bare `cargo build --target`
cannot finish.

It stops short of needing a cross toolchain, though. ring compiles its C with
`-nostdlibinc` and reads no libc headers at all, so a compiler that can *emit*
aarch64-linux code is the whole requirement — and clang is a cross compiler by
construction. Two more details and it links:

```bash
rustup component add llvm-tools
LLVM_BIN="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin"

CC_aarch64_unknown_linux_musl=clang \
CFLAGS_aarch64_unknown_linux_musl="--target=aarch64-unknown-linux-musl -U__musl__" \
AR_aarch64_unknown_linux_musl="$LLVM_BIN/llvm-ar" \
cargo build --release --target aarch64-unknown-linux-musl -p browser \
    --no-default-features --features kiosk,tls
```

The result is the same 6.5 MB static binary as every other demo, `https:`
included, built on macOS with nothing installed beyond the Xcode command line
tools that a Rust developer on a Mac already has.

The two details are each one small surprise:

- **`-U__musl__`.** Apple's `clang` hands `<stddef.h>` straight to the system
  header whenever it sees a musl target — and a cross build is exactly the case
  with no musl headers on disk. Undefining the macro puts clang back on its own
  copy. Elsewhere the macro is not set and undefining it does nothing.
- **`llvm-ar`.** The `ar` on macOS writes an archive that rust-lld reads as
  containing no symbols, so the link fails on every function ring's C provides,
  with nothing to suggest the archiver is at fault. rustup's `llvm-tools`
  component has one that works.

`scripts/deploy-pi.sh` does all of this by itself: it prefers a real
`aarch64-linux-musl-gcc` if one is installed, falls back to the clang route,
adds the `llvm-tools` component if it is missing, and only if none of that is
available builds the browser without `tls` — saying so rather than shipping a
quietly crippled demo. `DENISE_TLS=0` asks for that build deliberately.

Why it matters for the panel rather than being a footnote: the browser's welcome
page links to `https://example.com`, its search box posts to DuckDuckGo over
https, and the URL bar puts `https://` in front of anything typed bare. Without
TLS almost nothing in the demo reaches a page.
