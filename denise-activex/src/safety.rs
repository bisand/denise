//! The scripting safety claim, and what it actually asserts.
//!
//! `IObjectSafety` is a control telling a host two separate things:
//!
//! - **Safe for scripting.** Untrusted script may call this object's automation
//!   surface without being able to do anything it could not do on its own.
//! - **Safe for initialising.** Untrusted data may be loaded into it — a
//!   persisted stream from a page nobody wrote — without the same.
//!
//! Both are assertions about *this* control, not about COM, and a control that
//! makes them carelessly is the reason ActiveX has the reputation it does. So it
//! is worth writing down why they hold here.
//!
//! The whole scriptable surface is `Text`, `Caption`, `Enabled` and `Refresh`:
//! two strings, a boolean and a repaint. Nothing in it opens a file, spawns a
//! process, reads the registry, resolves a host name, or hands out a pointer.
//! `Load` reads nothing at all — there are no persisted properties yet — so
//! "untrusted data" currently has nothing to be untrusted *with*. A script that
//! drives this control to its limit has changed some text on a panel.
//!
//! # The condition this depends on
//!
//! That is a claim about the automation surface as it stands, and it stops being
//! true the moment a member reaches outside the control. A property that names a
//! file, a method that runs something, anything that takes a pointer or a window
//! handle from the caller — any one of those and this claim has to be re-argued
//! rather than inherited. [`dispatch::MEMBERS`](crate::dispatch::MEMBERS) is the
//! list to check it against, and it is short on purpose.
//!
//! # Two halves that have to agree
//!
//! A host may ask the object ([`IObjectSafety`]) or ask the registry (the
//! component categories below), and some ask only one. Both are written here so
//! they cannot drift: the categories are in
//! [`registry::entries`](crate::registry::entries) and the interface answers from
//! the same table.
//!
//! [`IObjectSafety`]: https://learn.microsoft.com/en-us/previous-versions/windows/internet-explorer/ie-developer/platform-apis/aa768223(v=vs.85)

/// `INTERFACESAFE_FOR_UNTRUSTED_CALLER` — untrusted script may call it.
pub const FOR_UNTRUSTED_CALLER: u32 = 0x0000_0001;

/// `INTERFACESAFE_FOR_UNTRUSTED_DATA` — untrusted data may initialise it.
pub const FOR_UNTRUSTED_DATA: u32 = 0x0000_0002;

/// `CATID_SafeForScripting`, as the registry spells it.
pub const CATID_SAFE_FOR_SCRIPTING: &str = "{7DD95801-9882-11CF-9FA9-00AA006C42C4}";

/// `CATID_SafeForInitializing`.
pub const CATID_SAFE_FOR_INITIALIZING: &str = "{7DD95802-9882-11CF-9FA9-00AA006C42C4}";

/// What a host is asking about, once the interface id has been recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A scripting interface — `IDispatch` and its relatives. The question is
    /// whether untrusted *script* may drive it.
    Automation,
    /// A persistence interface — the `IPersist*` family. The question is whether
    /// untrusted *data* may initialise it.
    Persistence,
    /// Anything else the control implements, and anything it does not.
    Other,
}

/// The options this control claims for an interface.
///
/// Zero means no claim, which a host must read as "not safe" rather than as
/// "safe by default" — and which the caller turns into `E_NOINTERFACE`.
///
/// The two claims are deliberately not merged. Answering `FOR_UNTRUSTED_DATA`
/// for `IDispatch` would be claiming something about a question nobody asked,
/// and a control that says yes to everything is one a host cannot learn anything
/// from.
pub const fn supported(asked: Asked) -> u32 {
    match asked {
        Asked::Automation => FOR_UNTRUSTED_CALLER,
        Asked::Persistence => FOR_UNTRUSTED_DATA,
        Asked::Other => 0,
    }
}

/// Whether a host's requested change is one this control can honour.
///
/// `SetInterfaceSafetyOptions` passes a mask of the options it wants to set and
/// the values to set them to. A mask naming an option the control does not
/// support is the failure — the host is asking about a guarantee that was never
/// offered, and `S_OK` would be a lie.
///
/// A request that *disables* a safety option is honoured rather than refused.
/// The control is safe whether or not anybody asked it to be, and there is no
/// mode here to switch out of.
pub const fn accepts(supported: u32, mask: u32) -> bool {
    mask & !supported == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two questions are answered separately, because they are two
    /// questions. A control that returns both bits for every interface has told
    /// the host nothing.
    #[test]
    fn each_kind_of_interface_gets_the_claim_that_matches_the_question() {
        assert_eq!(supported(Asked::Automation), FOR_UNTRUSTED_CALLER);
        assert_eq!(supported(Asked::Persistence), FOR_UNTRUSTED_DATA);
        assert_eq!(
            supported(Asked::Automation) & FOR_UNTRUSTED_DATA,
            0,
            "claiming safety for data on a scripting interface answers a question \
             the host did not ask"
        );
    }

    /// An interface with no claim is not a safe one. The absence has to stay an
    /// absence: a host reads zero as "no", and anything else here would be a
    /// default that nobody argued for.
    #[test]
    fn an_interface_with_no_claim_offers_nothing() {
        assert_eq!(supported(Asked::Other), 0);
        assert!(!accepts(supported(Asked::Other), FOR_UNTRUSTED_CALLER));
    }

    /// The ordinary case: a host asks for exactly the guarantee on offer.
    #[test]
    fn a_host_asking_for_what_is_offered_is_told_yes() {
        let scripting = supported(Asked::Automation);
        assert!(accepts(scripting, FOR_UNTRUSTED_CALLER));
        assert!(accepts(scripting, 0), "asking for nothing always succeeds");
    }

    /// And the case that must fail: a host asking for a guarantee this control
    /// does not make about that interface. Answering `S_OK` would be claiming it.
    #[test]
    fn a_host_asking_for_a_guarantee_that_was_never_offered_is_refused() {
        let scripting = supported(Asked::Automation);
        assert!(!accepts(scripting, FOR_UNTRUSTED_DATA));
        assert!(!accepts(
            scripting,
            FOR_UNTRUSTED_CALLER | FOR_UNTRUSTED_DATA
        ));

        // Including bits nobody has defined. A future option this control has
        // never heard of is one it cannot possibly be honouring.
        assert!(!accepts(scripting, 0x8000_0000));
    }

    /// Turning safety *off* is not a request this control can fail: there is no
    /// unsafe mode to switch into, so the honest answer is yes.
    #[test]
    fn a_host_switching_a_guarantee_off_is_not_an_error() {
        assert!(accepts(supported(Asked::Automation), FOR_UNTRUSTED_CALLER));
    }

    /// The two category ids, which a host may read instead of ever calling the
    /// interface. Well-formed and distinct — one digit apart, which is exactly
    /// the kind of pair that gets copied wrong.
    #[test]
    fn the_two_category_ids_are_distinct_and_well_formed() {
        for catid in [CATID_SAFE_FOR_SCRIPTING, CATID_SAFE_FOR_INITIALIZING] {
            assert_eq!(catid.len(), 38, "{catid} is not a braced GUID");
            assert!(catid.starts_with('{') && catid.ends_with('}'));
            assert_eq!(catid, catid.to_uppercase());
        }
        assert_ne!(CATID_SAFE_FOR_SCRIPTING, CATID_SAFE_FOR_INITIALIZING);
    }
}
