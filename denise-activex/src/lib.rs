//! The COM/ActiveX shim, so legacy Windows hosts can embed the Denise control.
//!
//! VB6, MFC, Delphi and WinForms all reach a control the same way: a class id in
//! the registry, a DLL that answers `DllGetClassObject`, and an object
//! implementing the OLE control interfaces. [`denise_win32`] already provides the
//! window such an object would host; this crate is the wrapper around it.
//!
//! # Status: the registration table only
//!
//! What is here is the half that can be tested without Windows, and it is also
//! the half that most often goes wrong: `registry`, the complete list of values
//! `DllRegisterServer` writes and `DllUnregisterServer` removes. A control fails
//! to appear in a host's toolbox for one of about four reasons, all of them a
//! missing or wrong registry value, and none of them producing an error anywhere.
//! So that list is data, and the tests check it as data — on any machine,
//! including one with no registry at all.
//!
//! What is **not** here is the COM object itself: `IClassFactory`, `IOleObject`,
//! `IOleInPlaceObject`, `IOleControl`, `IViewObject2`, `IPersistStreamInit`,
//! `IDispatch`, and the four `Dll*` entry points.
//!
//! That is a deliberate stop rather than an oversight. It would sit entirely on
//! top of [`denise_win32`], which has itself never run on Windows, and none of it
//! can be checked from here by anything stronger than "it compiles" — and
//! "compiles" is a very long way from "a VB6 form can host it". Stacking a
//! thousand lines of unverifiable COM on an unverified control produces something
//! that looks finished and is not. The right order is: run the Win32 control on
//! Windows first, then write this against a host that can load it.
//!
//! The constants below are settled and safe to depend on. In particular the class
//! id is generated once and never changes: a host stores it in a form file, so a
//! new one on every build would break every project that ever embedded the
//! control.

pub mod registry;

pub use registry::{
    CLSID_TEXT, FRIENDLY_NAME, MISC_STATUS, PROG_ID, VERSION, VERSION_INDEPENDENT_PROG_ID,
};
