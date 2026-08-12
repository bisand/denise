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
//! implementing `IOleObject`, `IOleInPlaceObject`, `IOleWindow`, `IOleControl`,
//! `IPersistStreamInit`, `IDispatch` and the connection point that carries its
//! events. A container can instantiate it, site it, activate it in place — at
//! which point it creates a real [`denise_win32`] child window — script it by
//! name, sink its events, and tear it down again.
//!
//! `registry`, `himetric` and `dispatch` are the halves that can be tested
//! without Windows, and they are also the halves that most often go wrong: a
//! control fails to appear in a host's toolbox for one of about four reasons, all
//! of them a missing or wrong registry value, and none of them producing an error
//! anywhere. So those lists are data, and the tests check them as data.
//!
//! # Scripting it
//!
//! There is no type library, so a host is late-bound: it asks for a name and
//! invokes it. VBScript, JScript, VB6 through an `Object` variable and MFC's
//! `COleDispatchDriver` all work that way.
//!
//! PowerShell does not. It builds its member table from `ITypeInfo` and will not
//! ask for a name it has not been told about — an object that answers
//! `GetTypeInfoCount` with zero is adapted as a bare `System.__ComObject` with no
//! members, and every property fails before a single COM call is made. So the
//! control describes itself on demand with `CreateDispTypeInfo`, built from the
//! same table `Invoke` reads. See [`dispatch`] and the `typeinfo` module.
//!
//! Either way the surface is short: without a type library each member is
//! something a person has to be told about rather than discover, so each one has
//! to earn its place.
//!
//! | Member | Dispid | |
//! |---|---|---|
//! | `Text` | 1 | property, read/write — the field's contents |
//! | `Caption` | 2 | property, read/write — the heading |
//! | `Enabled` | 3 | property, read/write — whether the field and button take input |
//! | `Refresh` | 4 | method — repaint everything |
//! | `Change` | 1 | event — somebody typed in the field |
//! | `Click` | -600 | event — the button was pressed (`DISPID_CLICK`) |
//!
//! ```text
//! $panel = New-Object -ComObject Denise.Panel
//! $panel.Caption = "Hei"
//! $panel.Caption
//! ```
//!
//! Events arrive through a connection point. Ask for
//! [`DIID_DENISE_PANEL_EVENTS`] with `IConnectionPointContainer::FindConnectionPoint`
//! and advise an object implementing `IDispatch`; there is no vtable to match and
//! nothing to compile against, only `Invoke` with one of the two dispids above.
//!
//! # What is not here yet
//!
//! **A registered type library.** `GetTypeInfo` answers, but there is no `.tlb`
//! and no `LIBID` in the registry, which is what a form designer's property sheet
//! and an object browser read, and what early binding needs.
//!
//! **`IViewObject2::Draw`.** The design-time view a form editor asks for before
//! the control is ever activated. Without it a control dropped on a form is a
//! blank rectangle until the form runs.
//!
//! **The scripting safety categories.** `IObjectSafety` and the "safe for
//! scripting" registry entries are deliberately absent: they are an assertion
//! about untrusted callers that has not been thought through, and claiming it
//! carelessly is worse than not claiming it.
//!
//! # What has been verified
//!
//! On Windows 11 ARM64: registered with `regsvr32`, instantiated through
//! `CoCreateInstance`, sited, activated in place, and rendering — with text typed
//! into it, including AltGr and dead keys. Then scripted: `Caption`, `Text` and
//! `Enabled` set and read **by name** through `GetIDsOfNames` and `Invoke`, a sink
//! advised on the connection point, and `Change` and `Click` arriving at it.
//!
//! The part worth naming is the re-entrancy. The click handler assigns to
//! `Caption` and the change handler reads `Text`, both from inside the event the
//! control raised — a few hundred round trips over eighteen clicks and every
//! keystroke of a sentence, with the control's `RefCell` borrowed around all of
//! them.
//!
//! `examples/host.rs` is the container that did it, and it goes through the
//! registry rather than around it, so what it proves is the *registered* server.
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

pub mod connections;
pub mod dispatch;
pub mod himetric;
pub mod registry;

#[cfg(windows)]
mod automation;
#[cfg(windows)]
mod control;
#[cfg(windows)]
mod factory;
#[cfg(windows)]
mod model;
#[cfg(windows)]
mod server;
#[cfg(windows)]
mod typeinfo;
#[cfg(windows)]
mod variant;

#[cfg(windows)]
pub use automation::DIID_DENISE_PANEL_EVENTS;
#[cfg(windows)]
pub use control::DenisePanel;
#[cfg(windows)]
pub use server::{
    CLSID_DENISE_PANEL, DllCanUnloadNow, DllGetClassObject, DllRegisterServer, DllUnregisterServer,
};

pub use registry::{
    CLSID_TEXT, FRIENDLY_NAME, MISC_STATUS, PROG_ID, VERSION, VERSION_INDEPENDENT_PROG_ID,
};
