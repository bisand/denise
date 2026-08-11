//! Choosing which output to drive and at what mode.
//!
//! Like [`crate::swapchain`], this knows nothing about DRM. It works on plain data
//! copied out of the connector list, so the policy can be tested exhaustively on a
//! machine with no display hardware.
//!
//! That separation is not ceremony. Mode selection is where a headless box picks
//! 640×480 because it took `modes[0]`, or where a kiosk with a DSI panel and an
//! HDMI debug monitor plugged in comes up on the wrong one. Both are trivial to
//! test as pure functions and close to impossible to debug in a rack.

use core::fmt;

/// The physical connector type, as far as picking an output cares.
///
/// Mirrors DRM's connector interface list, collapsed to the distinctions that
/// change the decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectorKind {
    /// MIPI DSI — the ribbon-cable panel on a Pi.
    Dsi,
    /// Parallel DPI.
    Dpi,
    /// Embedded DisplayPort, as in a laptop lid.
    Edp,
    /// LVDS, as in older industrial panels.
    Lvds,
    /// HDMI, either connector letter.
    Hdmi,
    /// External DisplayPort.
    DisplayPort,
    /// DVI, any flavour.
    Dvi,
    /// Analogue VGA.
    Vga,
    /// Composite, S-Video and similar analogue TV outputs.
    Composite,
    /// A virtual output, as offered by vkms or a virtualised GPU.
    Virtual,
    /// Anything not worth distinguishing.
    Other,
}

impl ConnectorKind {
    /// Returns `true` if this is a panel physically built into the product.
    #[inline]
    pub const fn is_internal_panel(self) -> bool {
        matches!(
            self,
            ConnectorKind::Dsi | ConnectorKind::Dpi | ConnectorKind::Edp | ConnectorKind::Lvds
        )
    }

    /// Ranking for [`OutputPreference::Auto`]; lower sorts first.
    ///
    /// Internal panels win. On a shipped kiosk the panel *is* the product, and an
    /// HDMI cable someone plugged in to look at a log should not move the UI off
    /// it. Anyone who wants the other behaviour asks for it by kind or by id.
    const fn rank(self) -> u8 {
        match self {
            ConnectorKind::Dsi | ConnectorKind::Dpi => 0,
            ConnectorKind::Edp | ConnectorKind::Lvds => 1,
            ConnectorKind::Hdmi | ConnectorKind::DisplayPort => 2,
            ConnectorKind::Dvi => 3,
            ConnectorKind::Vga => 4,
            ConnectorKind::Composite => 5,
            ConnectorKind::Other => 6,
            // Last resort: on a machine with a real output as well, a virtual one
            // is almost never what was wanted.
            ConnectorKind::Virtual => 7,
        }
    }
}

/// One mode a connector reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeInfo {
    /// Horizontal resolution in pixels.
    pub width: u16,
    /// Vertical resolution in pixels.
    pub height: u16,
    /// Vertical refresh in whole Hz.
    pub vrefresh: u32,
    /// Set if the driver flagged this as the connector's preferred mode, which for
    /// a fixed panel means its native resolution.
    pub preferred: bool,
}

impl ModeInfo {
    /// Pixel count.
    #[inline]
    pub const fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

impl fmt::Display for ModeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}@{}", self.width, self.height, self.vrefresh)
    }
}

/// One connector, reduced to what selection needs.
#[derive(Clone, Debug)]
pub struct ConnectorInfo {
    /// The DRM connector id, for reporting and for [`OutputPreference::Id`].
    pub id: u32,
    /// Physical connector type.
    pub kind: ConnectorKind,
    /// Whether something is plugged in and responding.
    pub connected: bool,
    /// Modes the connector reports, in driver order.
    pub modes: Vec<ModeInfo>,
}

impl ConnectorInfo {
    /// Returns `true` if this connector could actually be driven.
    #[inline]
    pub fn is_usable(&self) -> bool {
        self.connected && !self.modes.is_empty()
    }

    /// The largest mode, breaking ties on refresh rate.
    fn largest(&self) -> Option<usize> {
        self.modes
            .iter()
            .enumerate()
            .max_by_key(|(_, m)| (m.area(), m.vrefresh))
            .map(|(i, _)| i)
    }

    /// The driver's preferred mode, if it flagged one.
    fn preferred(&self) -> Option<usize> {
        self.modes.iter().position(|m| m.preferred)
    }
}

/// Which output to drive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputPreference {
    /// Internal panel first, then external digital, then analogue.
    #[default]
    Auto,
    /// The first usable connector of this type.
    Kind(ConnectorKind),
    /// One specific DRM connector id.
    Id(u32),
}

/// Which mode to set on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModePreference {
    /// Whatever the driver flagged as preferred — a fixed panel's native
    /// resolution. Correct for essentially every embedded display.
    #[default]
    Preferred,
    /// A specific resolution, at any refresh rate.
    Exact {
        /// Horizontal resolution.
        width: u16,
        /// Vertical resolution.
        height: u16,
    },
    /// A specific resolution at a specific refresh rate.
    ExactRefresh {
        /// Horizontal resolution.
        width: u16,
        /// Vertical resolution.
        height: u16,
        /// Vertical refresh in whole Hz.
        vrefresh: u32,
    },
    /// The highest pixel count on offer.
    Largest,
}

/// What [`select`] settled on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Index into the connector slice that was passed in.
    pub connector: usize,
    /// Index into that connector's `modes`.
    pub mode: usize,
    /// Set when the requested mode was unavailable and a fallback was used.
    ///
    /// Not an error — coming up at the wrong resolution beats not coming up — but
    /// the caller should log it, because it means a configured resolution was
    /// silently ignored.
    pub fell_back: bool,
}

/// Why no output could be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SelectionError {
    /// The device reported no connectors at all.
    #[error("the device has no connectors")]
    NoConnectors,

    /// Connectors exist but none has a display attached and modes to offer.
    #[error("no connector has a display attached")]
    NothingConnected,

    /// A specific connector was asked for and is not usable.
    #[error("connector {0} was requested but is not connected")]
    RequestedIdUnavailable(u32),

    /// A connector kind was asked for and none is usable.
    #[error("no usable {0:?} connector")]
    RequestedKindUnavailable(ConnectorKind),
}

/// Picks an output and a mode.
///
/// Mode selection always succeeds once a connector is chosen: an unavailable
/// requested resolution falls back to preferred, then to largest, rather than
/// refusing to start. A kiosk that comes up at the wrong resolution can be fixed
/// remotely; one that does not come up cannot.
pub fn select(
    connectors: &[ConnectorInfo],
    output: OutputPreference,
    mode: ModePreference,
) -> Result<Selection, SelectionError> {
    if connectors.is_empty() {
        return Err(SelectionError::NoConnectors);
    }

    let index = select_connector(connectors, output)?;
    let (mode, fell_back) = select_mode(&connectors[index], mode);

    Ok(Selection {
        connector: index,
        mode,
        fell_back,
    })
}

/// Picks the output alone.
pub fn select_connector(
    connectors: &[ConnectorInfo],
    preference: OutputPreference,
) -> Result<usize, SelectionError> {
    let usable = |i: &usize| connectors[*i].is_usable();

    match preference {
        OutputPreference::Id(id) => (0..connectors.len())
            .filter(usable)
            .find(|&i| connectors[i].id == id)
            .ok_or(SelectionError::RequestedIdUnavailable(id)),

        OutputPreference::Kind(kind) => (0..connectors.len())
            .filter(usable)
            .find(|&i| connectors[i].kind == kind)
            .ok_or(SelectionError::RequestedKindUnavailable(kind)),

        OutputPreference::Auto => (0..connectors.len())
            .filter(usable)
            // Rank first, then prefer the bigger panel, then keep driver order.
            .min_by_key(|&i| {
                let c = &connectors[i];
                let area = c
                    .preferred()
                    .or_else(|| c.largest())
                    .map(|m| c.modes[m].area())
                    .unwrap_or(0);
                (c.kind.rank(), core::cmp::Reverse(area), i)
            })
            .ok_or(SelectionError::NothingConnected),
    }
}

/// Picks the mode alone. Returns the index and whether a fallback was taken.
pub fn select_mode(connector: &ConnectorInfo, preference: ModePreference) -> (usize, bool) {
    let exact = |width: u16, height: u16, vrefresh: Option<u32>| {
        connector
            .modes
            .iter()
            .enumerate()
            // Among equal resolutions take the fastest, so a panel offering 60 and
            // 50 Hz does not land on 50 by accident of driver order.
            .filter(|(_, m)| {
                m.width == width && m.height == height && vrefresh.is_none_or(|hz| m.vrefresh == hz)
            })
            .max_by_key(|(_, m)| m.vrefresh)
            .map(|(i, _)| i)
    };

    let requested = match preference {
        ModePreference::Preferred => connector.preferred(),
        ModePreference::Exact { width, height } => exact(width, height, None),
        ModePreference::ExactRefresh {
            width,
            height,
            vrefresh,
        } => exact(width, height, Some(vrefresh)).or_else(|| exact(width, height, None)),
        ModePreference::Largest => connector.largest(),
    };

    match requested {
        Some(i) => (i, false),
        // Never index blindly into `modes[0]`. Drivers conventionally put the
        // preferred mode first, and conventions are not guarantees.
        None => (
            connector
                .preferred()
                .or_else(|| connector.largest())
                .unwrap_or(0),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u16, height: u16, vrefresh: u32, preferred: bool) -> ModeInfo {
        ModeInfo {
            width,
            height,
            vrefresh,
            preferred,
        }
    }

    fn connector(id: u32, kind: ConnectorKind, modes: Vec<ModeInfo>) -> ConnectorInfo {
        ConnectorInfo {
            id,
            kind,
            connected: true,
            modes,
        }
    }

    fn disconnected(id: u32, kind: ConnectorKind) -> ConnectorInfo {
        ConnectorInfo {
            id,
            kind,
            connected: false,
            modes: vec![mode(1920, 1080, 60, true)],
        }
    }

    /// A Pi with the official touchscreen on DSI and a monitor on HDMI.
    fn pi_with_debug_monitor() -> Vec<ConnectorInfo> {
        vec![
            connector(
                32,
                ConnectorKind::Hdmi,
                vec![mode(1920, 1080, 60, true), mode(1280, 720, 60, false)],
            ),
            connector(45, ConnectorKind::Dsi, vec![mode(800, 480, 60, true)]),
        ]
    }

    #[test]
    fn no_connectors_is_an_error() {
        assert_eq!(
            select(&[], OutputPreference::Auto, ModePreference::Preferred),
            Err(SelectionError::NoConnectors)
        );
    }

    #[test]
    fn nothing_plugged_in_is_an_error() {
        let connectors = vec![
            disconnected(32, ConnectorKind::Hdmi),
            disconnected(45, ConnectorKind::Dsi),
        ];
        assert_eq!(
            select_connector(&connectors, OutputPreference::Auto),
            Err(SelectionError::NothingConnected)
        );
    }

    #[test]
    fn a_connector_with_no_modes_is_not_usable() {
        // Connected but mode-less happens with a live cable and a sleeping display.
        let connectors = vec![connector(32, ConnectorKind::Hdmi, vec![])];
        assert_eq!(
            select_connector(&connectors, OutputPreference::Auto),
            Err(SelectionError::NothingConnected)
        );
    }

    #[test]
    fn the_built_in_panel_wins_over_a_debug_monitor() {
        // The scenario this policy exists for: the kiosk must not migrate to HDMI
        // because somebody plugged a monitor in, even though HDMI is bigger and
        // comes first in driver order.
        let connectors = pi_with_debug_monitor();
        let i = select_connector(&connectors, OutputPreference::Auto).expect("a connector");
        assert_eq!(connectors[i].kind, ConnectorKind::Dsi);
    }

    #[test]
    fn hdmi_is_used_when_there_is_no_panel() {
        let connectors = vec![connector(
            32,
            ConnectorKind::Hdmi,
            vec![mode(1920, 1080, 60, true)],
        )];
        let i = select_connector(&connectors, OutputPreference::Auto).expect("a connector");
        assert_eq!(connectors[i].kind, ConnectorKind::Hdmi);
    }

    #[test]
    fn a_virtual_output_loses_to_anything_real() {
        let connectors = vec![
            connector(1, ConnectorKind::Virtual, vec![mode(1024, 768, 60, true)]),
            connector(2, ConnectorKind::Vga, vec![mode(640, 480, 60, true)]),
        ];
        let i = select_connector(&connectors, OutputPreference::Auto).expect("a connector");
        assert_eq!(connectors[i].kind, ConnectorKind::Vga);
    }

    #[test]
    fn equal_rank_prefers_the_larger_display() {
        let connectors = vec![
            connector(1, ConnectorKind::Hdmi, vec![mode(1280, 720, 60, true)]),
            connector(
                2,
                ConnectorKind::DisplayPort,
                vec![mode(2560, 1440, 60, true)],
            ),
        ];
        let i = select_connector(&connectors, OutputPreference::Auto).expect("a connector");
        assert_eq!(connectors[i].id, 2);
    }

    #[test]
    fn an_explicit_id_overrides_the_ranking() {
        let connectors = pi_with_debug_monitor();
        let i = select_connector(&connectors, OutputPreference::Id(32)).expect("a connector");
        assert_eq!(connectors[i].kind, ConnectorKind::Hdmi);
    }

    #[test]
    fn an_explicit_id_that_is_absent_is_an_error_not_a_fallback() {
        // Silently driving a different display than the one configured is worse
        // than refusing: the operator would never find out.
        let connectors = pi_with_debug_monitor();
        assert_eq!(
            select_connector(&connectors, OutputPreference::Id(99)),
            Err(SelectionError::RequestedIdUnavailable(99))
        );
    }

    #[test]
    fn an_explicit_kind_that_is_absent_is_an_error() {
        let connectors = pi_with_debug_monitor();
        assert_eq!(
            select_connector(&connectors, OutputPreference::Kind(ConnectorKind::Vga)),
            Err(SelectionError::RequestedKindUnavailable(ConnectorKind::Vga))
        );
    }

    #[test]
    fn a_disconnected_requested_id_is_rejected() {
        let connectors = vec![disconnected(32, ConnectorKind::Hdmi)];
        assert_eq!(
            select_connector(&connectors, OutputPreference::Id(32)),
            Err(SelectionError::RequestedIdUnavailable(32))
        );
    }

    #[test]
    fn preferred_mode_is_taken_even_when_it_is_not_first() {
        let c = connector(
            1,
            ConnectorKind::Dsi,
            vec![
                mode(640, 480, 60, false),
                mode(800, 480, 60, true),
                mode(1024, 600, 60, false),
            ],
        );
        let (i, fell_back) = select_mode(&c, ModePreference::Preferred);
        assert_eq!(i, 1);
        assert!(!fell_back);
    }

    #[test]
    fn no_preferred_flag_falls_back_to_the_largest_not_the_first() {
        // The 640x480 bug: taking modes[0] because the driver happened to list it.
        let c = connector(
            1,
            ConnectorKind::Hdmi,
            vec![
                mode(640, 480, 60, false),
                mode(1920, 1080, 60, false),
                mode(1280, 720, 60, false),
            ],
        );
        let (i, fell_back) = select_mode(&c, ModePreference::Preferred);
        assert_eq!(c.modes[i], mode(1920, 1080, 60, false));
        assert!(fell_back);
    }

    #[test]
    fn exact_mode_is_found() {
        let c = connector(
            1,
            ConnectorKind::Hdmi,
            vec![mode(1920, 1080, 60, true), mode(1280, 720, 60, false)],
        );
        let (i, fell_back) = select_mode(
            &c,
            ModePreference::Exact {
                width: 1280,
                height: 720,
            },
        );
        assert_eq!(c.modes[i], mode(1280, 720, 60, false));
        assert!(!fell_back);
    }

    #[test]
    fn exact_mode_takes_the_fastest_refresh_available() {
        let c = connector(
            1,
            ConnectorKind::Hdmi,
            vec![mode(1920, 1080, 50, false), mode(1920, 1080, 60, false)],
        );
        let (i, _) = select_mode(
            &c,
            ModePreference::Exact {
                width: 1920,
                height: 1080,
            },
        );
        assert_eq!(c.modes[i].vrefresh, 60);
    }

    #[test]
    fn an_unavailable_exact_mode_falls_back_and_says_so() {
        let c = connector(1, ConnectorKind::Dsi, vec![mode(800, 480, 60, true)]);
        let (i, fell_back) = select_mode(
            &c,
            ModePreference::Exact {
                width: 1920,
                height: 1080,
            },
        );
        assert_eq!(c.modes[i], mode(800, 480, 60, true));
        assert!(
            fell_back,
            "a silently ignored configured mode must be flagged"
        );
    }

    #[test]
    fn exact_refresh_relaxes_to_the_same_resolution_before_giving_up() {
        let c = connector(
            1,
            ConnectorKind::Hdmi,
            vec![mode(1920, 1080, 60, false), mode(1280, 720, 60, true)],
        );
        let (i, fell_back) = select_mode(
            &c,
            ModePreference::ExactRefresh {
                width: 1920,
                height: 1080,
                vrefresh: 144,
            },
        );
        assert_eq!(c.modes[i], mode(1920, 1080, 60, false));
        assert!(
            !fell_back,
            "same resolution at another rate is not a fallback"
        );
    }

    #[test]
    fn largest_ignores_the_preferred_flag() {
        let c = connector(
            1,
            ConnectorKind::Hdmi,
            vec![mode(1280, 720, 60, true), mode(1920, 1080, 60, false)],
        );
        let (i, _) = select_mode(&c, ModePreference::Largest);
        assert_eq!(c.modes[i], mode(1920, 1080, 60, false));
    }

    #[test]
    fn selection_never_returns_an_out_of_range_index() {
        // Whatever the preference and however odd the mode list, the indices handed
        // back get used to index straight into DRM's arrays.
        let connectors = pi_with_debug_monitor();
        let prefs = [
            ModePreference::Preferred,
            ModePreference::Largest,
            ModePreference::Exact {
                width: 3840,
                height: 2160,
            },
            ModePreference::ExactRefresh {
                width: 800,
                height: 480,
                vrefresh: 75,
            },
        ];
        for pref in prefs {
            let s = select(&connectors, OutputPreference::Auto, pref).expect("a selection");
            assert!(s.connector < connectors.len());
            assert!(s.mode < connectors[s.connector].modes.len());
        }
    }
}
