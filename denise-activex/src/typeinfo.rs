//! Type information, built at run time from the same table `Invoke` reads.
//!
//! # Why this exists
//!
//! Answering `GetTypeInfoCount` with zero is honest — there is no type library —
//! and for VBScript, JScript and any container that calls `GetIDsOfNames` it
//! costs nothing, because those ask for a name and invoke it.
//!
//! **PowerShell is not one of those.** Its COM support is built on `ITypeInfo`:
//! it asks the object to describe itself and builds a member table from the
//! answer. An object that declines gets adapted as a bare `System.__ComObject`
//! with no members at all, and every property access fails with "cannot be found
//! on this object" — before any COM call is made. Nothing is wrong with the
//! control at that point; it has simply never been asked anything.
//!
//! `CreateDispTypeInfo` is the answer, and it is a much smaller one than a type
//! library: hand it a table of methods and it builds an `ITypeInfo` in memory.
//! No `.tlb` file, no `LIBID`, no extra registry keys, nothing to keep in step
//! with the DLL — and, because the table is generated from
//! [`crate::dispatch::entries`], nothing that can drift away from what `Invoke`
//! actually implements.
//!
//! What it still does not give anybody is a *registered* type library, which is
//! what a form designer's property sheet and an object browser read, and what
//! early binding needs. That remains outstanding.
//!
//! # The lifetimes
//!
//! `CreateDispTypeInfo` copies what it is given, so the buffers below only have
//! to survive the call. They are held in one owner anyway — the strings are
//! `PWSTR` into `Vec<u16>`s, and a description whose names had been freed would
//! be a use-after-free that reads as garbled member names rather than as a crash.

use windows::Win32::System::Com::{CC_STDCALL, ITypeInfo};
use windows::Win32::System::Ole::{CreateDispTypeInfo, INTERFACEDATA, METHODDATA, PARAMDATA};
use windows::Win32::System::Variant::VARENUM;
use windows_core::PWSTR;

use crate::dispatch::{self, PUT, VOID};

/// Builds an `ITypeInfo` describing the control's dispinterface.
pub fn describe() -> windows_core::Result<ITypeInfo> {
    let mut table = Table::new();
    // SAFETY: `table` owns every buffer `data` points into and outlives the call,
    // and `CreateDispTypeInfo` copies what it reads.
    let mut data = INTERFACEDATA {
        pmethdata: table.methods.as_mut_ptr(),
        cMembers: table.methods.len() as u32,
    };
    let mut out = None;
    // SAFETY: `data` describes `table`'s live arrays, and `out` receives the
    // interface. Locale zero is `LOCALE_NEUTRAL`: the names are ASCII and there is
    // nothing here to localise.
    unsafe { CreateDispTypeInfo(&mut data, 0, &mut out) }?;
    out.ok_or_else(|| windows_core::Error::from(windows::Win32::Foundation::E_FAIL))
}

/// The method table, and everything it points into.
///
/// One owner for the whole graph. `METHODDATA` holds raw pointers to the names
/// and the argument descriptions, so they have to outlive it — keeping them in
/// sibling fields of one value is what makes that true by construction rather
/// than by remembering.
struct Table {
    methods: Vec<METHODDATA>,
    /// Held so the `PWSTR`s in `methods` and `parameters` stay valid.
    #[allow(dead_code)]
    names: Vec<Vec<u16>>,
    /// Likewise for the `ppdata` pointers.
    #[allow(dead_code)]
    parameters: Vec<PARAMDATA>,
}

impl Table {
    fn new() -> Self {
        let entries = dispatch::entries();

        // Both collections are built to their final size *before* any pointer is
        // taken into either. Growing one afterwards would move what the pointers
        // refer to, and the symptom would be garbled member names rather than
        // anything that looks like a memory bug.
        //
        // Slot zero is the name of a put's single argument, shared by all of them:
        // it is read and never written, and every put takes the same thing.
        let mut names: Vec<Vec<u16>> = core::iter::once(wide("Value"))
            .chain(entries.iter().map(|entry| wide(entry.name)))
            .collect();
        let mut parameters: Vec<PARAMDATA> = Vec::new();

        let value_name = PWSTR(names[0].as_mut_ptr());
        parameters.extend(
            entries
                .iter()
                .filter(|entry| entry.flags == PUT)
                .map(|entry| PARAMDATA {
                    szName: value_name,
                    vt: VARENUM(entry.vt),
                }),
        );

        let mut methods = Vec::with_capacity(entries.len());
        let mut put = 0usize;

        for (index, entry) in entries.iter().enumerate() {
            let name = PWSTR(names[index + 1].as_mut_ptr());

            // A put takes the value it is assigning; nothing else takes anything.
            let arguments = if entry.flags == PUT {
                let pointer: *mut PARAMDATA = &mut parameters[put];
                put += 1;
                pointer
            } else {
                core::ptr::null_mut()
            };

            // A put returns nothing; a get returns the property's type; a method
            // returns whatever it was declared to, which for `Refresh` is also
            // nothing. `VOID` and not `VT_EMPTY` — see its comment; that mistake
            // is what a host unwraps into a null reference.
            let returns = if entry.flags == PUT { VOID } else { entry.vt };

            methods.push(METHODDATA {
                szName: name,
                ppdata: arguments,
                dispid: entry.dispid,
                // Sequential and dense, which is what a dispinterface's method
                // table means by an index.
                iMeth: index as u32,
                cc: CC_STDCALL,
                cArgs: entry.arguments,
                wFlags: entry.flags,
                vtReturn: VARENUM(returns),
            });
        }

        Self {
            methods,
            names,
            parameters,
        }
    }
}

/// A NUL-terminated UTF-16 buffer, which is what a `PWSTR` has to point at.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}
