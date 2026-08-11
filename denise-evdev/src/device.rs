//! Finding and reading `/dev/input/event*`.

use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use denise::{InputEvent, InputSource, Point, Size};

use crate::codes::abs;
use crate::error::EvdevError;
use crate::layout::{self, Layout};
use crate::translate::{AbsAxis, RawEvent, Translator};

/// What a device can report.
///
/// A set rather than a single kind, because plenty of real hardware is more than
/// one thing: a Logitech K400 is a keyboard with a touchpad on one event node, and
/// most laptops present their touchpad and keyboard together. Picking a single
/// label for those either loses the pointer or loses the keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Reports a mouse or an absolute pointing device.
    pub pointer: bool,
    /// Reports multitouch contacts.
    pub touch: bool,
    /// Reports letter keys.
    pub keyboard: bool,
}

impl Capabilities {
    /// Returns `true` if the device reports nothing this backend can use.
    #[inline]
    pub const fn is_empty(self) -> bool {
        !self.pointer && !self.touch && !self.keyboard
    }
}

impl core::fmt::Display for Capabilities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut first = true;
        for (present, name) in [
            (self.keyboard, "keyboard"),
            (self.pointer, "pointer"),
            (self.touch, "touch"),
        ] {
            if present {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        if first {
            f.write_str("none")?;
        }
        Ok(())
    }
}

/// One open input device, with its own translation state.
#[derive(Debug)]
pub struct InputDevice {
    device: evdev::Device,
    path: PathBuf,
    name: String,
    capabilities: Capabilities,
    translator: Translator,
}

impl InputDevice {
    /// Where the device node lives.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The device's self-reported name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the device can report.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// The absolute-axis calibration read from the device, as `(x, y)`.
    ///
    /// `None` for a relative device. An absolute device reporting `None` here
    /// would be unmappable, so this is worth looking at when a touchscreen lands
    /// in the wrong place.
    pub fn abs_ranges(&self) -> (Option<AbsAxis>, Option<AbsAxis>) {
        self.translator.abs_ranges()
    }
}

impl AsRawFd for InputDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.device.as_raw_fd()
    }
}

/// Every usable input device, read together.
#[derive(Debug)]
pub struct InputBackend {
    devices: Vec<InputDevice>,
    /// The pointer position shared across devices, so a mouse and a tablet move
    /// the same cursor rather than fighting over two.
    pointer: Point,
    scratch: Vec<RawEvent>,
    last_event_age: Option<Duration>,
}

impl InputBackend {
    /// Opens every pointer, touch and keyboard device the process can read.
    ///
    /// Devices that cannot be opened are skipped rather than fatal: a machine with
    /// one unreadable node and one good keyboard should still take input.
    pub fn open_all(surface: Size) -> Result<Self, EvdevError> {
        let mut devices = Vec::new();

        for (path, device) in evdev::enumerate() {
            let capabilities = classify(&device);
            if capabilities.is_empty() {
                continue;
            }

            let name = device.name().unwrap_or("<unnamed>").to_owned();
            let mut translator = Translator::new(surface);

            // An absolute device is unusable without knowing what its readings are
            // out of, and every device has its own range.
            if let Some(axes) = device.supported_absolute_axes() {
                for axis in axes.iter() {
                    let info = device.get_absinfo().ok().and_then(|mut all| {
                        all.find(|(code, _)| *code == axis).map(|(_, info)| info)
                    });
                    if let Some(info) = info {
                        translator
                            .set_abs_range(axis.0, AbsAxis::new(info.minimum(), info.maximum()));
                    }
                }
            }

            // Polling must never stall the frame loop; the loop decides when to
            // sleep, and it does that on the descriptors, not in here.
            if device.set_nonblocking(true).is_err() {
                continue;
            }

            devices.push(InputDevice {
                device,
                path,
                name,
                capabilities,
                translator,
            });
        }

        if devices.is_empty() {
            return Err(EvdevError::NoDevices);
        }

        Ok(Self {
            devices,
            pointer: Point::new(surface.width as i32 / 2, surface.height as i32 / 2),
            scratch: Vec::new(),
            last_event_age: None,
        })
    }

    /// The devices that were opened.
    pub fn devices(&self) -> &[InputDevice] {
        &self.devices
    }

    /// Reads every keyboard with `layout`.
    ///
    /// Defaults to [`layout::US`](crate::layout::US), because [`KeyCode`] names US
    /// positions and a different default would make the two disagree out of the
    /// box. A panel shipped to Norway sets this once at startup; there is no
    /// runtime layout switching to discover, because a kiosk has one keyboard and
    /// it does not change.
    ///
    /// [`KeyCode`]: denise::KeyCode
    pub fn set_layout(&mut self, layout: &'static Layout) {
        for device in &mut self.devices {
            device.translator.set_layout(layout);
        }
    }

    /// Reads every keyboard with the layout this system is configured for.
    ///
    /// Checks `DENISE_KEYMAP`, then `XKB_DEFAULT_LAYOUT`, then the console
    /// keyboard configuration files distributions actually write — so a Pi whose
    /// `/etc/conf.d/loadkmap` says Norwegian gets Norwegian without anyone having
    /// to remember an environment variable.
    ///
    /// Returns what was chosen and where it came from, which is worth logging: a
    /// system configured for a layout Denise has no table for falls back to US,
    /// and that is far easier to diagnose when the panel says so.
    pub fn set_layout_from_system(&mut self) -> (&'static Layout, layout::LayoutSource) {
        let (chosen, source) = layout::from_system();
        self.set_layout(chosen);
        (chosen, source)
    }

    /// Descriptors to wait on.
    ///
    /// Hand these, plus the DRM device's, to `poll`/`epoll` so the process sleeps
    /// until either input arrives or the display retires a flip.
    pub fn raw_fds(&self) -> Vec<RawFd> {
        self.devices.iter().map(AsRawFd::as_raw_fd).collect()
    }

    /// Tells every device the surface changed size.
    pub fn resize(&mut self, size: Size) {
        self.pointer = Point::new(size.width as i32 / 2, size.height as i32 / 2);
        for device in &mut self.devices {
            device.translator.resize(size);
        }
    }

    /// The pointer position shared by all pointing devices.
    pub fn pointer(&self) -> Point {
        self.pointer
    }

    /// How long the most recently read event had been waiting.
    ///
    /// The kernel timestamps an event when the driver receives it, so this is the
    /// delay between the hardware reporting and this process reading — time spent
    /// queued, which no measurement taken after the read can see. `None` until
    /// something has been read.
    ///
    /// A frame loop keeping up reads events within a millisecond or so of the
    /// kernel taking them. A growing figure means the loop is falling behind and
    /// is drawing positions the user has already moved on from.
    pub fn last_event_age(&self) -> Option<Duration> {
        self.last_event_age
    }
}

impl InputSource for InputBackend {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        for device in &mut self.devices {
            let Ok(events) = device.device.fetch_events() else {
                // WouldBlock is the normal case: nothing to read right now.
                continue;
            };

            self.scratch.clear();
            let now = SystemTime::now();
            for event in events {
                // The kernel's own timestamp, so queuing before this read is
                // included rather than invisible.
                self.last_event_age = now.duration_since(event.timestamp()).ok();
                self.scratch.push(RawEvent::new(
                    event.event_type().0,
                    event.code(),
                    event.value(),
                ));
            }

            if self.scratch.is_empty() {
                continue;
            }

            // Every pointing device drives the same cursor, so hand the shared
            // position in and take back whatever it became.
            device.translator.set_pointer(self.pointer);
            device.translator.feed_all(&self.scratch, out);
            self.pointer = device.translator.pointer();
        }
    }
}

/// Works out what a device can report, from what it says it supports.
fn classify(device: &evdev::Device) -> Capabilities {
    let abs_axes = device.supported_absolute_axes();
    let keys = device.supported_keys();

    let has_abs = |code: u16| abs_axes.is_some_and(|axes| axes.iter().any(|axis| axis.0 == code));
    let has_key = |code: u16| keys.is_some_and(|k| k.iter().any(|key| key.0 == code));

    Capabilities {
        // BTN_LEFT is what separates a pointing device from something that merely
        // has axes, such as a joystick or an accelerometer.
        pointer: has_key(crate::codes::btn::LEFT),
        // Slots mean a real touchscreen rather than a tablet or a touchpad.
        touch: has_abs(abs::MT_POSITION_X),
        // Letter keys, not any key at all: a power button and a lid switch both
        // report EV_KEY and neither is a keyboard. KEY_A is 30, KEY_Z is 44.
        keyboard: has_key(30) && has_key(44),
    }
}
