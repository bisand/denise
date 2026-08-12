//! The automation surface, as data: what a script can name and what it means.
//!
//! `IDispatch` is two decisions and a lot of pointer handling. The decisions are
//! *which member is this name* and *which of get, put or call did the host mean*,
//! and both are pure functions of a table. So the table lives here, outside
//! `cfg(windows)`, along with every rule that reads it — the same split
//! [`crate::himetric`] makes, for the same reason: this is the part that goes
//! wrong, and a Windows runner is a slow place to find that out.
//!
//! # No type library
//!
//! There is none, so a host is late-bound: it asks for a name, gets a dispid, and
//! invokes it. VBScript, JScript, VB6 through an `Object` variable and MFC's
//! `COleDispatchDriver` all work that way and need nothing else.
//!
//! PowerShell does not. It builds its member table from `ITypeInfo` and will not
//! ask for a name it has not been told about, so it reaches this control through
//! `[System.__ComObject].InvokeMember` instead. The crate documentation has the
//! incantation and the reason.
//!
//! Either way the table is short, because without a type library each member is
//! something a person has to read about rather than discover by pressing `.`, so
//! each one has to earn its place.
//!
//! # Why the dispids are pinned
//!
//! A compiled early-bound host stores the *number*, not the name. Renumbering a
//! member would not break a script, and would silently break every binary that
//! ever bound to it. The test below pins them for that reason.

// ------------------------------------------------------------------- the flags

// The `DISPATCH_*` values from oaidl.h, spelled here so the table can be read and
// tested on a machine with no Windows headers. A `cfg(windows)` test below checks
// each one against the real constant.

/// `DISPATCH_METHOD`: the host is calling this as a method.
pub const CALL: u16 = 0x1;
/// `DISPATCH_PROPERTYGET`: the host is reading it.
pub const GET: u16 = 0x2;
/// `DISPATCH_PROPERTYPUT`: the host is assigning to it.
pub const PUT: u16 = 0x4;
/// `DISPATCH_PROPERTYPUTREF`: assigning a reference rather than a value. This
/// control has no object-valued properties, so it means the same as [`PUT`] —
/// but a host that sends it and gets `DISP_E_MEMBERNOTFOUND` back has no way to
/// guess why, so it is accepted.
pub const PUTREF: u16 = 0x8;

/// `DISPID_UNKNOWN`, the answer for a name this control does not have.
pub const DISPID_UNKNOWN: i32 = -1;

// ----------------------------------------------------------------- the members

/// The field's contents, as a string.
pub const DISPID_TEXT: i32 = 1;
/// The heading above it, as a string.
pub const DISPID_CAPTION: i32 = 2;
/// Whether the field and the button respond to input, as a boolean.
pub const DISPID_ENABLED: i32 = 3;
/// Repaint everything, taking no arguments and returning nothing.
pub const DISPID_REFRESH: i32 = 4;

/// One member of the control's default dispatch interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    /// What `GetIDsOfNames` returns and `Invoke` receives.
    pub dispid: i32,
    /// The name a script writes. Compared case-insensitively: Basic is not a
    /// case-sensitive language and neither is `GetIDsOfNames`.
    pub name: &'static str,
    /// Which of [`CALL`], [`GET`] and [`PUT`] this member allows.
    pub flags: u16,
}

/// Everything a script can reach on the control.
pub const MEMBERS: &[Member] = &[
    Member {
        dispid: DISPID_TEXT,
        name: "Text",
        flags: GET | PUT,
    },
    Member {
        dispid: DISPID_CAPTION,
        name: "Caption",
        flags: GET | PUT,
    },
    Member {
        dispid: DISPID_ENABLED,
        name: "Enabled",
        flags: GET | PUT,
    },
    Member {
        dispid: DISPID_REFRESH,
        name: "Refresh",
        flags: CALL,
    },
];

// ------------------------------------------------------------------ the events

/// `DISPID_CLICK` from olectl.h.
///
/// The standard number rather than a private one: a host that knows the OLE
/// control conventions recognises it without being told, and with no type
/// library that is the only way it can.
pub const DISPID_CLICK: i32 = -600;

/// The field's contents changed because somebody typed in it.
///
/// Private, because there is no standard dispid for it. Not raised when a script
/// assigns to `Text` — an event that fires back at the assignment that caused it
/// is how a two-line handler becomes an infinite loop.
pub const DISPID_CHANGE: i32 = 1;

/// What the control raises, for documentation and for the tests.
pub const EVENTS: &[Member] = &[
    Member {
        dispid: DISPID_CLICK,
        name: "Click",
        flags: CALL,
    },
    Member {
        dispid: DISPID_CHANGE,
        name: "Change",
        flags: CALL,
    },
];

// ------------------------------------------------------------------ the lookups

/// The member with this name, ignoring case.
pub fn member(name: &str) -> Option<&'static Member> {
    MEMBERS.iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

/// The member with this dispid.
pub fn member_by_id(dispid: i32) -> Option<&'static Member> {
    MEMBERS.iter().find(|m| m.dispid == dispid)
}

/// Resolves the names `GetIDsOfNames` was given, reporting whether all of them
/// were known.
///
/// `names[0]` is the member. Everything after it is a *named argument*, and this
/// control has none — so those never resolve, which is the honest answer and the
/// one that makes a host fall back to positional arguments rather than silently
/// binding to the wrong parameter.
///
/// Unknown names are still written as [`DISPID_UNKNOWN`] even though the call
/// fails, because that is what the contract says a caller may read afterwards.
pub fn resolve(names: &[&str], out: &mut [i32]) -> bool {
    let mut all_known = true;
    for (index, name) in names.iter().enumerate() {
        let dispid = match index {
            0 => member(name).map(|m| m.dispid),
            _ => None,
        };
        if let Some(slot) = out.get_mut(index) {
            *slot = dispid.unwrap_or(DISPID_UNKNOWN);
        }
        all_known &= dispid.is_some();
    }
    all_known
}

/// What a host meant by an `Invoke`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Read a property.
    Get,
    /// Assign to a property.
    Put,
    /// Call a method.
    Call,
}

/// Which of get, put and call the host asked for, or `None` if this member does
/// not allow any of them.
///
/// Hosts are not tidy about `wFlags`. VBScript sends `METHOD | PROPERTYGET` for
/// anything whose result it uses, because at the point of the call it does not
/// know which one the object has; assignment arrives as `PROPERTYPUT`, or as
/// `PROPERTYPUTREF` for an object. So the flags are a set of things the host
/// would accept, and this picks the one the member actually offers.
pub fn action(member: &Member, flags: u16) -> Option<Action> {
    // Assignment first: a put is unambiguous, and a host never sends it by
    // accident alongside a read.
    if flags & (PUT | PUTREF) != 0 && member.flags & PUT != 0 {
        return Some(Action::Put);
    }
    // A method. `PROPERTYGET` alone counts, because that is what some hosts send
    // for a method whose return value is used in an expression, and a method that
    // refuses it fails in a way nobody can read.
    if member.flags & CALL != 0 && flags & (CALL | GET) != 0 {
        return Some(Action::Call);
    }
    if flags & GET != 0 && member.flags & GET != 0 {
        return Some(Action::Get);
    }
    None
}

// ------------------------------------------------------------------ the raising

/// The events one pass over the tree raises, in the order a host should see them.
///
/// `previous` is the text the host last saw. A property put updates it too, which
/// is what stops `Text = "x"` from raising `Change` back at the script that wrote
/// it.
///
/// `Change` comes before `Click`: somebody who types into the field and then
/// presses the button expects the handler for the button to see what they typed.
pub fn events_raised(previous: &str, current: &str, clicked: bool) -> Vec<i32> {
    let mut raised = Vec::new();
    if previous != current {
        raised.push(DISPID_CHANGE);
    }
    if clicked {
        raised.push(DISPID_CLICK);
    }
    raised
}

/// The automation surface as a script would have to be told it, since there is no
/// type library to read it from.
///
/// Used by the crate's documentation and by `examples/host.rs`, so the printed
/// description cannot drift from the table it describes.
pub fn describe() -> String {
    use core::fmt::Write as _;

    let mut out = String::new();
    for m in MEMBERS {
        let kind = if m.flags & CALL != 0 {
            "method"
        } else if m.flags & PUT != 0 {
            "property, read/write"
        } else {
            "property, read-only"
        };
        let _ = writeln!(out, "  {:<10} {:>5}  {kind}", m.name, m.dispid);
    }
    for e in EVENTS {
        let _ = writeln!(out, "  {:<10} {:>5}  event", e.name, e.dispid);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Basic is not case-sensitive and neither is `GetIDsOfNames`. A host that
    /// writes `panel.text` and gets `DISP_E_UNKNOWNNAME` has been told the member
    /// does not exist, which is a lie.
    #[test]
    fn every_member_resolves_whatever_case_it_is_written_in() {
        for m in MEMBERS {
            for spelling in [
                m.name.to_ascii_lowercase(),
                m.name.to_ascii_uppercase(),
                m.name.to_string(),
            ] {
                assert_eq!(
                    member(&spelling).map(|found| found.dispid),
                    Some(m.dispid),
                    "{spelling} did not resolve"
                );
            }
            assert_eq!(member_by_id(m.dispid), Some(m));
        }
    }

    /// A compiled early-bound host stores the *number*, not the name. Renumbering
    /// a member would not break a single script and would silently break every
    /// binary that ever bound to it.
    #[test]
    fn the_dispids_are_the_ones_that_were_published() {
        assert_eq!(DISPID_TEXT, 1);
        assert_eq!(DISPID_CAPTION, 2);
        assert_eq!(DISPID_ENABLED, 3);
        assert_eq!(DISPID_REFRESH, 4);
        assert_eq!(DISPID_CLICK, -600, "DISPID_CLICK from olectl.h");
        assert_eq!(DISPID_CHANGE, 1);

        for (index, m) in MEMBERS.iter().enumerate() {
            assert!(m.dispid > 0, "{} must not use a reserved dispid", m.name);
            assert!(
                MEMBERS[..index]
                    .iter()
                    .all(|other| other.dispid != m.dispid),
                "{} reuses a dispid",
                m.name
            );
        }
    }

    /// An unknown name has to fail *and* leave something readable behind: the
    /// contract says a caller may inspect the array either way.
    #[test]
    fn an_unknown_name_is_reported_and_still_written() {
        let mut out = [0i32; 1];
        assert!(!resolve(&["Nonsense"], &mut out));
        assert_eq!(out, [DISPID_UNKNOWN]);

        assert!(resolve(&["Caption"], &mut out));
        assert_eq!(out, [DISPID_CAPTION]);
    }

    /// Only the first name is a member; the rest are named arguments, which this
    /// control has none of. Resolving one anyway would bind a host's argument to
    /// a member's dispid — a wrong answer where a refusal makes the host fall
    /// back to positional arguments and work.
    #[test]
    fn a_named_argument_never_resolves_even_when_it_matches_a_member() {
        let mut out = [0i32; 2];
        assert!(!resolve(&["Refresh", "Caption"], &mut out));
        assert_eq!(out, [DISPID_REFRESH, DISPID_UNKNOWN]);
    }

    /// VBScript sends `METHOD | PROPERTYGET` for anything whose result it uses,
    /// because at the call site it does not know which one the object has. Both
    /// of these are that one flag combination, and they have to land differently.
    #[test]
    fn the_ambiguous_flags_every_script_host_sends_land_on_the_right_action() {
        let text = member("Text").expect("Text");
        let refresh = member("Refresh").expect("Refresh");

        assert_eq!(action(text, CALL | GET), Some(Action::Get));
        assert_eq!(action(refresh, CALL | GET), Some(Action::Call));
        assert_eq!(action(refresh, CALL), Some(Action::Call));
        // A method whose result is used, from a host that sends only the read
        // flag. Refusing this is correct by the letter and useless in practice.
        assert_eq!(action(refresh, GET), Some(Action::Call));
    }

    /// `PROPERTYPUTREF` is what a host sends when assigning an object. Nothing
    /// here is object-valued, so it means the same as `PROPERTYPUT` — and a host
    /// that sends it and is refused has no way to find out why.
    #[test]
    fn both_spellings_of_assignment_are_a_put() {
        let caption = member("Caption").expect("Caption");
        assert_eq!(action(caption, PUT), Some(Action::Put));
        assert_eq!(action(caption, PUTREF), Some(Action::Put));
        assert_eq!(action(caption, PUT | PUTREF), Some(Action::Put));
    }

    /// Assigning to a method is a mistake in the script, and the only useful
    /// answer is a refusal the host can turn into an error message.
    #[test]
    fn assigning_to_something_that_is_not_a_property_is_refused() {
        let refresh = member("Refresh").expect("Refresh");
        assert_eq!(action(refresh, PUT), None);
        assert_eq!(action(refresh, PUTREF), None);
        assert_eq!(action(refresh, 0), None);
    }

    /// The rule that keeps a handler from being its own cause: assigning to
    /// `Text` moves `previous` as well, so the assignment raises nothing.
    #[test]
    fn a_script_assigning_to_text_does_not_raise_change_at_itself() {
        assert!(events_raised("hei", "hei", false).is_empty());
        assert_eq!(events_raised("", "h", false), vec![DISPID_CHANGE]);
    }

    /// Typing and then pressing the button in one pass: the click handler has to
    /// see the text that was typed, which means `Change` goes first.
    #[test]
    fn change_is_raised_before_the_click_that_follows_it() {
        assert_eq!(
            events_raised("", "hei", true),
            vec![DISPID_CHANGE, DISPID_CLICK]
        );
        assert_eq!(events_raised("hei", "hei", true), vec![DISPID_CLICK]);
    }

    #[test]
    fn the_description_names_every_member_and_every_event() {
        let text = describe();
        for m in MEMBERS.iter().chain(EVENTS) {
            assert!(text.contains(m.name), "{} is undocumented", m.name);
        }
    }

    /// The flags above are oaidl.h's, spelled out so the table can be read on a
    /// machine with no Windows headers. This is the check that they are the same
    /// numbers.
    #[test]
    #[cfg(windows)]
    fn the_flags_are_the_ones_oaidl_defines() {
        use windows::Win32::System::Com::{
            DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPATCH_PROPERTYPUTREF,
        };
        assert_eq!(CALL, DISPATCH_METHOD.0);
        assert_eq!(GET, DISPATCH_PROPERTYGET.0);
        assert_eq!(PUT, DISPATCH_PROPERTYPUT.0);
        assert_eq!(PUTREF, DISPATCH_PROPERTYPUTREF.0);
        assert_eq!(DISPID_UNKNOWN, windows::Win32::System::Ole::DISPID_UNKNOWN);
    }
}
