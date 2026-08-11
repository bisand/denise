//! The subset of `linux/input-event-codes.h` this backend needs.
//!
//! Spelled out rather than pulled from a binding crate so the translation layer
//! stays platform-independent and testable off Linux.

/// Event types.
pub mod ev {
    /// Frame separator.
    pub const SYN: u16 = 0x00;
    /// Keys and buttons.
    pub const KEY: u16 = 0x01;
    /// Relative axes.
    pub const REL: u16 = 0x02;
    /// Absolute axes.
    pub const ABS: u16 = 0x03;
}

/// `EV_SYN` codes.
pub mod syn {
    /// End of an event frame.
    pub const REPORT: u16 = 0;
    /// The kernel dropped events; state must be resynchronised.
    pub const DROPPED: u16 = 3;
}

/// `EV_REL` codes.
pub mod rel {
    /// Horizontal motion.
    pub const X: u16 = 0x00;
    /// Vertical motion.
    pub const Y: u16 = 0x01;
    /// Horizontal wheel, in detents.
    pub const HWHEEL: u16 = 0x06;
    /// Vertical wheel, in detents.
    pub const WHEEL: u16 = 0x08;
}

/// `EV_ABS` codes.
pub mod abs {
    /// Absolute horizontal position.
    pub const X: u16 = 0x00;
    /// Absolute vertical position.
    pub const Y: u16 = 0x01;
    /// Selects the multitouch slot subsequent events apply to.
    pub const MT_SLOT: u16 = 0x2f;
    /// Contact horizontal position.
    pub const MT_POSITION_X: u16 = 0x35;
    /// Contact vertical position.
    pub const MT_POSITION_Y: u16 = 0x36;
    /// Contact identity; `-1` ends the contact in this slot.
    pub const MT_TRACKING_ID: u16 = 0x39;
}

/// `EV_KEY` codes for buttons.
pub mod btn {
    /// Primary pointer button.
    pub const LEFT: u16 = 0x110;
    /// Secondary pointer button.
    pub const RIGHT: u16 = 0x111;
    /// Middle pointer button.
    pub const MIDDLE: u16 = 0x112;
    /// A contact is present. Single-touch panels report this instead of slots.
    pub const TOUCH: u16 = 0x14a;
}

/// `EV_KEY` values.
pub mod key_value {
    /// Released.
    pub const UP: i32 = 0;
    /// Pressed.
    pub const DOWN: i32 = 1;
    /// Held long enough to auto-repeat.
    pub const REPEAT: i32 = 2;
}
