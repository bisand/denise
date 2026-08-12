//! The description a host reads before it will ask the control anything.
//!
//! `examples/host.rs` proves `IDispatch` works, and proves nothing about this:
//! a container written in Rust calls `GetIDsOfNames` directly and never looks at
//! a type description. PowerShell does the opposite — it builds its member table
//! from `ITypeInfo` and will not ask for a name it has not been told about — and
//! that is the half nothing in this repository was checking.
//!
//! So these read the description back the way a host does, and compare it against
//! the table it was built from. What that cannot prove is that PowerShell's
//! adapter is *satisfied* by it; only PowerShell can say that. What it does prove
//! is that the description exists, is well formed, and says what `Invoke`
//! implements — which is every way it could be wrong that is this code's fault.
//!
//! No registration and no window: the control is constructed directly and asked
//! over its own `IDispatch`, so this runs on a CI runner with nothing installed.

#![cfg(windows)]

use denise_activex::DenisePanel;
use denise_activex::dispatch::{self, Action};
use windows::Win32::System::Com::{
    IDispatch, INVOKE_FUNC, INVOKE_PROPERTYGET, INVOKE_PROPERTYPUT, TKIND_DISPATCH,
};
use windows::Win32::System::Variant::VT_EMPTY;
use windows_core::{BSTR, GUID, PCWSTR};

/// The control, reached the way a host reaches it.
fn panel() -> IDispatch {
    let panel: IDispatch = DenisePanel::new().into();
    panel
}

/// The whole reason this file exists. Zero here is what leaves PowerShell
/// adapting the control as a bare `System.__ComObject` with no members, so every
/// property fails before a single COM call is made.
#[test]
fn the_control_admits_to_having_type_information() {
    // SAFETY: `panel` is a live object owned for the length of the call.
    let count = unsafe { panel().GetTypeInfoCount() }.expect("GetTypeInfoCount");
    assert_eq!(count, 1, "an object that says zero is never asked anything");
}

/// One dispinterface, and it has to be a dispinterface: a host that found
/// `TKIND_INTERFACE` here would look for a vtable that does not exist.
#[test]
fn the_description_is_a_dispinterface_with_one_entry_per_operation() {
    let panel = panel();
    // SAFETY: index zero is the only type information this control has.
    let info = unsafe { panel.GetTypeInfo(0, 0) }.expect("GetTypeInfo");

    // SAFETY: `info` is live, and the attributes are released below — they are a
    // borrow of the type information's own memory, not a copy.
    let attributes = unsafe { info.GetTypeAttr() }.expect("GetTypeAttr");
    // SAFETY: the pointer came from `GetTypeAttr` and is readable until released.
    let (kind, functions) = unsafe { ((*attributes).typekind, (*attributes).cFuncs) };
    // SAFETY: paired with the `GetTypeAttr` above.
    unsafe { info.ReleaseTypeAttr(attributes) };

    assert_eq!(kind, TKIND_DISPATCH);
    assert_eq!(
        functions as usize,
        dispatch::entries().len(),
        "the description and the table it was built from disagree about how many \
         operations there are"
    );
}

/// The description has to say what `Invoke` actually implements. A get described
/// as a put, or a put described as taking no arguments, is a host marshalling the
/// wrong thing at a control that will refuse it.
#[test]
fn every_entry_is_described_as_the_operation_it_is() {
    let panel = panel();
    // SAFETY: index zero is the only type information this control has.
    let info = unsafe { panel.GetTypeInfo(0, 0) }.expect("GetTypeInfo");

    for (index, entry) in dispatch::entries().iter().enumerate() {
        // SAFETY: `index` is below `cFuncs`, which the test above pins to the
        // same length.
        let description = unsafe { info.GetFuncDesc(index as u32) }.expect("GetFuncDesc");
        // SAFETY: the pointer came from `GetFuncDesc` and is readable until
        // released below.
        let (memid, invoke, parameters, returns) = unsafe {
            (
                (*description).memid,
                (*description).invkind,
                (*description).cParams,
                (*description).elemdescFunc.tdesc.vt,
            )
        };
        // SAFETY: paired with the `GetFuncDesc` above.
        unsafe { info.ReleaseFuncDesc(description) };

        assert_eq!(memid, entry.dispid, "{} has the wrong dispid", entry.name);

        let expected = match entry.flags {
            dispatch::GET => INVOKE_PROPERTYGET,
            dispatch::PUT => INVOKE_PROPERTYPUT,
            _ => INVOKE_FUNC,
        };
        assert_eq!(invoke, expected, "{} is described wrongly", entry.name);
        assert_eq!(
            parameters as u32, entry.arguments,
            "{} takes the wrong number of arguments",
            entry.name
        );

        // The return type, which is where this went wrong once. A put and a
        // method that returns nothing are `VT_VOID`; `VT_EMPTY` is a variant
        // holding nothing, which is a *value*, and a host told to expect one from
        // a call that hands it none unwraps a null.
        let expected = if entry.flags == dispatch::PUT {
            dispatch::VOID
        } else {
            entry.vt
        };
        assert_eq!(
            returns.0, expected,
            "{} is described as returning the wrong type",
            entry.name
        );
        assert_ne!(
            returns, VT_EMPTY,
            "{} claims to return VT_EMPTY, which is not a return type",
            entry.name
        );

        // The name, which is the only thing a host has to go on.
        let mut names = [BSTR::default()];
        let mut written = 0u32;
        // SAFETY: `names` is a live array of one and `written` a live local.
        unsafe { info.GetNames(memid, &mut names, &mut written) }.expect("GetNames");
        assert!(written >= 1);
        assert_eq!(names[0].to_string(), entry.name);
    }
}

/// The closest thing to what PowerShell does: look a name up in the description
/// rather than on the object. A member the description cannot name is a member a
/// host will never invoke, however well `IDispatch::GetIDsOfNames` works.
#[test]
fn the_description_resolves_the_names_a_script_would_write() {
    let panel = panel();
    // SAFETY: index zero is the only type information this control has.
    let info = unsafe { panel.GetTypeInfo(0, 0) }.expect("GetTypeInfo");

    for member in dispatch::MEMBERS {
        // Lower case on purpose: a script writes `panel.caption`, and neither
        // lookup is case-sensitive.
        let wide: Vec<u16> = member
            .name
            .to_ascii_lowercase()
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let name = PCWSTR(wide.as_ptr());
        let mut dispid = 0i32;
        // SAFETY: `wide` outlives the call; one name in, one dispid out.
        unsafe { info.GetIDsOfNames(&name, 1, &mut dispid) }
            .unwrap_or_else(|e| panic!("the description cannot name {}: {e}", member.name));
        assert_eq!(dispid, member.dispid, "{} resolved wrongly", member.name);
    }
}

/// And the object itself still answers, which is the path every container that is
/// not PowerShell takes. Both have to agree, or a host that uses one and then the
/// other invokes the wrong member.
#[test]
fn the_object_and_its_description_agree_about_every_dispid() {
    let panel = panel();
    for member in dispatch::MEMBERS {
        let wide: Vec<u16> = member
            .name
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let name = PCWSTR(wide.as_ptr());
        let mut dispid = 0i32;
        // SAFETY: `riid` is reserved and must be `IID_NULL`; `wide` outlives the
        // call and one dispid is written.
        unsafe { panel.GetIDsOfNames(&GUID::zeroed(), &name, 1, 0, &mut dispid) }
            .unwrap_or_else(|e| panic!("the object cannot name {}: {e}", member.name));
        assert_eq!(dispid, member.dispid);

        // And the table agrees with both, which is what `Invoke` reads.
        assert_eq!(
            dispatch::member_by_id(dispid).map(|found| found.name),
            Some(member.name)
        );
    }
}

/// Keeps `Action` named: the description above says which of these a host may
/// ask for, and `dispatch::action` is what decides it at the other end.
const _: fn(&dispatch::Member, u16) -> Option<Action> = dispatch::action;
