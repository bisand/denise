//! The dispatch path, driven the way a script drives it.
//!
//! `examples/host.rs` proves this too, and needs a person, a registered DLL and a
//! window. These need none of those: the control is constructed directly and
//! asked over its own `IDispatch`, so they run on a CI runner with nothing
//! installed — which is what makes them run on every commit.
//!
//! What they are really here for is `variant.rs`. Every value a script assigns
//! arrives as a `VARIANT`, and the crate builds and reads them by hand through
//! three nested unions. That code is unit-testable nowhere: it needs `oleaut32`
//! to coerce, and a real `Invoke` at either end to mean anything.

#![cfg(windows)]

use std::mem::ManuallyDrop;

use denise_activex::DenisePanel;
use denise_activex::dispatch;
use windows::Win32::Foundation::{DISP_E_UNKNOWNNAME, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, IDispatch,
};
use windows::Win32::System::Ole::DISPID_PROPERTYPUT;
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_I4, VariantClear,
};
use windows_core::{BSTR, GUID, PCWSTR};

fn panel() -> IDispatch {
    DenisePanel::new().into()
}

/// The control answers with type information exactly when it has some.
///
/// This started life asserting zero, back when there was no library and the
/// comment said that anyone changing it was taking on writing one. Somebody did.
/// What survives is the rule underneath: claiming type information and then
/// failing `GetTypeInfo` is worse than declining, because a host that would have
/// fallen back to `GetIDsOfNames` gives up instead.
///
/// So this proves both halves, using the file the server would have written. The
/// library lands beside the test binary, which is where `GetTypeInfo` looks when
/// nothing is registered — the same fallback a control gets in a build tree.
#[test]
fn the_control_describes_itself_once_the_library_is_beside_it() {
    let beside = std::env::current_exe()
        .expect("the test binary's path")
        .with_extension("tlb");
    let _ = std::fs::remove_file(&beside);

    // Nothing to read: it must say so rather than promise and fail.
    // SAFETY: `panel` is a live object owned for the call.
    let count = unsafe { panel().GetTypeInfoCount() }.expect("GetTypeInfoCount");
    assert_eq!(
        count, 0,
        "with no library anywhere, promising one and then failing GetTypeInfo \
         makes a host give up instead of falling back to GetIDsOfNames"
    );

    denise_activex::typelib::build(&beside.to_string_lossy()).expect("build the library");

    let panel = panel();
    // SAFETY: as above.
    let count = unsafe { panel.GetTypeInfoCount() }.expect("GetTypeInfoCount");
    assert_eq!(count, 1, "the library is right there");

    // SAFETY: index zero is the only description a control with one has.
    let info = unsafe { panel.GetTypeInfo(0, 0) }.expect("GetTypeInfo");
    // The name a script writes, resolved through the description rather than
    // through the object — which is the path PowerShell takes and the whole
    // reason the library exists.
    let wide: Vec<u16> = "caption"
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let mut dispid = 0i32;
    // SAFETY: `wide` outlives the call; one name in, one dispid out.
    unsafe { info.GetIDsOfNames(&PCWSTR(wide.as_ptr()), 1, &mut dispid) }.expect("Caption");
    assert_eq!(dispid, dispatch::DISPID_CAPTION);

    let _ = std::fs::remove_file(&beside);
}

/// The path every OLE container takes. Case-insensitively, because Basic is.
#[test]
fn every_member_resolves_by_name_through_the_real_dispatch() {
    let panel = panel();
    for member in dispatch::MEMBERS {
        for spelling in [member.name.to_ascii_lowercase(), member.name.to_string()] {
            assert_eq!(
                dispid(&panel, &spelling).expect("a known name"),
                member.dispid,
                "{spelling} resolved wrongly"
            );
        }
    }
}

/// A host that asks for a name this control does not have must be told so, not
/// handed a dispid that happens to be nearby.
#[test]
fn an_unknown_name_is_refused_with_the_documented_error() {
    let error = dispid(&panel(), "Nonsense").expect_err("an unknown name");
    assert_eq!(error.code(), DISP_E_UNKNOWNNAME);
}

/// A string through the whole width of the plumbing: a `BSTR` in a `VARIANT` the
/// caller built, into `Invoke`, out of the model, back into a `VARIANT` the
/// control built, and read.
///
/// Non-ASCII on purpose. A `BSTR` is UTF-16 and the model is a Rust `String`, so
/// every crossing is a conversion, and `æøå` is what this project is for.
#[test]
fn a_string_survives_being_assigned_and_read_back() {
    let panel = panel();
    put(&panel, "Caption", string_variant("Blåbærsyltetøy")).expect("put Caption");
    assert_eq!(get_string(&panel, "Caption"), "Blåbærsyltetøy");

    // Empty is its own case: a zero-length `BSTR` is legitimately a null pointer,
    // and code that reads one without expecting that crashes on a cleared field.
    put(&panel, "Text", string_variant("")).expect("put empty Text");
    assert_eq!(get_string(&panel, "Text"), "");
}

/// Basic's `True` is -1 and everyone else's is 1. The control writes the constant
/// so there is nothing to get wrong, and reads against zero so both are
/// understood — which is what this checks from the outside.
#[test]
fn a_boolean_survives_in_both_of_its_spellings() {
    let panel = panel();

    put(&panel, "Enabled", bool_variant(false)).expect("put Enabled");
    assert!(!get_bool(&panel, "Enabled"));

    put(&panel, "Enabled", bool_variant(true)).expect("put Enabled");
    assert!(get_bool(&panel, "Enabled"));

    // What a C host or a careless script sends: 1, not -1, and as an integer
    // rather than a boolean at all. `VariantChangeType` is the reason both work.
    put(&panel, "Enabled", variant(VT_I4, VARIANT_0_0_0 { lVal: 0 }))
        .expect("put Enabled as an integer");
    assert!(!get_bool(&panel, "Enabled"));
}

/// The coercion is the point of going through `VariantChangeType` rather than
/// inspecting the tag: a script that assigns a number to a string property is
/// doing something ordinary, and a control that refuses it is being difficult.
#[test]
fn a_number_assigned_to_a_string_property_is_coerced_rather_than_refused() {
    let panel = panel();
    put(&panel, "Text", variant(VT_I4, VARIANT_0_0_0 { lVal: 42 })).expect("put Text");
    assert_eq!(get_string(&panel, "Text"), "42");
}

/// A method, which takes nothing and returns nothing. Without a window there is
/// nothing to repaint, and succeeding anyway is correct: a script may call this
/// before a container has sited the control.
#[test]
fn refresh_is_callable_with_no_window_to_refresh() {
    let panel = panel();
    let dispid = dispid(&panel, "Refresh").expect("Refresh");
    let params = DISPPARAMS::default();
    // SAFETY: `params` describes no arguments and no result is wanted.
    unsafe {
        panel.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            DISPATCH_METHOD,
            &params,
            None,
            None,
            None,
        )
    }
    .expect("Refresh");
}

// ------------------------------------------------------------------- the helpers

fn dispid(object: &IDispatch, name: &str) -> windows_core::Result<i32> {
    let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let name = PCWSTR(wide.as_ptr());
    let mut dispid = 0i32;
    // SAFETY: `riid` is reserved and must be `IID_NULL`; `wide` outlives the call
    // and one dispid is written.
    unsafe { object.GetIDsOfNames(&GUID::zeroed(), &name, 1, 0, &mut dispid) }?;
    Ok(dispid)
}

/// Assigns, with the named argument and the reversed array a put carries.
fn put(object: &IDispatch, name: &str, mut argument: VARIANT) -> windows_core::Result<()> {
    let dispid = dispid(object, name)?;
    let mut named = DISPID_PROPERTYPUT;
    let params = DISPPARAMS {
        rgvarg: &mut argument,
        rgdispidNamedArgs: &mut named,
        cArgs: 1,
        cNamedArgs: 1,
    };
    // SAFETY: `params` borrows two live locals and describes them correctly.
    let result = unsafe {
        object.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            DISPATCH_PROPERTYPUT,
            &params,
            None,
            None,
            None,
        )
    };
    // SAFETY: `argument` is ours and holds the only reference to its payload.
    unsafe {
        let _ = VariantClear(&mut argument);
    }
    result
}

fn get(object: &IDispatch, name: &str) -> VARIANT {
    let dispid = dispid(object, name).expect("a known name");
    let params = DISPPARAMS::default();
    let mut result = VARIANT::default();
    // SAFETY: `params` describes no arguments and `result` is a live local
    // initialised to `VT_EMPTY`, which is what the contract asks of a caller.
    unsafe {
        object.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            DISPATCH_PROPERTYGET,
            &params,
            Some(&mut result),
            None,
            None,
        )
    }
    .expect("a property get");
    result
}

fn get_string(object: &IDispatch, name: &str) -> String {
    let mut result = get(object, name);
    // SAFETY: the control answers these with a `VT_BSTR`, and a null `BSTR` reads
    // as the empty string.
    let text = unsafe { result.Anonymous.Anonymous.Anonymous.bstrVal.to_string() };
    // SAFETY: `result` is ours and the payload is the caller's to free.
    unsafe {
        let _ = VariantClear(&mut result);
    }
    text
}

fn get_bool(object: &IDispatch, name: &str) -> bool {
    let result = get(object, name);
    // SAFETY: the control answers `Enabled` with a `VT_BOOL`. Against zero, not
    // against 1: Basic's `True` is -1.
    unsafe { result.Anonymous.Anonymous.Anonymous.boolVal.0 != 0 }
}

fn string_variant(text: &str) -> VARIANT {
    variant(
        VT_BSTR,
        VARIANT_0_0_0 {
            bstrVal: ManuallyDrop::new(BSTR::from(text)),
        },
    )
}

fn bool_variant(value: bool) -> VARIANT {
    variant(
        VT_BOOL,
        VARIANT_0_0_0 {
            boolVal: if value { VARIANT_TRUE } else { VARIANT_FALSE },
        },
    )
}

/// Assembled whole rather than field by field: a `VARIANT` is three nested unions
/// deep, and assigning through them writes *over* whatever arm was there before.
fn variant(vt: VARENUM, payload: VARIANT_0_0_0) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: payload,
            }),
        },
    }
}
