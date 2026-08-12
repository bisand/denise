//! The type library, built and then read back the way a host reads it.
//!
//! This is the file that has to exist. The last attempt at describing this control
//! to PowerShell was pushed with tests that failed on the Windows runner —
//! `STATUS_ACCESS_VIOLATION`, because the description outlived the buffers it
//! pointed at — and the CI log went unread for three rounds of asking somebody to
//! rebuild on a VM instead. So: build a library into a temporary file, load it,
//! and compare what comes back against the table it was generated from.
//!
//! `REGKIND_NONE` means none of this touches the registry, so it runs unprivileged
//! on a CI runner with nothing installed.

#![cfg(windows)]

use denise_activex::dispatch;
use denise_activex::typelib::{self, DIID_DENISE_PANEL, LIBID_DENISE};
use windows::Win32::System::Com::{
    INVOKE_FUNC, INVOKE_PROPERTYGET, INVOKE_PROPERTYPUT, ITypeLib, TKIND_COCLASS, TKIND_DISPATCH,
};
use windows::Win32::System::Ole::{LoadTypeLibEx, REGKIND_NONE};
use windows::Win32::System::Variant::VT_EMPTY;
use windows_core::{BSTR, GUID, PCWSTR};

/// Builds a library into the test's own temporary directory and loads it.
///
/// The file is named after the test so two of them running at once cannot fight
/// over it, which they will: cargo runs tests in parallel by default.
fn build_and_load(name: &str) -> (ITypeLib, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("denise-{name}.tlb"));
    let _ = std::fs::remove_file(&path);

    let text = path.to_string_lossy().to_string();
    typelib::build(&text).expect("build the library");
    assert!(path.exists(), "SaveAllChanges wrote nothing");

    let wide: Vec<u16> = text.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and live for the call. `REGKIND_NONE` loads
    // without registering, which is what makes this runnable without privileges.
    let library = unsafe { LoadTypeLibEx(PCWSTR(wide.as_ptr()), REGKIND_NONE) }
        .expect("load the library back");
    (library, path)
}

/// The three types a host expects to find, and the identity of each.
#[test]
fn the_library_holds_the_class_and_both_of_its_interfaces() {
    let (library, path) = build_and_load("shape");

    // SAFETY: `library` is live for the rest of this test.
    let count = unsafe { library.GetTypeInfoCount() };
    assert_eq!(count, 3, "the dispinterface, the events and the coclass");

    // SAFETY: as above; the attributes are released before the borrow ends.
    let attributes = unsafe { library.GetLibAttr() }.expect("library attributes");
    // SAFETY: the pointer came from `GetLibAttr` and is readable until released.
    let (guid, major, minor) = unsafe {
        (
            (*attributes).guid,
            (*attributes).wMajorVerNum,
            (*attributes).wMinorVerNum,
        )
    };
    // SAFETY: paired with the call above.
    unsafe { library.ReleaseTLibAttr(attributes) };
    assert_eq!(guid, LIBID_DENISE, "the library id a host stores");
    assert_eq!((major, minor), typelib::VERSION);

    // The class, and that it says it can be created — which is what makes
    // `New-Object -ComObject` and `CreateObject` mean anything.
    // SAFETY: `library` is live and the GUID outlives the call.
    let coclass = unsafe { library.GetTypeInfoOfGuid(&denise_activex::CLSID_DENISE_PANEL) }
        .expect("the coclass, by its class id");
    // SAFETY: released immediately below.
    let attributes = unsafe { coclass.GetTypeAttr() }.expect("coclass attributes");
    // SAFETY: readable until released.
    let (kind, implemented) = unsafe { ((*attributes).typekind, (*attributes).cImplTypes) };
    // SAFETY: paired.
    unsafe { coclass.ReleaseTypeAttr(attributes) };
    assert_eq!(kind, TKIND_COCLASS);
    assert_eq!(implemented, 2, "the dispinterface and the event source");

    let _ = std::fs::remove_file(path);
}

/// The one that matters for PowerShell: a **dispinterface**, not an interface.
///
/// `CreateDispTypeInfo` produced `TKIND_INTERFACE` here, which is why it could not
/// do this job however well formed the rest of it was.
#[test]
fn the_panel_interface_is_a_dispinterface_with_one_entry_per_operation() {
    let (library, path) = build_and_load("dispinterface");

    // SAFETY: `library` is live and the GUID outlives the call.
    let info = unsafe { library.GetTypeInfoOfGuid(&DIID_DENISE_PANEL) }.expect("the dispinterface");
    // SAFETY: released below.
    let attributes = unsafe { info.GetTypeAttr() }.expect("attributes");
    // SAFETY: readable until released.
    let (kind, functions) = unsafe { ((*attributes).typekind, (*attributes).cFuncs) };
    // SAFETY: paired.
    unsafe { info.ReleaseTypeAttr(attributes) };

    assert_eq!(
        kind, TKIND_DISPATCH,
        "a host looking for a dispinterface must find one"
    );
    assert_eq!(functions as usize, dispatch::entries().len());

    let _ = std::fs::remove_file(path);
}

/// Every entry, against the table it was generated from: the dispid a host will
/// invoke, whether it is a get, a put or a call, how many arguments it takes, what
/// it returns, and what it is called.
#[test]
fn every_entry_matches_the_table_it_was_generated_from() {
    let (library, path) = build_and_load("entries");
    // SAFETY: `library` is live and the GUID outlives the call.
    let info = unsafe { library.GetTypeInfoOfGuid(&DIID_DENISE_PANEL) }.expect("the dispinterface");

    for (index, entry) in dispatch::entries().iter().enumerate() {
        // SAFETY: `index` is below `cFuncs`, pinned by the test above.
        let description = unsafe { info.GetFuncDesc(index as u32) }.expect("a function");
        // SAFETY: readable until released below.
        let (memid, invoke, parameters, returns) = unsafe {
            (
                (*description).memid,
                (*description).invkind,
                (*description).cParams,
                (*description).elemdescFunc.tdesc.vt,
            )
        };
        // SAFETY: paired with `GetFuncDesc`.
        unsafe { info.ReleaseFuncDesc(description) };

        assert_eq!(memid, entry.dispid, "{} has the wrong dispid", entry.name);
        assert_eq!(
            invoke,
            match entry.flags {
                dispatch::GET => INVOKE_PROPERTYGET,
                dispatch::PUT => INVOKE_PROPERTYPUT,
                _ => INVOKE_FUNC,
            },
            "{} is described as the wrong kind of operation",
            entry.name
        );
        assert_eq!(
            parameters as u32, entry.arguments,
            "{} takes the wrong number of arguments",
            entry.name
        );

        // The return type. `VT_EMPTY` here is the mistake that cost an afternoon:
        // it is a variant holding nothing, which is a *value*, and a host told to
        // expect one from a call that hands it none unwraps a null.
        let expected = if entry.flags == dispatch::PUT {
            dispatch::VOID
        } else {
            entry.vt
        };
        assert_eq!(returns.0, expected, "{} returns the wrong type", entry.name);
        assert_ne!(
            returns, VT_EMPTY,
            "{} claims to return VT_EMPTY",
            entry.name
        );

        // The name, which is the whole point of the exercise. A put's value
        // parameter has none of its own — the property's name covers it, and
        // trying to give it one is refused outright.
        let mut names = [BSTR::default(), BSTR::default()];
        let mut written = 0u32;
        // SAFETY: `names` and `written` are live locals.
        unsafe { info.GetNames(memid, &mut names, &mut written) }.expect("names");
        assert!(written >= 1);
        assert_eq!(names[0].to_string(), entry.name);
    }

    let _ = std::fs::remove_file(path);
}

/// What PowerShell actually does: look a name up in the description rather than on
/// the object. A member the library cannot name is a member no such host will ever
/// invoke, however well `IDispatch::GetIDsOfNames` works.
#[test]
fn the_library_resolves_the_names_a_script_would_write() {
    let (library, path) = build_and_load("names");
    // SAFETY: `library` is live and the GUID outlives the call.
    let info = unsafe { library.GetTypeInfoOfGuid(&DIID_DENISE_PANEL) }.expect("the dispinterface");

    for member in dispatch::MEMBERS {
        // Lower case on purpose: a script writes `$panel.caption`, and neither
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
            .unwrap_or_else(|e| panic!("the library cannot name {}: {e}", member.name));
        assert_eq!(dispid, member.dispid, "{} resolved wrongly", member.name);
    }

    let _ = std::fs::remove_file(path);
}

/// The events, which is what `WithEvents` and `Register-ObjectEvent` hook up to.
#[test]
fn the_event_interface_carries_both_events_at_their_published_dispids() {
    let (library, path) = build_and_load("events");
    // SAFETY: `library` is live and the GUID outlives the call.
    let info = unsafe { library.GetTypeInfoOfGuid(&denise_activex::DIID_DENISE_PANEL_EVENTS) }
        .expect("the event interface");

    // SAFETY: released below.
    let attributes = unsafe { info.GetTypeAttr() }.expect("attributes");
    // SAFETY: readable until released.
    let (kind, functions) = unsafe { ((*attributes).typekind, (*attributes).cFuncs) };
    // SAFETY: paired.
    unsafe { info.ReleaseTypeAttr(attributes) };
    assert_eq!(kind, TKIND_DISPATCH);
    assert_eq!(functions as usize, dispatch::EVENTS.len());

    for (index, event) in dispatch::EVENTS.iter().enumerate() {
        // SAFETY: `index` is below `cFuncs`.
        let description = unsafe { info.GetFuncDesc(index as u32) }.expect("an event");
        // SAFETY: readable until released.
        let memid = unsafe { (*description).memid };
        // SAFETY: paired.
        unsafe { info.ReleaseFuncDesc(description) };
        assert_eq!(memid, event.dispid, "{} has the wrong dispid", event.name);
    }

    let _ = std::fs::remove_file(path);
}

/// The registry holds the library id as text and COM compares it as bytes. Two
/// spellings of one identity is exactly what silently disagrees — and the symptom
/// is a class that registers and a description no host can find.
#[test]
fn the_binary_and_text_library_ids_are_the_same_identity() {
    let text = format!("{{{:?}}}", LIBID_DENISE).to_uppercase();
    assert_eq!(text, denise_activex::registry::LIBID_TEXT.to_uppercase());
}

/// Keeps `GUID` named: every identity above is one, and they are the part that
/// cannot be changed once anything has stored them.
const _: fn() -> GUID = GUID::zeroed;
