---
title: Raspberry Pi
description: Running a DeniseUI kiosk build on a Raspberry Pi with no desktop, from enabling KMS to cross-compiling, input, deployment, screenshots and the on-screen keyboard.
---

Everything here was established on a **Raspberry Pi 3 Model A+ running Alpine Linux** (aarch64), driving an HDMI display with no desktop environment. The long version, including the measurements and the mistakes, is [docs/raspberry-pi.md](https://github.com/bisand/denise/blob/main/docs/raspberry-pi.md).

## First: enable the vc4 KMS driver

A stock Pi very often has no DRM device at all:

```text
$ ls /dev/dri/
ls: /dev/dri/: No such file or directory

$ cat /sys/class/graphics/fb0/name
BCM2708 FB
```

`BCM2708 FB` is the firmware framebuffer: no page flip, no vblank, no vsync. Denise will still run on it, but every frame can tear and the display cannot pace the loop. The Pi's real display driver is `vc4`, and it needs a device-tree overlay. That one line decides whether you get real page flips.

Check the prerequisites before rebooting, because a missing module can leave the machine with no display:

```bash
find /lib/modules/$(uname -r) -name 'vc4*' -o -name 'v3d*'
ls -l /boot/overlays/vc4-kms-v3d.dtbo
grep -E 'MemTotal|CmaTotal' /proc/meminfo
```

You want `vc4.ko`, the `.dtbo`, and at least about 64 MB of CMA. On **Alpine**, put the line in `usercfg.txt` so a package update to `config.txt` cannot wipe it:

```bash
doas sh -c 'echo dtoverlay=vc4-kms-v3d >> /boot/usercfg.txt'
doas reboot
```

On **Raspberry Pi OS**, add `dtoverlay=vc4-kms-v3d` to `/boot/firmware/config.txt` (older releases: `/boot/config.txt`). Do not use `vc4-fkms-v3d`; the fake-KMS variant is deprecated and keeps the firmware in the display path.

After the reboot, `/dev/dri/card0` exists and `fb0` is named `vc4drmfb`. The resolution may change too: KMS uses the display's preferred mode rather than the firmware's overscan-trimmed one. If the screen goes black, SSH still works; remove the line with `sed` and reboot.

## DRM, and fbdev as the fallback

`denise-drm` opens the display directly, sets a mode, and page-flips CPU-rendered dumb buffers. There is no `gbm`, no Mesa and no compositor, which is what keeps the result one static binary. `denise-fbdev` is the fallback for kernels with no usable DRM driver, and the demos try DRM first and fall back on their own.

Setting a mode requires being **DRM master**, and only one process can be. If a compositor holds it, opening fails with `EBUSY` or `EACCES` — the most common reason a first run does nothing. Run on a bare VT, be handed a file descriptor by `libseat` or systemd, or, least good, run as root. Neither DRM nor evdev needs root otherwise: add the user to the `video` and `input` groups.

### Vsync, immediate, and what a tear costs

`PresentMode::Vsync` is the default: flips wait for vblank, so tearing is impossible and the loop is paced for free. On the Pi 3 A+ the vc4 driver's async flips tore visibly enough to read as flicker, and a panel is read rather than aimed at.

`PresentMode::Immediate` asks for the latency back, and it is not a small amount: measured on the same machine, input to pixels was 0.17 ms without vsync and 16.64 ms with it. Immediate mode follows the damage rather than applying blindly — damage covering under a quarter of the screen's rows flips at once, and anything taller waits for vblank, so a scrolling viewport is still paced. The gallery takes either:

```bash
denise-gallery --present immediate    # async flip, tearing possible
denise-gallery --present vsync        # paced by vblank, tearing impossible
```

If vsync does not cure a flicker, the flip was never involved and something is repainting when nothing changed — which is a bug worth reporting.

vc4 has a hardware cursor plane, and `denise-drm` uses it: moving the pointer becomes one ioctl with no repaint and no page flip. In a measured run that took frames per pointer report from 1.02 to 0.07.

## Cross-compiling

The Pi never compiles anything. Rust ships musl's `libc.a`, and the repository's `.cargo/config.toml` sets `linker = "rust-lld"` for the musl targets, so a static binary builds from any machine, macOS included, with no cross toolchain:

```bash
rustup target add aarch64-unknown-linux-musl
cargo build -p hello --no-default-features --features kiosk \
    --release --target aarch64-unknown-linux-musl
scp target/aarch64-unknown-linux-musl/release/hello pi:/tmp/
```

A musl static binary runs on glibc distributions such as Raspberry Pi OS as well. One demo breaks the no-compiler rule: the browser's `tls` feature pulls in `ring`, which compiles C. The long document shows the clang route; the deploy script below handles it for you.

## Installing the demo panel

On a Pi running Alpine, one script cross-builds every demo, copies them over, and runs `dist/install.sh` on the far end. The host is anything ssh understands:

```bash
scripts/deploy-pi.sh rpi3b
scripts/deploy-pi.sh rpi3b --no-boot-config    # binaries and services only
scripts/deploy-pi.sh rpi3b --demo gallery      # boot straight into one demo
DENISE_TLS=0 scripts/deploy-pi.sh rpi3b        # browser without https
```

Beyond copying binaries, the installer adds a splash and a panel service (the default boot demo is the launcher), sets the boot configuration — the `vc4-kms-v3d` overlay, `gpu_mem=128`, and kernel chatter moved to `tty8` — frees `tty1` for the panel, and wraps the other consoles so logging out brings the panel back. It backs up `/boot/cmdline.txt` and `/etc/inittab` first, as `.before-denise` copies.

Worth knowing before you need it: **while the panel runs there is no local console.** It mutes the console keyboard, and VT switching goes with it. The way out is the launcher's *Exit to console* button, or SSH.

## Input

`denise-evdev` reads mice, touchscreens and keyboards from `/dev/input/event*` and produces `denise::InputEvent`s:

```rust
use denise::{InputSource, Size};
use denise_evdev::InputBackend;

// The surface size is what absolute touch coordinates are scaled into.
let mut input = InputBackend::open_all(Size::new(1280, 800))?;

let mut events = Vec::new();
input.poll(&mut events);      // never blocks
```

To sleep rather than spin, wait on `InputBackend::raw_fds` together with the display's descriptor. That list changes: a wireless mouse that was asleep at boot has no device node until someone moves it, so ask `devices_changed()` each pass and take the descriptors again.

### Muting the console

Reading evdev does not stop the login shell behind the panel reading the same keystrokes, so text typed into a field is also typed at the shell. `Console::mute_keyboard` switches the console keyboard off while evdev still sees everything, and pairs it with graphics mode so the console stops blanking and repainting. The guard restores the mode it read on drop, including during a panic, and refuses to mute a pty, so an SSH session is never the one muted. Nothing restores it after `SIGKILL`; the escape hatch over SSH is `kbd_mode -u -C /dev/tty1`.

### Keyboard layouts

A `KeyCode` is a position; what it types depends on the layout. `denise-layout` reads the machine's choice from `DENISE_KEYMAP`, then `XKB_DEFAULT_LAYOUT`, then the console keymap files distributions write — on Alpine, `/etc/conf.d/loadkmap`. It ships US, Norwegian and German tables with dead keys and AltGr. The non-US AltGr assignments are a reconstruction that wants checking against real hardware, and a system asking for a layout there is no table for falls back to US, visibly. In the `panel` example, F3 cycles layouts; `cargo run -p denise-evdev --example keys` shows what each key produces.

## Screenshots with no desktop

There is no compositor to ask, so ask the program. The `panel` example writes its scanout buffer to `/tmp/denise-panel.ppm` on **F12**, after painting and before presenting — exactly what the display is about to show, cursor included, on DRM and fbdev alike.

```bash
scp pi@raspberrypi:/tmp/denise-panel.ppm . && magick denise-panel.ppm shot.png
```

`--snapshot out.ppm` on the examples renders a frame with no display at all, which is deterministic but shows what the tree would draw rather than what is on the glass. Grabbing from outside is less reliable: `ffmpeg`'s `kmsgrab` needs `CAP_SYS_ADMIN` and may not cooperate with an application holding the display, and with vc4 KMS, `/dev/fb0` is an emulation that often shows the console underneath rather than the panel.

## The on-screen keyboard

A panel with nothing plugged into it still has to be typed on. `denise-keyboard` is a shelf of keys that slides up from the bottom and emits exactly what evdev would — `Key` down, the composed `Text`, `Key` up — so nothing downstream can tell the difference. Each key is a button that never takes focus, and a shelf pushes no scene, so the field keeps its caret.

```rust
use denise_keyboard::Keyboard;

let (keyboard, source) = Keyboard::from_system();
let mut keyboard = keyboard.with_scale(scale);

// Once a frame: open when a text field has focus, close when it does not.
keyboard.follow_focus(&mut ui, Msg::Key);

// The keyboard sees pointer and touch events before the tree does.
let typed = keyboard.handle(&mut ui, &events);
ui.handle(&typed);
ui.handle(&events);
```

A message from a key goes back through `keyboard.press(code)`, and `Keyboard::tick` delivers Backspace's repeats. Things worth knowing before it goes on a wall:

- It is **276 logical pixels tall**, five rows of 48 — a quarter of a 1080p panel and over half of an 800×480 one. `with_scale` applies the display's factor.
- **Escape is the application's.** Nothing in the tree closes the keyboard for you.
- **Holding a letter offers its alternates** from the layout, so a board configured Norwegian offers what a Norwegian writer reaches for. Only Backspace repeats.
- It costs nothing when nobody touches it: a key asks to be woken only between its press and its release.
- The layout key walks `us`, `no` and `de`, relettering keys where they stand.

The browser, the gallery and the table editor all take `--keyboard`, which puts the caret in a field at startup so the keyboard comes up.

## Touch is unverified on hardware

The multitouch slot path is unit tested, and a single touch routes to widgets as a pointer would, but **no physical touchscreen has driven it**. The on-screen keyboard is the thing to verify it with: attach a touchscreen, confirm it appears in `/proc/bus/input/devices` with `ABS_MT_SLOT`, run `denise-browser --keyboard`, and check that a tap types one character rather than none or two. The hold-and-slide alternates gesture is the part most worth trying on real glass.

API reference: [denise-drm](https://docs.rs/denise-drm), [denise-evdev](https://docs.rs/denise-evdev), [denise-keyboard](https://docs.rs/denise-keyboard), [denise-layout](https://docs.rs/denise-layout).
