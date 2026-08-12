//! The class factory: what COM asks for before it asks for a control.
//!
//! A container never constructs the object itself. It asks the DLL for the class
//! object, asks that for an instance, and only then talks to the control. This is
//! the middle step, and it is almost entirely boilerplate — the one decision in
//! it is refusing aggregation.

use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::IClassFactory_Impl;
use windows_core::{BOOL, IUnknown, Interface, Ref, implement};

use crate::control::DenisePanel;
use crate::server::{lock_server, unlock_server};

/// Hands out [`DenisePanel`] instances.
///
/// Stateless, so one value serves every request and there is nothing to keep
/// alive between them.
#[implement(windows::Win32::System::Com::IClassFactory)]
pub struct PanelFactory;

impl IClassFactory_Impl for PanelFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        riid: *const windows_core::GUID,
        object: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()> {
        if object.is_null() || riid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: COM promises a writable out-pointer.
        unsafe { object.write(core::ptr::null_mut()) };

        // Aggregation would mean this control's IUnknown deferring to an outer
        // object's. Nothing here is written for that, and saying so is better
        // than half-supporting it: a container that wanted it gets a clear
        // refusal rather than a control whose reference counting is subtly wrong.
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }

        let panel: IUnknown = DenisePanel::new().into();
        // SAFETY: `riid` is readable and `object` writable, as promised above.
        unsafe { panel.query(riid, object).ok() }
    }

    fn LockServer(&self, lock: BOOL) -> windows_core::Result<()> {
        // A container holds the server loaded across creating and destroying
        // instances. Ignoring this is how a DLL gets unloaded between two calls
        // that were meant to be one session.
        if lock.as_bool() {
            lock_server();
        } else {
            unlock_server();
        }
        Ok(())
    }
}
