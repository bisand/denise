//! The type library: a `.tlb` built at registration from the dispatch table.
//!
//! # Why this exists
//!
//! Answering `GetTypeInfoCount` with zero is honest and costs nothing for the
//! hosts that bind names late — VBScript, JScript, VB6 through an `Object`
//! variable, MFC's `COleDispatchDriver`, every OLE container. It costs PowerShell
//! entirely: PowerShell builds its member table from type information and will not
//! ask for a name it has not been told about, so a control with none is adapted as
//! a bare `System.__ComObject` and `$panel.Caption` fails before a single COM call
//! is made.
//!
//! `CreateDispTypeInfo` was tried first, as a way to answer without a library.
//! It cannot: the description it builds is vtable-shaped — `TKIND_INTERFACE`, not
//! `TKIND_DISPATCH` — and PowerShell looks for a dispinterface, does not find one,
//! and produces an object with no members and no complaint. Nothing in the method
//! table changes the kind. So this is the real thing.
//!
//! # Built rather than shipped
//!
//! There is no `.idl` and no MIDL step. `DllRegisterServer` calls
//! [`build`] to write `denise_activex.tlb` beside the DLL, from
//! [`crate::dispatch::entries`] — the same table `Invoke` reads. A library
//! compiled from separate source could disagree with the implementation; one
//! generated from the implementation's own table cannot.
//!
//! The cost is a second file to deploy. `RegisterTypeLib` records its full path,
//! so it has to stay next to the DLL.

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Com::{
    CC_STDCALL, ELEMDESC, ELEMDESC_0, FUNC_DISPATCH, FUNCDESC, FUNCFLAGS, IDLDESC,
    IMPLTYPEFLAG_FDEFAULT, IMPLTYPEFLAG_FSOURCE, INVOKE_FUNC, INVOKE_PROPERTYGET,
    INVOKE_PROPERTYPUT, ITypeInfo, ITypeLib, SYSKIND, TKIND_COCLASS, TKIND_DISPATCH, TYPEDESC,
    TYPEDESC_0,
};
use windows::Win32::System::Ole::{
    CreateTypeLib2, ICreateTypeInfo, LoadRegTypeLib, LoadTypeLibEx, PARAMDESC, REGKIND_NONE,
    REGKIND_REGISTER, TYPEFLAG_FCANCREATE, TYPEFLAG_FDISPATCHABLE, UnRegisterTypeLib,
};
use windows::Win32::System::Variant::VARENUM;
use windows_core::{BSTR, GUID, Interface, PCWSTR};

use crate::dispatch::{self, PUT};
use crate::server::CLSID_DENISE_PANEL;

/// The type library's own id.
///
/// Generated once and never changed: it is written into the registry, and a
/// compiled early-bound host stores it.
pub const LIBID_DENISE: GUID = GUID::from_u128(0x5CA2_EE57_C922_483E_8FDA_B0A8_B3D3_B195);

/// The incoming dispinterface — what a script reaches when it names a property.
pub const DIID_DENISE_PANEL: GUID = GUID::from_u128(0x4C51_48FF_09F3_4C34_9B77_00C8_50E1_F940);

/// The library's version. Bumping this makes a new registry key, so it is part of
/// the contract rather than a build number.
pub const VERSION: (u16, u16) = (1, 0);

/// Which registry key `RegisterTypeLib` writes the path under.
///
/// A decision made by the compiler, not by the machine: a 32-bit build registers
/// under `win32` and a 64-bit one under `win64`, and Windows on ARM64 is 64-bit.
/// A host loads the one matching its own word size, so a mismatch here is a
/// library that registers and never loads.
const fn syskind() -> SYSKIND {
    #[cfg(target_pointer_width = "64")]
    {
        windows::Win32::System::Com::SYS_WIN64
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        windows::Win32::System::Com::SYS_WIN32
    }
}

/// The library file that belongs beside `dll`.
///
/// `…\denise_activex.dll` becomes `…\denise_activex.tlb`.
pub fn path_beside(dll: &str) -> String {
    match dll.rfind('.') {
        Some(dot) => format!("{}.tlb", &dll[..dot]),
        None => format!("{dll}.tlb"),
    }
}

/// Writes the library to `path`.
pub fn build(path: &str) -> windows_core::Result<()> {
    let wide = wide(path);
    // SAFETY: `wide` is NUL-terminated and live for the call.
    let library = unsafe { CreateTypeLib2(syskind(), PCWSTR(wide.as_ptr())) }?;

    // SAFETY: every one of these takes values live for its own call.
    unsafe {
        library.SetGuid(&LIBID_DENISE)?;
        library.SetName(&BSTR::from("Denise"))?;
        library.SetVersion(VERSION.0, VERSION.1)?;
        library.SetDocString(&BSTR::from("Denise panel control"))?;
        // Locale-neutral. The names are ASCII and there is nothing to localise;
        // claiming a locale would make a host in another one look elsewhere.
        library.SetLcid(0)?;
    }

    // SAFETY: `library` is live; each call builds one type into it.
    let panel = unsafe { library.CreateTypeInfo(&BSTR::from("DDenisePanel"), TKIND_DISPATCH) }?;
    describe_panel(&panel)?;

    let events =
        // SAFETY: as above.
        unsafe { library.CreateTypeInfo(&BSTR::from("DDenisePanelEvents"), TKIND_DISPATCH) }?;
    describe_events(&events)?;

    // SAFETY: as above.
    let coclass = unsafe { library.CreateTypeInfo(&BSTR::from("Panel"), TKIND_COCLASS) }?;
    describe_coclass(&coclass, &panel, &events)?;

    // SAFETY: writes the file. Everything above is held until this returns.
    unsafe { library.SaveAllChanges() }
}

/// The members a script can reach.
fn describe_panel(info: &ICreateTypeInfo) -> windows_core::Result<()> {
    // SAFETY: a live builder and a GUID that outlives the call.
    unsafe {
        info.SetGuid(&DIID_DENISE_PANEL)?;
        info.SetVersion(VERSION.0, VERSION.1)?;
        // `FDISPATCHABLE` is what makes this reachable through `IDispatch` rather
        // than through a vtable a dispinterface does not have.
        info.SetTypeFlags(TYPEFLAG_FDISPATCHABLE.0 as u32)?;
    }

    for (index, entry) in dispatch::entries().iter().enumerate() {
        // A put takes the value being assigned; a get and a method take nothing.
        // Held in a local that outlives `AddFuncDesc`, which copies what it reads.
        let mut argument = [ELEMDESC {
            tdesc: TYPEDESC {
                Anonymous: TYPEDESC_0 {
                    lptdesc: core::ptr::null_mut(),
                },
                vt: VARENUM(entry.vt),
            },
            Anonymous: ELEMDESC_0 {
                paramdesc: PARAMDESC::default(),
            },
        }];

        // A put returns nothing; everything else returns what it was declared to.
        let returns = if entry.flags == PUT {
            dispatch::VOID
        } else {
            entry.vt
        };

        let description = FUNCDESC {
            memid: entry.dispid,
            lprgscode: core::ptr::null_mut(),
            lprgelemdescParam: if entry.arguments == 0 {
                core::ptr::null_mut()
            } else {
                argument.as_mut_ptr()
            },
            // `FUNC_DISPATCH`, because these are reached by dispid rather than by
            // slot: a dispinterface has no vtable to be at an offset into.
            funckind: FUNC_DISPATCH,
            invkind: match entry.flags {
                dispatch::GET => INVOKE_PROPERTYGET,
                dispatch::PUT => INVOKE_PROPERTYPUT,
                _ => INVOKE_FUNC,
            },
            callconv: CC_STDCALL,
            cParams: entry.arguments as i16,
            cParamsOpt: 0,
            oVft: 0,
            cScodes: 0,
            elemdescFunc: ELEMDESC {
                tdesc: TYPEDESC {
                    Anonymous: TYPEDESC_0 {
                        lptdesc: core::ptr::null_mut(),
                    },
                    vt: VARENUM(returns),
                },
                Anonymous: ELEMDESC_0 {
                    idldesc: IDLDESC::default(),
                },
            },
            wFuncFlags: FUNCFLAGS(0),
        };

        // SAFETY: `description` and `argument` are live locals, and `AddFuncDesc`
        // copies what it is given rather than retaining the pointers — which is
        // the promise that a previous attempt at this got wrong, freeing the
        // buffers a description still pointed at and crashing the machine reading
        // it back.
        unsafe { info.AddFuncDesc(index as u32, &description) }?;

        // The name, and one name per parameter after it. A put's argument is
        // called `Value`, which is the convention every automation host expects.
        let name = wide(entry.name);
        let value = wide("Value");
        let mut names = vec![PCWSTR(name.as_ptr())];
        if entry.arguments > 0 {
            names.push(PCWSTR(value.as_ptr()));
        }
        // SAFETY: both buffers outlive the call, and `names` describes them.
        unsafe { info.SetFuncAndParamNames(index as u32, &names) }?;
    }
    Ok(())
}

/// The events the control raises.
fn describe_events(info: &ICreateTypeInfo) -> windows_core::Result<()> {
    // SAFETY: a live builder and a GUID that outlives the call.
    unsafe {
        info.SetGuid(&crate::automation::DIID_DENISE_PANEL_EVENTS)?;
        info.SetVersion(VERSION.0, VERSION.1)?;
        info.SetTypeFlags(TYPEFLAG_FDISPATCHABLE.0 as u32)?;
    }

    for (index, event) in dispatch::EVENTS.iter().enumerate() {
        let description = FUNCDESC {
            memid: event.dispid,
            lprgscode: core::ptr::null_mut(),
            lprgelemdescParam: core::ptr::null_mut(),
            funckind: FUNC_DISPATCH,
            invkind: INVOKE_FUNC,
            callconv: CC_STDCALL,
            cParams: 0,
            cParamsOpt: 0,
            oVft: 0,
            cScodes: 0,
            elemdescFunc: ELEMDESC {
                tdesc: TYPEDESC {
                    Anonymous: TYPEDESC_0 {
                        lptdesc: core::ptr::null_mut(),
                    },
                    vt: VARENUM(dispatch::VOID),
                },
                Anonymous: ELEMDESC_0 {
                    idldesc: IDLDESC::default(),
                },
            },
            wFuncFlags: FUNCFLAGS(0),
        };
        // SAFETY: `description` is a live local and is copied by the call.
        unsafe { info.AddFuncDesc(index as u32, &description) }?;

        let name = wide(event.name);
        // SAFETY: `name` outlives the call.
        unsafe { info.SetFuncAndParamNames(index as u32, &[PCWSTR(name.as_ptr())]) }?;
    }
    Ok(())
}

/// The class, and which of the two interfaces is which.
///
/// This is the part a host reads to answer "what happens when I create a
/// `Denise.Panel`": the default interface is what it binds to, and the default
/// *source* is what `WithEvents` in VB6 or `Register-ObjectEvent` in PowerShell
/// hooks up to.
fn describe_coclass(
    info: &ICreateTypeInfo,
    panel: &ICreateTypeInfo,
    events: &ICreateTypeInfo,
) -> windows_core::Result<()> {
    // SAFETY: a live builder; `CLSID_DENISE_PANEL` outlives the call.
    unsafe {
        info.SetGuid(&CLSID_DENISE_PANEL)?;
        info.SetVersion(VERSION.0, VERSION.1)?;
        info.SetTypeFlags(TYPEFLAG_FCANCREATE.0 as u32)?;
    }

    for (index, (part, flags)) in [
        (panel, IMPLTYPEFLAG_FDEFAULT),
        (events, IMPLTYPEFLAG_FDEFAULT | IMPLTYPEFLAG_FSOURCE),
    ]
    .into_iter()
    .enumerate()
    {
        let part: ITypeInfo = part.cast()?;
        let mut href = 0u32;
        // SAFETY: `part` is live, and `href` receives the reference. The binding
        // declares the out-parameter `*const`, which is a quirk of the generated
        // signature rather than of the API — `AddRefTypeInfo` writes through it.
        unsafe {
            info.AddRefTypeInfo(&part, &mut href as *mut u32 as *const u32)?;
            info.AddImplType(index as u32, href)?;
            info.SetImplTypeFlags(index as u32, flags)?;
        }
    }
    Ok(())
}

/// Registers the library at `path` with the system.
pub fn register(path: &str) -> windows_core::Result<()> {
    let wide = wide(path);
    // SAFETY: `wide` is NUL-terminated and live. `REGKIND_REGISTER` both loads it
    // and writes the `TypeLib` keys, including the platform subkey that says which
    // word size this file is for.
    unsafe { LoadTypeLibEx(PCWSTR(wide.as_ptr()), REGKIND_REGISTER) }?;
    Ok(())
}

/// Removes it again.
///
/// The file itself is left where it is. It was written next to the DLL, it is
/// rewritten by the next registration, and deleting files on the way out is a
/// bigger promise than a server should make.
pub fn unregister() -> windows_core::Result<()> {
    // SAFETY: the library id and version are constants; the locale matches what
    // `build` set.
    unsafe { UnRegisterTypeLib(&LIBID_DENISE, VERSION.0, VERSION.1, 0, syskind()) }
}

/// The dispinterface a host should be handed, from the registered library.
///
/// Falls back to the file beside the DLL, so a control that was created without
/// `regsvr32` — from a test, or by a host that registered it per-user — still
/// describes itself.
pub fn panel_type_info(fallback: Option<&str>) -> windows_core::Result<ITypeInfo> {
    // SAFETY: constants, and an out-parameter the binding owns.
    if let Ok(library) = unsafe { LoadRegTypeLib(&LIBID_DENISE, VERSION.0, VERSION.1, 0) } {
        // SAFETY: `library` is live and the GUID outlives the call.
        if let Ok(info) = unsafe { library.GetTypeInfoOfGuid(&DIID_DENISE_PANEL) } {
            return Ok(info);
        }
    }

    let path = fallback.ok_or(E_FAIL)?;
    let wide = wide(path);
    // SAFETY: `wide` is live. `REGKIND_NONE` loads without touching the registry,
    // which is what makes this usable from a test that must not need privileges.
    let library: ITypeLib = unsafe { LoadTypeLibEx(PCWSTR(wide.as_ptr()), REGKIND_NONE) }?;
    // SAFETY: `library` is live and the GUID outlives the call.
    unsafe { library.GetTypeInfoOfGuid(&DIID_DENISE_PANEL) }
}

/// A NUL-terminated UTF-16 buffer, which is what every `W` entry point wants.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}
