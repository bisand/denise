//! Finding and reading `/dev/input/event*`.

use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rustix::fs::inotify;
use rustix::io::Errno;

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

/// Where the device nodes are. Not configurable: this is the only place Linux
/// puts them, and a backend that looked somewhere else would be looking at
/// nothing.
const DEV_INPUT: &str = "/dev/input";

/// Every usable input device, read together.
#[derive(Debug)]
pub struct InputBackend {
    devices: Vec<InputDevice>,
    /// The pointer position shared across devices, so a mouse and a tablet move
    /// the same cursor rather than fighting over two.
    pointer: Point,
    scratch: Vec<RawEvent>,
    last_event_age: Option<Duration>,
    /// The surface size, kept so a device opened later is calibrated like the
    /// ones opened at startup.
    surface: Size,
    /// An inotify descriptor on `/dev/input`, or `None` where one could not be
    /// had — a container with no permission, mostly. Input still works; it just
    /// stops being noticed after startup.
    watch: Option<OwnedFd>,
    /// Set when [`InputBackend::poll`] opened or dropped a device, cleared by
    /// whoever asks. See [`InputBackend::devices_changed`].
    changed: bool,
}

impl InputBackend {
    /// Opens every pointer, touch and keyboard device the process can read.
    ///
    /// Devices that cannot be opened are skipped rather than fatal: a machine with
    /// one unreadable node and one good keyboard should still take input.
    pub fn open_all(surface: Size) -> Result<Self, EvdevError> {
        let devices: Vec<InputDevice> = evdev::enumerate()
            .filter_map(|(path, device)| adopt(path, device, surface))
            .collect();

        if devices.is_empty() {
            return Err(EvdevError::NoDevices);
        }

        Ok(Self {
            devices,
            pointer: Point::new(surface.width as i32 / 2, surface.height as i32 / 2),
            scratch: Vec::new(),
            last_event_age: None,
            surface,
            watch: watch_dev_input(),
            changed: false,
        })
    }

    /// Whether the device set changed when [`poll`](InputSource::poll) last ran,
    /// clearing the flag.
    ///
    /// A loop waiting on [`raw_fds`](Self::raw_fds) has to ask, because the
    /// descriptors it is holding are stale the moment this returns `true` — one
    /// of them may name a device that has been closed, and a device that has just
    /// been opened is not in the set at all.
    pub fn devices_changed(&mut self) -> bool {
        core::mem::take(&mut self.changed)
    }

    /// Opens devices that have appeared and drops ones that have gone.
    ///
    /// Called from [`poll`](InputSource::poll); there is no reason to call it
    /// directly, and doing so costs a directory read.
    ///
    /// **Why this exists at all.** A wireless mouse that is asleep when the panel
    /// starts has no `/dev/input/event*` node — the receiver enumerates, the mouse
    /// does not, and the node is created minutes later when somebody moves it. A
    /// backend that scanned once at startup would never see that mouse, and the
    /// only cure would be restarting the application. Measured on a Pi 3 with a
    /// Logitech unifying receiver: the node appeared 775 seconds after boot.
    fn rescan(&mut self) {
        let Ok(entries) = std::fs::read_dir(DEV_INPUT) else {
            return;
        };

        let mut present: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("event"))
            })
            .collect();
        // Deterministic order, so a device that appears twice in one scan cannot
        // land in a different slot than it would have at startup.
        present.sort();

        // Gone first: a descriptor for a removed device stays readable and always
        // returns an error, which would otherwise be polled forever.
        let before = self.devices.len();
        self.devices.retain(|device| present.contains(&device.path));
        self.changed |= self.devices.len() != before;

        for path in present {
            if self.devices.iter().any(|device| device.path == path) {
                continue;
            }
            // A node that mdev has created but not yet chowned fails here with
            // EACCES. That is not final: the chmod is an IN_ATTRIB of its own, so
            // this runs again in a moment and succeeds the second time.
            let Ok(device) = evdev::Device::open(&path) else {
                continue;
            };
            if let Some(device) = adopt(path, device, self.surface) {
                self.devices.push(device);
                self.changed = true;
            }
        }
    }

    /// Drains the watch descriptor, reporting whether anything happened.
    ///
    /// The events themselves are thrown away. Which file changed is not worth
    /// acting on individually: a full directory read costs microseconds and is
    /// right in every case, including the ones inotify cannot report — a queue
    /// overflow, or a device that was already there when the watch was set.
    fn watch_fired(&mut self) -> bool {
        let Some(watch) = self.watch.as_ref() else {
            return false;
        };
        let mut buf = [MaybeUninit::uninit(); 512];
        let mut reader = inotify::Reader::new(watch.as_fd(), &mut buf);
        let mut fired = false;
        loop {
            match reader.next() {
                Ok(_) => fired = true,
                // The normal case, every frame in which nothing was plugged in.
                Err(Errno::WOULDBLOCK) => return fired,
                Err(_) => return fired,
            }
        }
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
    ///
    /// The last of these is the `/dev/input` watch rather than a device, which is
    /// what wakes a sleeping loop when a mouse is plugged in — without it the new
    /// device would sit unread until something else happened to wake the process.
    /// Re-read this list whenever [`devices_changed`](Self::devices_changed) says
    /// to; the old one names descriptors that may since have been closed.
    pub fn raw_fds(&self) -> Vec<RawFd> {
        let mut fds: Vec<RawFd> = self.devices.iter().map(AsRawFd::as_raw_fd).collect();
        if let Some(watch) = self.watch.as_ref() {
            fds.push(watch.as_raw_fd());
        }
        fds
    }

    /// Tells every device the surface changed size.
    pub fn resize(&mut self, size: Size) {
        self.surface = size;
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
        // Before reading, so a device that appeared during the wait is read in
        // this frame rather than the next one.
        if self.watch_fired() {
            self.rescan();
        }

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

/// Prepares one opened device, or `None` if it reports nothing usable.
fn adopt(path: PathBuf, device: evdev::Device, surface: Size) -> Option<InputDevice> {
    let capabilities = classify(&device);
    if capabilities.is_empty() {
        return None;
    }

    let name = device.name().unwrap_or("<unnamed>").to_owned();
    let mut translator = Translator::new(surface);

    // An absolute device is unusable without knowing what its readings are out
    // of, and every device has its own range.
    if let Some(axes) = device.supported_absolute_axes() {
        for axis in axes.iter() {
            let info = device
                .get_absinfo()
                .ok()
                .and_then(|mut all| all.find(|(code, _)| *code == axis).map(|(_, info)| info));
            if let Some(info) = info {
                translator.set_abs_range(axis.0, AbsAxis::new(info.minimum(), info.maximum()));
            }
        }
    }

    // Polling must never stall the frame loop; the loop decides when to sleep,
    // and it does that on the descriptors, not in here.
    device.set_nonblocking(true).ok()?;

    Some(InputDevice {
        device,
        path,
        name,
        capabilities,
        translator,
    })
}

/// Watches `/dev/input` for devices arriving and leaving.
///
/// `ATTRIB` is in the mask alongside `CREATE` because the node and its
/// permissions are two separate events: udev and mdev both create the node as
/// root-only and chmod it a moment later, so a backend that only listened for
/// `CREATE` would try to open it exactly once, too early, and give up.
///
/// A failure here is not an error. Input works; it just stops being noticed.
fn watch_dev_input() -> Option<OwnedFd> {
    let watch =
        inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK).ok()?;
    inotify::add_watch(
        &watch,
        DEV_INPUT,
        inotify::WatchFlags::CREATE
            | inotify::WatchFlags::ATTRIB
            | inotify::WatchFlags::DELETE
            | inotify::WatchFlags::MOVED_TO
            | inotify::WatchFlags::MOVED_FROM,
    )
    .ok()?;
    Some(watch)
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
