//! Reading and writing the one type every scripting host speaks.
//!
//! A `VARIANT` is a tagged union, and the tag is whatever the host felt like
//! sending. VBScript passes every string as `VT_BSTR`, VB6 passes an integer
//! property as `VT_I2`, PowerShell passes `$true` as `VT_BOOL` and `1` as
//! `VT_I4`, and a control that insists on one of them rejects perfectly ordinary
//! script. So nothing here inspects the tag: everything goes through
//! `VariantChangeType`, which is the coercion the whole automation ecosystem
//! agrees on.
//!
//! The one asymmetry worth naming is booleans. `VARIANT_TRUE` is **-1**, not 1 —
//! Basic's `True` is all bits set. Writing uses the constant so there is nothing
//! to get wrong; reading tests against zero, so a host that sends 1 is understood
//! as well as one that sends -1.

use core::mem::ManuallyDrop;

use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Variant::{
    VAR_CHANGE_FLAGS, VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR,
    VariantChangeType, VariantClear,
};
use windows_core::BSTR;

/// Assembles a variant from its tag and one arm of its union.
///
/// Built whole rather than field by field: a `VARIANT` is three nested unions
/// deep, and assigning through them means writing *over* whatever arm was there
/// before — which is how a `BSTR` gets leaked or freed twice. Constructing the
/// value once has neither problem, and needs no `unsafe` at all.
fn assemble(vt: VARENUM, payload: VARIANT_0_0_0) -> VARIANT {
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

/// Reads a variant as a string, coercing whatever the host actually sent.
///
/// # Safety
///
/// `value` must be a variant the caller owns and keeps valid for the call.
pub unsafe fn to_string(value: &VARIANT) -> windows_core::Result<String> {
    let mut coerced = VARIANT::default();
    // SAFETY: `coerced` is a live, zeroed (`VT_EMPTY`) variant, which is what
    // `VariantChangeType` requires of its destination, and `value` is the
    // caller's.
    unsafe { VariantChangeType(&mut coerced, value, VAR_CHANGE_FLAGS(0), VT_BSTR)? };

    // SAFETY: the coercion succeeded, so the union holds a `BSTR`. A null one is
    // a legitimate empty string, which `BSTR` already reads as `""`.
    let text = unsafe { coerced.Anonymous.Anonymous.Anonymous.bstrVal.to_string() };

    // SAFETY: `coerced` is ours and holds the only reference to that `BSTR`.
    unsafe {
        let _ = VariantClear(&mut coerced);
    }
    Ok(text)
}

/// Reads a variant as a boolean, coercing whatever the host actually sent.
///
/// # Safety
///
/// As [`to_string`].
pub unsafe fn to_bool(value: &VARIANT) -> windows_core::Result<bool> {
    let mut coerced = VARIANT::default();
    // SAFETY: as in `to_string`.
    unsafe { VariantChangeType(&mut coerced, value, VAR_CHANGE_FLAGS(0), VT_BOOL)? };
    // SAFETY: the coercion succeeded, so the union holds a `VARIANT_BOOL`.
    // Compared against zero rather than against `VARIANT_TRUE`: Basic's `True` is
    // -1 and a C host's is 1, and both mean true.
    let value = unsafe { coerced.Anonymous.Anonymous.Anonymous.boolVal.0 != 0 };
    // SAFETY: nothing in a `VT_BOOL` needs freeing, but clearing is what keeps
    // this symmetrical with `to_string` if the type ever changes.
    unsafe {
        let _ = VariantClear(&mut coerced);
    }
    Ok(value)
}

/// Writes a string into a caller's out-variant, doing nothing if it is null.
///
/// A null `pVarResult` means the host is not interested in the answer, which is
/// legal and which a control that dereferences it turns into a crash.
///
/// # Safety
///
/// `out` must be null or point to a writable variant the caller has initialised
/// to `VT_EMPTY`, as the `IDispatch` contract requires.
pub unsafe fn write_string(out: *mut VARIANT, text: &str) {
    if out.is_null() {
        return;
    }
    // The `BSTR` sits in a `ManuallyDrop`, so Rust does not free the string the
    // caller is about to own — and free itself, with `VariantClear`.
    let value = assemble(
        VT_BSTR,
        VARIANT_0_0_0 {
            bstrVal: ManuallyDrop::new(BSTR::from(text)),
        },
    );
    // SAFETY: `out` is non-null and the caller promises it is writable.
    unsafe { out.write(value) };
}

/// Writes a boolean into a caller's out-variant, doing nothing if it is null.
///
/// # Safety
///
/// As [`write_string`].
pub unsafe fn write_bool(out: *mut VARIANT, value: bool) {
    if out.is_null() {
        return;
    }
    // `VARIANT_TRUE` rather than a literal: it is -1, and every hand-written 1 in
    // this position is a bug that only ever shows up in Basic.
    let variant = assemble(
        VT_BOOL,
        VARIANT_0_0_0 {
            boolVal: if value { VARIANT_TRUE } else { VARIANT_FALSE },
        },
    );
    // SAFETY: `out` is non-null and the caller promises it is writable.
    unsafe { out.write(variant) };
}

/// The one positional argument of a property put.
///
/// `DISPPARAMS` holds its arguments **backwards** — last first — and a put also
/// carries a named argument, `DISPID_PROPERTYPUT`, describing which of them is
/// the value. For a single-argument put both conventions land on the same slot,
/// which is exactly why reading `rgvarg[0]` looks right in every example and is
/// only right by accident. Naming it here keeps that fact in one place.
///
/// # Safety
///
/// `params` must be the `DISPPARAMS` the host passed to `Invoke`.
pub unsafe fn sole_argument<'a>(
    params: *const windows::Win32::System::Com::DISPPARAMS,
) -> Option<&'a VARIANT> {
    if params.is_null() {
        return None;
    }
    // SAFETY: the host promises a readable `DISPPARAMS` for the call.
    let params = unsafe { &*params };
    if params.cArgs == 0 || params.rgvarg.is_null() {
        return None;
    }
    // SAFETY: `cArgs` is at least one, so `rgvarg[0]` is readable. The lifetime
    // is unbound and the caller chooses it: every caller in this crate consumes
    // the borrow inside the `Invoke` that produced it, which is exactly as long
    // as the host guarantees the memory.
    unsafe { params.rgvarg.as_ref() }
}
