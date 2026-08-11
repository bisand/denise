//! Opening a DRM device and reading what it can drive.

use std::cell::Cell;
use std::fs::OpenOptions;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use drm::Device as BasicDevice;
use drm::control::{Device as ControlDevice, Mode, ModeTypeFlags, connector, crtc};

use crate::error::DrmError;
use crate::mode::{ConnectorInfo, ConnectorKind, ModeInfo};

/// Where card nodes live.
const DRI_DIR: &str = "/dev/dri";

/// An open DRM device.
///
/// Releases DRM master on drop, so a crash or a clean exit both hand the console
/// back rather than leaving a black screen that needs a reboot.
#[derive(Debug)]
pub struct Card {
    fd: OwnedFd,
    path: Option<PathBuf>,
    mastered: Cell<bool>,
}

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl BasicDevice for Card {}
impl ControlDevice for Card {}

impl Card {
    /// Opens a specific card node, such as `/dev/dri/card0`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DrmError> {
        let path = path.as_ref();
        // std opens with O_CLOEXEC already, which matters: a leaked DRM fd in a
        // child process keeps the device busy for the next run.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| DrmError::Open {
                path: path.to_path_buf(),
                source,
            })?;

        Ok(Self {
            fd: OwnedFd::from(file),
            path: Some(path.to_path_buf()),
            mastered: Cell::new(false),
        })
    }

    /// Opens the first card node that actually has a display output.
    ///
    /// Nodes are tried in name order and skipped unless they report at least one
    /// connector, which filters out render-only devices.
    pub fn open_first() -> Result<Self, DrmError> {
        let dir = std::fs::read_dir(DRI_DIR).map_err(|source| DrmError::Open {
            path: PathBuf::from(DRI_DIR),
            source,
        })?;

        let mut cards: Vec<PathBuf> = dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("card"))
            })
            .collect();
        cards.sort();

        if cards.is_empty() {
            return Err(DrmError::NoDevice);
        }

        let mut last_error = None;
        for path in cards {
            match Card::open(&path) {
                Ok(card) if card.has_display_output() => return Ok(card),
                Ok(_) => {}
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or(DrmError::NoDisplayCapableDevice))
    }

    /// Adopts a descriptor opened by somebody else.
    ///
    /// This is how to coexist with `libseat` or a systemd unit that holds the
    /// device rights, rather than demanding root.
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self {
            fd,
            path: None,
            mastered: Cell::new(false),
        }
    }

    /// The node this card was opened from, if it was opened by path.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns `true` if this device has connectors, rather than being a render
    /// node.
    pub fn has_display_output(&self) -> bool {
        self.resource_handles()
            .map(|res| !res.connectors().is_empty())
            .unwrap_or(false)
    }

    /// Takes the DRM master lock, which is required to set a mode.
    ///
    /// Fails if a compositor or another instance already holds it. See the crate
    /// documentation for the ways to have the right to it.
    pub fn become_master(&self) -> Result<(), DrmError> {
        self.acquire_master_lock().map_err(DrmError::NotMaster)?;
        self.mastered.set(true);
        Ok(())
    }

    /// Gives the master lock back, restoring whatever had the console before.
    pub fn release_master(&self) {
        if self.mastered.replace(false) {
            let _ = self.release_master_lock();
        }
    }

    /// Returns `true` if this process currently holds DRM master.
    pub fn is_master(&self) -> bool {
        self.mastered.get()
    }

    /// Reads every connector, as handles and as plain data for [`crate::mode`].
    ///
    /// The two vectors are index-aligned: a [`crate::Selection::connector`] indexes
    /// both.
    pub fn connectors(&self) -> Result<(Vec<connector::Handle>, Vec<ConnectorInfo>), DrmError> {
        let resources = self.resource_handles().map_err(DrmError::Resources)?;
        let mut handles = Vec::with_capacity(resources.connectors().len());
        let mut infos = Vec::with_capacity(resources.connectors().len());

        for &handle in resources.connectors() {
            // Force a probe: a display powered on after boot reads as disconnected
            // from the cached state, and a kiosk that has to be rebooted because
            // the screen was switched on second is not a kiosk.
            let info = self
                .get_connector(handle, true)
                .map_err(DrmError::Resources)?;

            handles.push(handle);
            infos.push(ConnectorInfo {
                id: u32::from(handle),
                kind: connector_kind(info.interface()),
                connected: info.state() == connector::State::Connected,
                modes: info.modes().iter().map(mode_info).collect(),
            });
        }

        Ok((handles, infos))
    }

    /// Finds a CRTC that can drive `connector`.
    ///
    /// Prefers the one already routed to it, so taking over from the console does
    /// not reshuffle the pipeline for no reason.
    pub fn crtc_for(&self, connector: connector::Handle) -> Result<crtc::Handle, DrmError> {
        let resources = self.resource_handles().map_err(DrmError::Resources)?;
        let info = self
            .get_connector(connector, false)
            .map_err(DrmError::Resources)?;

        if let Some(encoder) = info.current_encoder()
            && let Ok(encoder) = self.get_encoder(encoder)
            && let Some(crtc) = encoder.crtc()
        {
            return Ok(crtc);
        }

        for &handle in info.encoders() {
            let Ok(encoder) = self.get_encoder(handle) else {
                continue;
            };
            if let Some(&crtc) = resources.filter_crtcs(encoder.possible_crtcs()).first() {
                return Ok(crtc);
            }
        }

        Err(DrmError::NoCrtc {
            connector: u32::from(connector),
        })
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        self.release_master();
    }
}

/// Copies the parts of a DRM mode that selection cares about.
fn mode_info(mode: &Mode) -> ModeInfo {
    let (width, height) = mode.size();
    ModeInfo {
        width,
        height,
        vrefresh: mode.vrefresh(),
        preferred: mode.mode_type().contains(ModeTypeFlags::PREFERRED),
    }
}

/// Collapses DRM's connector interface list to the distinctions that matter.
fn connector_kind(interface: connector::Interface) -> ConnectorKind {
    use connector::Interface;

    match interface {
        Interface::DSI => ConnectorKind::Dsi,
        Interface::DPI => ConnectorKind::Dpi,
        Interface::EmbeddedDisplayPort => ConnectorKind::Edp,
        Interface::LVDS => ConnectorKind::Lvds,
        Interface::HDMIA | Interface::HDMIB => ConnectorKind::Hdmi,
        Interface::DisplayPort => ConnectorKind::DisplayPort,
        Interface::DVII | Interface::DVID | Interface::DVIA => ConnectorKind::Dvi,
        Interface::VGA => ConnectorKind::Vga,
        Interface::Composite | Interface::SVideo | Interface::Component | Interface::TV => {
            ConnectorKind::Composite
        }
        Interface::Virtual => ConnectorKind::Virtual,
        _ => ConnectorKind::Other,
    }
}
