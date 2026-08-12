//! Opening a display, input and the console on a Linux machine with no desktop.
//!
//! Every bare-Linux example needs the same four things before it can draw a
//! pixel, and none of them is interesting:
//!
//! 1. A display — DRM/KMS if the machine has it, fbdev if it does not.
//! 2. Input — every evdev device that looks like a keyboard, mouse or touchscreen.
//! 3. The console muted, so keystrokes stop reaching the shell behind the panel.
//! 4. A timeout for `poll`, so the loop sleeps rather than spins.
//!
//! This is those four. It is an example support crate, not part of the library:
//! nothing here is published and nothing in `denise` depends on it.
//!
//! # What it deliberately does not do
//!
//! **There is no `run` function.** The event loop stays in the application,
//! because where input is read relative to the display wait is a decision with a
//! measurable cost — on a Pi, reading before `acquire` rather than after it put
//! 6.2 ms of queuing at p50 into every pointer position. A helper that owned the
//! loop would make that decision for you, silently, and it is not a helper's
//! decision to make. See `examples/panel`, which explains its own ordering.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use denise::{CursorPlane, PixelFormat, Rect, Size, Surface, SurfaceError};
use denise_drm::{DrmSurface, PresentMode, SurfaceConfig};
use denise_evdev::layout::Layout;
use denise_evdev::{Console, EvdevError, InputBackend};
use denise_fbdev::FbdevSurface;
use rustix::event::Timespec;

/// The display, kept concrete so the hardware cursor plane stays reachable.
///
/// `Box<dyn Surface>` would have been shorter and would have thrown the plane
/// away with the type: a cursor plane is not part of the `Surface` contract,
/// because most backends have nothing to implement it with.
// One of these exists per process and lives for the whole run, so the variants
// differing in size costs a few hundred bytes once. Boxing to even them out would
// trade that for a pointer chase on every `acquire`.
#[allow(clippy::large_enum_variant)]
pub enum Display {
    /// Real modesetting: page flips, buffer age, a cursor plane.
    Drm(DrmSurface),
    /// The fallback: one linear buffer, no flip, so it tears.
    Fbdev(FbdevSurface),
}

impl Display {
    /// Opens the best display this machine has, reporting which on stderr.
    ///
    /// The fallback is not a formality. A stock Raspberry Pi has no `/dev/dri` at
    /// all until the vc4 KMS overlay is enabled, and the difference between the
    /// two lines this prints is the difference between page flips and tearing —
    /// which is worth saying out loud rather than leaving somebody to wonder.
    pub fn open(present_mode: PresentMode) -> Result<Self, String> {
        match DrmSurface::open(SurfaceConfig {
            present_mode,
            ..SurfaceConfig::default()
        }) {
            Ok(drm) => {
                eprintln!(
                    "display DRM/KMS {} — {} buffers, {}",
                    drm.mode_name(),
                    drm.buffer_count(),
                    match drm.present_mode() {
                        PresentMode::Vsync => "vsync: tear-free, paced by vblank",
                        PresentMode::Immediate => "immediate: async flips, no vblank wait",
                    }
                );
                Ok(Display::Drm(drm))
            }
            Err(drm_error) => match FbdevSurface::open_first() {
                Ok(fb) => {
                    eprintln!("display fbdev {} ({})", fb.info(), fb.path().display());
                    eprintln!("        no DRM: {drm_error}");
                    Ok(Display::Fbdev(fb))
                }
                Err(fb_error) => Err(format!("no display — DRM: {drm_error}; fbdev: {fb_error}")),
            },
        }
    }

    /// The hardware cursor plane, if this display has one.
    ///
    /// Worth using where it exists: on a Pi it took the pointer from 1.02 frames
    /// per report down to 0.07, because moving a cursor plane is a register write
    /// rather than a repaint.
    pub fn cursor_plane(&mut self) -> Option<&mut dyn CursorPlane> {
        match self {
            Display::Drm(drm) => Some(drm),
            Display::Fbdev(_) => None,
        }
    }
}

impl Surface for Display {
    fn size(&self) -> Size {
        match self {
            Display::Drm(d) => d.size(),
            Display::Fbdev(f) => f.size(),
        }
    }
    fn scale_factor(&self) -> f32 {
        match self {
            Display::Drm(d) => d.scale_factor(),
            Display::Fbdev(f) => f.scale_factor(),
        }
    }
    fn format(&self) -> PixelFormat {
        match self {
            Display::Drm(d) => d.format(),
            Display::Fbdev(f) => f.format(),
        }
    }
    fn acquire(&mut self) -> Result<denise::Frame<'_>, SurfaceError> {
        match self {
            Display::Drm(d) => d.acquire(),
            Display::Fbdev(f) => f.acquire(),
        }
    }
    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError> {
        match self {
            Display::Drm(d) => d.present(damage),
            Display::Fbdev(f) => f.present(damage),
        }
    }
}

/// Opens every input device and reports what was found.
///
/// Returns the layout that was chosen and where it came from, which is worth
/// printing: a layout set only by an environment variable is one somebody forgets
/// to set, and the symptom — a Norwegian key typing a semicolon — reads as a
/// broken keyboard rather than a wrong setting.
pub fn open_input(size: Size) -> Result<(InputBackend, &'static Layout), EvdevError> {
    let mut input = InputBackend::open_all(size)?;
    let (keymap, source) = input.set_layout_from_system();
    for device in input.devices() {
        eprintln!("input   {}: {}", device.capabilities(), device.name());
    }
    eprintln!("keymap  {} (from {source})", keymap.name);
    Ok((input, keymap))
}

/// Mutes the console keyboard, returning the guard that restores it.
///
/// Hold the return value for the whole run: dropping it puts the console back
/// exactly as it was. Without this every character typed into a Denise text field
/// is *also* typed at the login shell behind the panel, which is invisible until
/// somebody quits and finds a command line full of their password.
///
/// Returns `None` over SSH, where there is no console to take, and on
/// `DENISE_KEEP_CONSOLE`, which is the escape hatch for anyone debugging.
pub fn mute_console() -> Option<Console> {
    if std::env::var_os("DENISE_KEEP_CONSOLE").is_some() {
        eprintln!("console left alone (DENISE_KEEP_CONSOLE)");
        return None;
    }
    match Console::open_if_present() {
        Some(mut console) => {
            let result = console
                .mute_keyboard()
                .and_then(|()| console.graphics_mode());
            match result {
                Ok(()) => eprintln!("console muted — restored on exit"),
                Err(e) => eprintln!("console found but not muted: {e}"),
            }
            Some(console)
        }
        None => {
            eprintln!("console none (over SSH, so nothing to mute)");
            None
        }
    }
}

/// How long `poll` may block: until something wants waking, or the run ends.
///
/// `next_wake` is [`Ui::next_wake_ms`](denise_ui::Ui::next_wake_ms). `None` means
/// nothing is animating, and then the only limit is the deadline — which is what
/// makes an idle panel cost nothing at all rather than a timer's worth of
/// wake-ups per second.
pub fn poll_timeout(next_wake: Option<u64>, now_ms: u64, deadline: Instant) -> Option<Timespec> {
    let until_deadline = deadline.saturating_duration_since(Instant::now());
    let wait = match next_wake {
        Some(at) => Duration::from_millis(at.saturating_sub(now_ms)).min(until_deadline),
        None => until_deadline,
    };
    Some(Timespec {
        tv_sec: wait.as_secs() as i64,
        tv_nsec: wait.subsec_nanos() as i64,
    })
}

/// Writes what is about to reach the display, as a PPM.
///
/// Call it with the frame after painting and before presenting: what lands in the
/// file is then exactly what the display is about to show, cursor sprite included
/// — a capture rather than a re-render that might differ from what somebody is
/// looking at.
///
/// This exists because a machine with no desktop has no screenshot tool. There is
/// no compositor to ask and no window server to ask it of, so the program holding
/// the buffer is the only thing that can answer, and it should offer to.
pub fn capture(frame: &denise::Frame<'_>, path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let size = frame.size();
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", size.width, size.height)?;
    for y in 0..size.height {
        // Row by row, because a surface's stride is not its width — a scanout
        // buffer is very often padded, and reading it as one flat slice produces a
        // picture that shears further right on every line.
        let Some(row) = frame.row(y) else { continue };
        for word in &row[..size.width as usize] {
            out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
        }
    }
    out.flush()
}
