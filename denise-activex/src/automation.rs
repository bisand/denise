//! `IDispatch` and the connection point: the half a script talks to.
//!
//! [`crate::dispatch`] decides what a name means and what a flag combination
//! asks for; this is the pointer handling either side of those decisions, and the
//! plumbing that carries an event back out to the host.
//!
//! # One object, both ends of the connection
//!
//! `IConnectionPointContainer` and `IConnectionPoint` are both implemented on the
//! control itself rather than on a separate connection-point object. There is
//! exactly one outgoing interface, so a second object would exist only to hold a
//! back-pointer to this one — and that back-pointer is the reference cycle every
//! connection-point bug is made of. A container reaches the connection point
//! through `FindConnectionPoint`, which is where the identity is decided, so it
//! cannot tell the difference.

use windows::Win32::Foundation::{
    DISP_E_BADPARAMCOUNT, DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNINTERFACE, DISP_E_UNKNOWNNAME,
    E_POINTER, TYPE_E_ELEMENTNOTFOUND,
};
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO, IConnectionPoint, IConnectionPoint_Impl,
    IConnectionPointContainer, IConnectionPointContainer_Impl, IDispatch, IDispatch_Impl,
    IEnumConnectionPoints, IEnumConnections, ITypeInfo,
};
use windows::Win32::System::Ole::{CONNECT_E_CANNOTCONNECT, CONNECT_E_NOCONNECTION};
use windows::Win32::System::Variant::VARIANT;
use windows_core::{GUID, IUnknown, IUnknownImpl, Interface, PCWSTR, Ref};

use crate::control::DenisePanel_Impl;
use crate::dispatch::{self, Action};
use crate::typeinfo;
use crate::variant;

/// The outgoing dispinterface a container sinks events on.
///
/// Generated once and never changed, for the same reason as the class id: a host
/// that stores it stores the number. Pass it to `FindConnectionPoint`.
pub const DIID_DENISE_PANEL_EVENTS: GUID =
    GUID::from_u128(0x5405_253D_6E92_42BA_916C_F483_4D09_9F69);

// ------------------------------------------------------------------- IDispatch

impl IDispatch_Impl for DenisePanel_Impl {
    fn GetTypeInfoCount(&self) -> windows_core::Result<u32> {
        // One. This started as zero — honest, since there is no type library, and
        // free for VBScript and every container that asks for a name and invokes
        // it. PowerShell does not: its COM support builds a member table from
        // `ITypeInfo`, and an object that declines is adapted as a bare
        // `System.__ComObject` with no members, so every property fails before a
        // single COM call is made. See [`crate::typeinfo`].
        Ok(1)
    }

    fn GetTypeInfo(&self, index: u32, _lcid: u32) -> windows_core::Result<ITypeInfo> {
        // One dispinterface, so index zero and nothing else.
        if index != 0 {
            return Err(TYPE_E_ELEMENTNOTFOUND.into());
        }
        if let Some(info) = self.state.borrow().type_info.clone() {
            return Ok(info);
        }
        // Built once per control and kept: it describes a table that cannot change
        // while the process runs.
        let info = typeinfo::describe()?;
        self.state.borrow_mut().type_info = Some(info.clone());
        Ok(info)
    }

    fn GetIDsOfNames(
        &self,
        riid: *const GUID,
        names: *const PCWSTR,
        count: u32,
        _lcid: u32,
        out: *mut i32,
    ) -> windows_core::Result<()> {
        if names.is_null() || out.is_null() {
            return Err(E_POINTER.into());
        }
        reserved_iid(riid)?;

        let count = count as usize;
        // SAFETY: the host promises `count` readable strings and `count` writable
        // dispids.
        let (names, out) = unsafe {
            (
                core::slice::from_raw_parts(names, count),
                core::slice::from_raw_parts_mut(out, count),
            )
        };
        // Owned first, borrowed second: the resolver takes `&str`, and a `PCWSTR`
        // has to be copied out of the host's memory to become one.
        let owned: Vec<String> = names
            .iter()
            // SAFETY: each entry is a NUL-terminated string the host owns for the
            // call. An unreadable one is a host bug; an empty name simply fails to
            // resolve, which is the answer it deserves.
            .map(|name| unsafe { name.to_string() }.unwrap_or_default())
            .collect();
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();

        if dispatch::resolve(&borrowed, out) {
            Ok(())
        } else {
            Err(DISP_E_UNKNOWNNAME.into())
        }
    }

    fn Invoke(
        &self,
        dispid: i32,
        riid: *const GUID,
        _lcid: u32,
        flags: DISPATCH_FLAGS,
        params: *const DISPPARAMS,
        result: *mut VARIANT,
        _exception: *mut EXCEPINFO,
        _argument_error: *mut u32,
    ) -> windows_core::Result<()> {
        reserved_iid(riid)?;

        let member = dispatch::member_by_id(dispid).ok_or(DISP_E_MEMBERNOTFOUND)?;
        // A put to a method, or a call to a property: the script is wrong, and
        // the only useful answer is one the host can turn into an error message.
        let action = dispatch::action(member, flags.0).ok_or(DISP_E_MEMBERNOTFOUND)?;

        match action {
            Action::Get => {
                match member.dispid {
                    dispatch::DISPID_TEXT => {
                        let text = self.model.borrow().text.clone();
                        // SAFETY: `result` is the host's out-variant, null when it
                        // does not want the answer, which `write_string` handles.
                        unsafe { variant::write_string(result, &text) };
                    }
                    dispatch::DISPID_CAPTION => {
                        let caption = self.model.borrow().caption.clone();
                        // SAFETY: as above.
                        unsafe { variant::write_string(result, &caption) };
                    }
                    dispatch::DISPID_ENABLED => {
                        let enabled = self.model.borrow().enabled;
                        // SAFETY: as above.
                        unsafe { variant::write_bool(result, enabled) };
                    }
                    _ => return Err(DISP_E_MEMBERNOTFOUND.into()),
                }
                Ok(())
            }

            Action::Put => {
                // SAFETY: the host promises a readable `DISPPARAMS` for the call.
                let argument =
                    unsafe { variant::sole_argument(params) }.ok_or(DISP_E_BADPARAMCOUNT)?;
                match member.dispid {
                    dispatch::DISPID_TEXT => {
                        // SAFETY: the argument is the host's, live for the call.
                        let text = unsafe { variant::to_string(argument) }?;
                        self.model.borrow_mut().text = text;
                    }
                    dispatch::DISPID_CAPTION => {
                        // SAFETY: as above.
                        let caption = unsafe { variant::to_string(argument) }?;
                        self.model.borrow_mut().caption = caption;
                    }
                    dispatch::DISPID_ENABLED => {
                        // SAFETY: as above.
                        let enabled = unsafe { variant::to_bool(argument) }?;
                        self.model.borrow_mut().enabled = enabled;
                    }
                    _ => return Err(DISP_E_MEMBERNOTFOUND.into()),
                }
                self.model.borrow_mut().touch();
                self.sync();
                Ok(())
            }

            Action::Call => match member.dispid {
                dispatch::DISPID_REFRESH => {
                    {
                        let mut model = self.model.borrow_mut();
                        model.refresh = true;
                        model.touch();
                    }
                    self.sync();
                    Ok(())
                }
                _ => Err(DISP_E_MEMBERNOTFOUND.into()),
            },
        }
    }
}

/// `riid` is reserved on both dispatch entry points and must be `IID_NULL`.
///
/// Checked rather than ignored because a host that passes something else is
/// asking for a *different* dispatch interface, and answering with this one's
/// members would be a wrong answer rather than a refusal.
fn reserved_iid(riid: *const GUID) -> windows_core::Result<()> {
    if riid.is_null() {
        return Ok(());
    }
    // SAFETY: the host promises a readable GUID when it passes one.
    if unsafe { *riid } == GUID::zeroed() {
        Ok(())
    } else {
        Err(DISP_E_UNKNOWNINTERFACE.into())
    }
}

// -------------------------------------------------------- IConnectionPoint(er)

impl IConnectionPointContainer_Impl for DenisePanel_Impl {
    fn EnumConnectionPoints(&self) -> windows_core::Result<IEnumConnectionPoints> {
        // A container that wants to discover the outgoing interfaces of an object
        // with no type library has nothing to discover them *with*. Every host
        // that sinks these events knows the IID and calls `FindConnectionPoint`.
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }

    fn FindConnectionPoint(&self, riid: *const GUID) -> windows_core::Result<IConnectionPoint> {
        if riid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the container promises a readable GUID.
        if unsafe { *riid } != DIID_DENISE_PANEL_EVENTS {
            return Err(CONNECT_E_NOCONNECTION.into());
        }
        Ok(self.to_interface())
    }
}

impl IConnectionPoint_Impl for DenisePanel_Impl {
    fn GetConnectionInterface(&self) -> windows_core::Result<GUID> {
        Ok(DIID_DENISE_PANEL_EVENTS)
    }

    fn GetConnectionPointContainer(&self) -> windows_core::Result<IConnectionPointContainer> {
        Ok(self.to_interface())
    }

    fn Advise(&self, sink: Ref<'_, IUnknown>) -> windows_core::Result<u32> {
        let sink = sink.cloned().ok_or(E_POINTER)?;
        // A dispinterface *is* `IDispatch`, so this is the whole requirement on a
        // sink: no vtable to match, nothing to compile against, just `Invoke` with
        // one of the two event dispids.
        let sink: IDispatch = sink.cast().map_err(|_| CONNECT_E_CANNOTCONNECT)?;
        Ok(self.model.borrow_mut().advise(sink))
    }

    fn Unadvise(&self, cookie: u32) -> windows_core::Result<()> {
        if self.model.borrow_mut().unadvise(cookie) {
            Ok(())
        } else {
            // Being told about a cookie that was never connected is how a
            // container finds out it double-unadvised, which is worth knowing.
            Err(CONNECT_E_NOCONNECTION.into())
        }
    }

    fn EnumConnections(&self) -> windows_core::Result<IEnumConnections> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
}
