//! Muting the kernel's virtual terminal.
//!
//! A Denise application that owns the display still shares the keyboard with
//! whatever is behind it. On a console-booted kiosk that is a login shell, so
//! every character typed into a Denise text field is also typed at the shell —
//! and `reboot<Enter>` in a form field does what it says. Holding DRM master
//! stops the console *drawing*; it does nothing about the keyboard.
//!
//! [`Console::mute_keyboard`] sets `KDSKBMODE` to `K_OFF`, which makes the kernel
//! discard console keystrokes entirely. Reading `/dev/input/event*` is unaffected,
//! because evdev sits below the console layer — which is the whole trick.
//!
//! # Getting your console back
//!
//! `K_OFF` is process-independent kernel state on a shared device. It is restored
//! on [`Drop`], including while a panic unwinds, but nothing runs on `SIGKILL` or
//! a hard reset, and a muted console has no working `Ctrl+Alt+F2` to escape
//! through — `K_OFF` swallows that too.
//!
//! So write down the escape hatch before you need it. From SSH, or from another
//! VT if you can still reach one:
//!
//! ```text
//! kbd_mode -u -C /dev/tty1     # back to Unicode
//! ```
//!
//! Do not mute during development on a machine you cannot reach over the network.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;

use rustix::ioctl::{self, Getter, IntegerSetter, Opcode, opcode};

/// `KDGKBTYPE` — reports the keyboard type, and fails on anything that is not a
/// console. The standard test for "is this fd really a VT", because every other
/// console ioctl either fails confusingly or, worse, does not.
const KDGKBTYPE: Opcode = opcode::none(0x4B, 0x33);
/// `KDGKBMODE` — read the keyboard translation mode.
const KDGKBMODE: Opcode = opcode::none(0x4B, 0x44);
/// `KDSKBMODE` — set it. Takes the mode as an integer argument, not a pointer.
const KDSKBMODE: Opcode = opcode::none(0x4B, 0x45);
/// `KDGETMODE` — read the console's text/graphics mode.
const KDGETMODE: Opcode = opcode::none(0x4B, 0x3B);
/// `KDSETMODE` — set it. Integer argument, as with `KDSKBMODE`.
const KDSETMODE: Opcode = opcode::none(0x4B, 0x3A);

/// `K_RAW`: the console delivers raw scancodes.
pub const K_RAW: u32 = 0x00;
/// `K_XLATE`: scancodes translated to bytes through the loaded keymap.
pub const K_XLATE: u32 = 0x01;
/// `K_MEDIUMRAW`: keycodes rather than scancodes, still untranslated.
pub const K_MEDIUMRAW: u32 = 0x02;
/// `K_UNICODE`: translated and UTF-8 encoded. Where a modern console sits.
pub const K_UNICODE: u32 = 0x03;
/// `K_OFF`: the kernel reads the keyboard and throws the result away.
pub const K_OFF: u32 = 0x04;

/// `KD_TEXT`: the console draws text. The default.
pub const KD_TEXT: u32 = 0x00;
/// `KD_GRAPHICS`: the console stops drawing text and stops blanking the screen.
pub const KD_GRAPHICS: u32 = 0x01;

/// Where to look for a console, in the order a kiosk wants them tried.
///
/// `/dev/tty` is the process's own controlling terminal, which is the VT itself
/// when the application was started by a getty autologin — the normal kiosk boot
/// — and needs no privilege. `/dev/tty0` is the *active* VT whoever owns it, and
/// is `root`-only on most distributions.
///
/// Over SSH `/dev/tty` is a pty, and a pty is not a console. That is what
/// `KDGKBTYPE` is for.
const CANDIDATES: [&str; 3] = ["/dev/tty", "/dev/tty0", "/dev/console"];

/// Something went wrong talking to the console.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    /// None of the candidate paths was openable and a real console.
    ///
    /// Over SSH this is the expected outcome: there is no VT to mute.
    #[error("no console found (tried {}); over SSH there is no VT to mute", CANDIDATES.join(", "))]
    NoConsole,
    /// A console ioctl failed.
    #[error("console ioctl failed: {0}")]
    Ioctl(#[source] std::io::Error),
    /// The console device could not be opened.
    #[error("could not open {path}: {source}")]
    Open {
        /// The path that failed.
        path: String,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}

/// A handle to the virtual terminal, which restores whatever it changed on drop.
///
/// Nothing is changed by opening one. Call [`mute_keyboard`](Self::mute_keyboard)
/// and [`graphics_mode`](Self::graphics_mode) for that, and each remembers the
/// mode it replaced so [`restore`](Self::restore) puts back exactly what was
/// there rather than a guess at the default.
///
/// ```no_run
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use denise_evdev::Console;
///
/// // A kiosk owns the screen and the keyboard for as long as this lives.
/// let mut console = Console::open()?;
/// console.mute_keyboard()?;
/// console.graphics_mode()?;
/// # Ok(())
/// # }
/// ```
///
/// Developing over SSH, [`Console::open`] returns [`ConsoleError::NoConsole`] and
/// the right response is to carry on without one — see
/// [`open_if_present`](Self::open_if_present).
#[derive(Debug)]
pub struct Console {
    file: File,
    /// The keyboard mode to put back, if we changed it.
    keyboard: Option<u32>,
    /// The text/graphics mode to put back, if we changed it.
    screen: Option<u32>,
}

impl Console {
    /// Opens the first of `/dev/tty`, `/dev/tty0`, `/dev/console` that is a real
    /// virtual terminal.
    ///
    /// Returns [`ConsoleError::NoConsole`] when there is none — over SSH, under a
    /// terminal emulator, or in a container.
    pub fn open() -> Result<Self, ConsoleError> {
        for path in CANDIDATES {
            // A candidate that is missing, forbidden or not a console is not an
            // error yet; the next one may work. Only running out of them is.
            if let Ok(console) = Self::open_path(Path::new(path)) {
                return Ok(console);
            }
        }
        Err(ConsoleError::NoConsole)
    }

    /// [`open`](Self::open), but a missing console is `None` rather than an error.
    ///
    /// For the common shape: mute if there is something to mute, and run normally
    /// on a development machine where there is not.
    pub fn open_if_present() -> Option<Self> {
        Self::open().ok()
    }

    /// Opens a specific console device.
    ///
    /// Fails with [`ConsoleError::NoConsole`] if the path opens but is not a
    /// virtual terminal.
    pub fn open_path(path: &Path) -> Result<Self, ConsoleError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| ConsoleError::Open {
                path: path.display().to_string(),
                source,
            })?;

        // SAFETY: KDGKBTYPE is a console opcode that writes one byte through the
        // argument pointer. `Getter<_, u8>` supplies storage of exactly that size
        // and reads it back only on success. On a non-console fd the ioctl fails,
        // which is precisely the question being asked.
        let kind = unsafe { ioctl::ioctl(file.as_fd(), Getter::<KDGKBTYPE, u8>::new()) };
        if kind.is_err() {
            return Err(ConsoleError::NoConsole);
        }

        Ok(Self {
            file,
            keyboard: None,
            screen: None,
        })
    }

    /// Stops console keystrokes reaching the shell behind the UI.
    ///
    /// evdev still delivers everything, so Denise's own input is unaffected. This
    /// also disables `Ctrl+Alt+F<n>` VT switching, because the kernel discards
    /// those keys along with the rest — read the module docs before using it on a
    /// machine you cannot reach over the network.
    ///
    /// Calling it twice is harmless: the mode remembered is the one from before
    /// the first call.
    pub fn mute_keyboard(&mut self) -> Result<(), ConsoleError> {
        if self.keyboard.is_none() {
            // Read before writing, and give up if the read fails. Muting a
            // console we cannot un-mute is the one outcome worth refusing.
            self.keyboard = Some(self.keyboard_mode()?);
        }
        self.set_keyboard_mode(K_OFF)
    }

    /// Puts the console into graphics mode, so it stops drawing text over the
    /// display and stops blanking it.
    ///
    /// DRM master already keeps fbcon off the scanout buffer in normal operation.
    /// This covers what master does not: console blanking on an idle panel, and
    /// the kernel repainting text after a VT switch or an oops.
    pub fn graphics_mode(&mut self) -> Result<(), ConsoleError> {
        if self.screen.is_none() {
            self.screen = Some(self.screen_mode()?);
        }
        self.set_screen_mode(KD_GRAPHICS)
    }

    /// Puts back every mode this handle changed, and forgets them.
    ///
    /// Runs automatically on drop. Call it directly to hand the console back
    /// early — before spawning a shell, say — or to see the error, which [`Drop`]
    /// has nowhere to report.
    pub fn restore(&mut self) -> Result<(), ConsoleError> {
        let mut result = Ok(());
        // Both are attempted even if the first fails: a console left in graphics
        // mode is bad, and one left with no keyboard is worse, so neither should
        // be skipped because of the other.
        if let Some(mode) = self.keyboard.take() {
            result = result.and(self.set_keyboard_mode(mode));
        }
        if let Some(mode) = self.screen.take() {
            result = result.and(self.set_screen_mode(mode));
        }
        result
    }

    /// The current keyboard translation mode: one of [`K_RAW`], [`K_XLATE`],
    /// [`K_MEDIUMRAW`], [`K_UNICODE`] or [`K_OFF`].
    pub fn keyboard_mode(&self) -> Result<u32, ConsoleError> {
        // SAFETY: KDGKBMODE writes one `int` through the argument pointer on a
        // console fd, which `open_path` established this is. `Getter<_, u32>`
        // provides storage of exactly that size.
        let mode = unsafe { ioctl::ioctl(self.file.as_fd(), Getter::<KDGKBMODE, u32>::new()) };
        mode.map_err(|e| ConsoleError::Ioctl(e.into()))
    }

    /// The current screen mode: [`KD_TEXT`] or [`KD_GRAPHICS`].
    pub fn screen_mode(&self) -> Result<u32, ConsoleError> {
        // SAFETY: as `keyboard_mode`, for KDGETMODE.
        let mode = unsafe { ioctl::ioctl(self.file.as_fd(), Getter::<KDGETMODE, u32>::new()) };
        mode.map_err(|e| ConsoleError::Ioctl(e.into()))
    }

    fn set_keyboard_mode(&self, mode: u32) -> Result<(), ConsoleError> {
        // SAFETY: KDSKBMODE takes its mode as the integer argument rather than
        // through a pointer. `mode` is either K_OFF or a value KDGKBMODE just
        // returned, so it is in range by construction.
        let result = unsafe {
            ioctl::ioctl(
                self.file.as_fd(),
                IntegerSetter::<KDSKBMODE>::new_usize(mode as usize),
            )
        };
        result.map_err(|e| ConsoleError::Ioctl(e.into()))
    }

    fn set_screen_mode(&self, mode: u32) -> Result<(), ConsoleError> {
        // SAFETY: as `set_keyboard_mode`, for KDSETMODE and KD_GRAPHICS.
        let result = unsafe {
            ioctl::ioctl(
                self.file.as_fd(),
                IntegerSetter::<KDSETMODE>::new_usize(mode as usize),
            )
        };
        result.map_err(|e| ConsoleError::Ioctl(e.into()))
    }
}

impl AsFd for Console {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        // Deliberately ignored: drop has nowhere to report to, and leaving the
        // console muted because the restore failed is strictly worse than trying
        // and failing quietly. A caller that wants the error calls `restore`.
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers from `linux/kd.h`, spelled out.
    ///
    /// rustix composes opcodes from a group and a number, and a console opcode is
    /// a bare `_IO` with no size or direction bits — so the composition should be
    /// the identity. If it ever is not, or a digit here is wrong, these ioctls
    /// would still be *valid*; they would just be a different driver command
    /// against the same fd. Assert the values, not the arithmetic.
    #[test]
    fn opcodes_match_the_kernel_headers() {
        assert_eq!(KDGKBTYPE, 0x4B33);
        assert_eq!(KDSETMODE, 0x4B3A);
        assert_eq!(KDGETMODE, 0x4B3B);
        assert_eq!(KDGKBMODE, 0x4B44);
        assert_eq!(KDSKBMODE, 0x4B45);
    }

    #[test]
    fn mode_constants_match_the_kernel_headers() {
        assert_eq!(
            [K_RAW, K_XLATE, K_MEDIUMRAW, K_UNICODE, K_OFF],
            [0, 1, 2, 3, 4]
        );
        assert_eq!([KD_TEXT, KD_GRAPHICS], [0, 1]);
    }

    /// `/dev/null` opens read-write on every Linux machine and is not a console.
    ///
    /// The gate this checks is the one standing between a developer over SSH and
    /// a muted pty: without it, `open` would hand back the first thing that
    /// opened and the ioctls would fail later, somewhere less obvious.
    #[test]
    fn a_character_device_that_is_not_a_console_is_refused() {
        let result = Console::open_path(Path::new("/dev/null"));
        assert!(
            matches!(result, Err(ConsoleError::NoConsole)),
            "expected NoConsole, got {result:?}"
        );
    }
}
