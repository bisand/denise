//! The COM/ActiveX shim, so legacy Windows hosts can embed the Denise control.
//!
//! VB6, MFC, Delphi and WinForms all reach a control the same way: a class id in
//! the registry, a DLL that answers `DllGetClassObject`, and an object
//! implementing the OLE control interfaces. [`denise_win32`] already provides the
//! window such an object would host; this crate is the wrapper around it.
//!
//! # Status
//!
//! The server is here: the four `Dll*` exports, a class factory, and a control
//! implementing `IOleObject`, `IOleInPlaceObject`, `IOleWindow`, `IOleControl`
//! and `IPersistStreamInit`. A container can instantiate it, site it, activate it
//! in place — at which point it creates a real [`denise_win32`] child window — and
//! tear it down again.
//!
//! `registry` is the half that can be tested without Windows, and it is also the
//! half that most often goes wrong: a control fails to appear in a host's toolbox
//! for one of about four reasons, all of them a missing or wrong registry value,
//! and none of them producing an error anywhere. So that list is data, and the
//! tests check it as data.
//!
//! # What is not here yet
//!
//! **`IDispatch`.** Without it a container can host the control and cannot script
//! it: no properties, no methods, no events. The tree behind the control emits
//! messages already; delivering them to the host is what this would add.
//!
//! **`IViewObject2::Draw`.** The design-time view a form editor asks for before
//! the control is ever activated. Without it a control dropped on a form is a
//! blank rectangle until the form runs.
//!
//! **Any evidence it works.** This has never been loaded by a container. The
//! Win32 control underneath it now has been — that was the gate — but COM has
//! failure modes a compiler cannot see, and every one of them will be found by a
//! host rather than by CI.
//!
//! # Registering it
//!
//! ```text
//! regsvr32 denise_activex.dll
//! regsvr32 /u denise_activex.dll
//! ```
//!
//! The class id is generated once and never changes: a host stores it in a form
//! file, so a new one on every build would break every project that ever embedded
//! the control.

pub mod registry;

#[cfg(windows)]
mod control;
#[cfg(windows)]
mod factory;
#[cfg(windows)]
mod server;

#[cfg(windows)]
pub use control::DenisePanel;
#[cfg(windows)]
pub use server::{
    CLSID_DENISE_PANEL, DllCanUnloadNow, DllGetClassObject, DllRegisterServer, DllUnregisterServer,
};

pub use registry::{
    CLSID_TEXT, FRIENDLY_NAME, MISC_STATUS, PROG_ID, VERSION, VERSION_INDEPENDENT_PROG_ID,
};
