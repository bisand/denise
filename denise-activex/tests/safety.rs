//! The safety claim, asked the way a host asks it.
//!
//! The arithmetic underneath is in `src/safety.rs` and runs on every platform.
//! What is left for here is the part that needs interface ids: that the right
//! question is recognised from each of them, and that the answers come back
//! through a real vtable rather than out of a table.
//!
//! Worth being blunt about what these tests do and do not establish. They check
//! that the control *says* what it means to say. Whether the claim is true is a
//! question about the members in `dispatch::MEMBERS`, and the answer is an
//! argument written down in `src/safety.rs`, not something a test can settle.

#![cfg(windows)]

use denise_activex::DenisePanel;
use denise_activex::safety::{FOR_UNTRUSTED_CALLER, FOR_UNTRUSTED_DATA};
use windows::Win32::Foundation::{E_FAIL, E_NOINTERFACE, E_POINTER};
use windows::Win32::System::Com::{IDispatch, IPersist, IPersistStreamInit};
use windows::Win32::System::Diagnostics::Debug::IObjectSafety;
use windows::Win32::System::Ole::{IOleObject, IViewObject2};
use windows_core::{GUID, Interface};

fn panel() -> IObjectSafety {
    DenisePanel::new().into()
}

/// Reports both numbers, or the error.
fn options(object: &IObjectSafety, riid: &GUID) -> windows_core::Result<(u32, u32)> {
    let mut supported = 0u32;
    let mut enabled = 0u32;
    // SAFETY: `riid` and both counters are live locals owned by the caller.
    unsafe { object.GetInterfaceSafetyOptions(riid, &mut supported, &mut enabled) }?;
    Ok((supported, enabled))
}

/// The scripting question, asked about the scripting interface.
///
/// Supported and enabled are equal because there is no mode to switch into: the
/// control is safe because of what its four members do. A control that reported
/// a guarantee as supported-but-disabled would be telling a host to ask again.
#[test]
fn the_automation_interface_is_claimed_safe_for_untrusted_callers() {
    let (supported, enabled) = options(&panel(), &IDispatch::IID).expect("IDispatch");
    assert_eq!(supported, FOR_UNTRUSTED_CALLER);
    assert_eq!(enabled, supported, "nothing here has to be switched on");
}

/// The data question, asked about the persistence interfaces. Both of them: a
/// host may name the derived interface or the base.
#[test]
fn the_persistence_interfaces_are_claimed_safe_for_untrusted_data() {
    for riid in [IPersistStreamInit::IID, IPersist::IID] {
        let (supported, enabled) = options(&panel(), &riid).expect("a persistence interface");
        assert_eq!(supported, FOR_UNTRUSTED_DATA, "{riid:?}");
        assert_eq!(enabled, supported);
    }
}

/// The two claims stay separate. Answering both bits everywhere would make the
/// interface useless to ask — a host learns nothing from an object that says yes
/// to every question.
#[test]
fn neither_claim_leaks_into_the_other_interface() {
    let (scripting, _) = options(&panel(), &IDispatch::IID).expect("IDispatch");
    let (data, _) = options(&panel(), &IPersistStreamInit::IID).expect("IPersistStreamInit");
    assert_eq!(scripting & FOR_UNTRUSTED_DATA, 0);
    assert_eq!(data & FOR_UNTRUSTED_CALLER, 0);
}

/// An interface the control implements but makes no safety claim about.
///
/// `IOleObject` and `IViewObject2` are container interfaces, not scripting ones,
/// and a host that reaches them is already trusted enough to site a control. The
/// refusal is the honest answer: silence would be read as a claim.
#[test]
fn an_interface_with_no_claim_is_refused_rather_than_waved_through() {
    for riid in [IOleObject::IID, IViewObject2::IID, GUID::zeroed()] {
        let error = options(&panel(), &riid).expect_err("no claim about this one");
        assert_eq!(error.code(), E_NOINTERFACE, "{riid:?}");
    }
}

/// What a scripting host actually does: ask for the guarantee before using the
/// object.
#[test]
fn a_host_asking_for_the_guarantee_on_offer_is_told_yes() {
    let panel = panel();
    // SAFETY: `riid` is a live local; the two masks are values.
    unsafe {
        panel.SetInterfaceSafetyOptions(&IDispatch::IID, FOR_UNTRUSTED_CALLER, FOR_UNTRUSTED_CALLER)
    }
    .expect("the guarantee this control makes");
}

/// And the one that has to fail. A host asking whether untrusted *data* is safe
/// through the *scripting* interface is asking about a guarantee that was never
/// offered, and `S_OK` would be claiming it.
#[test]
fn a_host_asking_for_a_guarantee_never_offered_is_refused() {
    let panel = panel();
    // SAFETY: as above.
    let result = unsafe {
        panel.SetInterfaceSafetyOptions(&IDispatch::IID, FOR_UNTRUSTED_DATA, FOR_UNTRUSTED_DATA)
    };
    assert_eq!(result.expect_err("not on offer").code(), E_FAIL);

    // Including an option nobody has defined: a future guarantee this control
    // has never heard of is one it cannot be honouring.
    // SAFETY: as above.
    let result =
        unsafe { panel.SetInterfaceSafetyOptions(&IDispatch::IID, 0x8000_0000, 0x8000_0000) };
    assert_eq!(result.expect_err("an unknown option").code(), E_FAIL);
}

/// Turning a guarantee off is not an error. There is no unsafe mode to switch
/// into, so refusing would be inventing one.
#[test]
fn a_host_switching_the_guarantee_off_is_not_refused() {
    let panel = panel();
    // SAFETY: as above.
    unsafe { panel.SetInterfaceSafetyOptions(&IDispatch::IID, FOR_UNTRUSTED_CALLER, 0) }
        .expect("still safe either way");
}

/// The out-parameters are raw pointers from somebody else's code.
#[test]
fn null_pointers_are_refused_rather_than_written_through() {
    let panel = panel();
    let mut value = 0u32;

    // SAFETY: passing the nulls the test is about; `value` is a live local.
    let missing_enabled = unsafe {
        panel.GetInterfaceSafetyOptions(&IDispatch::IID, &mut value, core::ptr::null_mut())
    };
    // SAFETY: as above.
    let missing_supported = unsafe {
        panel.GetInterfaceSafetyOptions(&IDispatch::IID, core::ptr::null_mut(), &mut value)
    };
    // SAFETY: a null interface id, which is not a readable GUID.
    let missing_riid =
        unsafe { panel.GetInterfaceSafetyOptions(core::ptr::null(), &mut value, &mut value) };

    assert_eq!(missing_enabled.expect_err("null").code(), E_POINTER);
    assert_eq!(missing_supported.expect_err("null").code(), E_POINTER);
    assert_eq!(
        missing_riid.expect_err("null").code(),
        E_NOINTERFACE,
        "no interface was named, so there is no claim to report"
    );
}

/// The registry and the interface are two halves of one claim, and hosts are
/// split on which they ask. Between them, the interfaces the control answers for
/// must cover both categories it registers — otherwise it is safe for scripting
/// according to the registry and silent when asked.
#[test]
fn the_interface_covers_every_category_the_registry_claims() {
    let panel = panel();
    let mut claimed = 0u32;
    for riid in [IDispatch::IID, IPersistStreamInit::IID] {
        claimed |= options(&panel, &riid).expect("a claimed interface").0;
    }

    let entries = denise_activex::registry::entries("C:\\denise_activex.dll");
    let registered = |catid: &str| entries.iter().any(|e| e.key.ends_with(catid));

    assert_eq!(
        registered(denise_activex::safety::CATID_SAFE_FOR_SCRIPTING),
        claimed & FOR_UNTRUSTED_CALLER != 0,
        "the registry and the interface disagree about scripting"
    );
    assert_eq!(
        registered(denise_activex::safety::CATID_SAFE_FOR_INITIALIZING),
        claimed & FOR_UNTRUSTED_DATA != 0,
        "the registry and the interface disagree about initialisation"
    );
}
