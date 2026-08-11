//! Finding and reading `/dev/input/event*`.

use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use denise::{InputEvent, InputSource, Point, Size};

use crate::codes::abs;
use crate::error::EvdevError;
use crate::translate::{AbsAxis, RawEvent, Translator};

/// What a device is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// A mouse or an absolute pointing device.
    Pointer,
    /// A touchscreen.
    Touch,
    /// A keyboard.
    Keyboard,
}

/// One open input device, with its own translation state.
#[derive(Debug)]
pub struct InputDevice {
    device: evdev::Device,
    path: PathBuf,
    name: String,
    kind: DeviceKind,
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

    /// What the device is for.
    pub fn kind(&self) -> DeviceKind {
        self.kind
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
}

impl InputBackend {
    /// Opens every pointer, touch and keyboard device the process can read.
    ///
    /// Devices that cannot be opened are skipped rather than fatal: a machine with
    /// one unreadable node and one good keyboard should still take input.
    pub fn open_all(surface: Size) -> Result<Self, EvdevError> {
        let mut devices = Vec::new();

        for (path, device) in evdev::enumerate() {
            let Some(kind) = classify(&device) else {
                continue;
            };

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
                kind,
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
        })
    }

    /// The devices that were opened.
    pub fn devices(&self) -> &[InputDevice] {
        &self.devices
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
}

impl InputSource for InputBackend {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        for device in &mut self.devices {
            let Ok(events) = device.device.fetch_events() else {
                // WouldBlock is the normal case: nothing to read right now.
                continue;
            };

            self.scratch.clear();
            for event in events {
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

/// Decides what a device is, from what it can report.
fn classify(device: &evdev::Device) -> Option<DeviceKind> {
    let abs_axes = device.supported_absolute_axes();
    let keys = device.supported_keys();

    let has_abs = |code: u16| abs_axes.is_some_and(|axes| axes.iter().any(|axis| axis.0 == code));

    // Slots mean a real touchscreen. Checked first: a touchscreen also reports
    // BTN_TOUCH and absolute axes, and would otherwise look like a pointer.
    if has_abs(abs::MT_POSITION_X) {
        return Some(DeviceKind::Touch);
    }

    let has_key = |code: u16| keys.is_some_and(|k| k.iter().any(|key| key.0 == code));

    // BTN_LEFT is what separates a pointing device from a device that merely has
    // axes, such as a joystick or an accelerometer.
    if has_key(crate::codes::btn::LEFT) {
        return Some(DeviceKind::Pointer);
    }

    // Letter keys, rather than any key at all: a power button and a lid switch
    // both report EV_KEY and neither is a keyboard.
    if has_key(30) && has_key(44) {
        return Some(DeviceKind::Keyboard);
    }

    None
}
