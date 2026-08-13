# denise-activex

[![crates.io](https://img.shields.io/crates/v/denise-activex?color=CBA6F7&label=crates.io)](https://crates.io/crates/denise-activex)
[![docs.rs](https://img.shields.io/docsrs/denise-activex?color=94E2D5&label=docs.rs)](https://docs.rs/denise-activex)
[![Licence](https://img.shields.io/badge/licence-MIT-89B4FA)](https://github.com/bisand/denise/blob/main/LICENSE)

The COM/ActiveX shim for **[Denise]**, so legacy Windows hosts can embed the
control.

VB6, MFC, Delphi and WinForms all reach a control the same way: a class id in the
registry, a DLL that answers `DllGetClassObject`, and an object implementing the OLE
control interfaces. [`denise-win32`](https://crates.io/crates/denise-win32) already
provides the window such an object hosts; this crate is the wrapper around it.

```text
regsvr32 denise_activex.dll
```

```powershell
$panel = New-Object -ComObject Denise.Panel
$panel.Caption = "Hei"
$panel.Text
```

## What is implemented

The four `Dll*` exports, a class factory, and a control implementing `IOleObject`,
`IOleInPlaceObject`, `IOleWindow`, `IOleControl`, `IPersistStreamInit`, `IDispatch`,
`IViewObject2`, `IObjectSafety` and the connection point that carries its events.

A container can instantiate it, site it, activate it in place — at which point it
creates a real `denise-win32` child window — script it by name, sink its events, and
tear it down again.

It can also be asked to draw **without any of that**, which is what a form editor
does: `IViewObject2::Draw` renders the tree from the current property values into
whatever device context the container passes, with no site and no window. Without it
a control dropped on a form is a blank rectangle until the form runs.

## The scriptable surface

Deliberately short. A type library makes each member *discoverable*, which is a
reason to have fewer good ones rather than a licence to add more.

| Member | Dispid | |
|---|---|---|
| `Text` | 1 | property, read/write — the field's contents |
| `Caption` | 2 | property, read/write — the heading |
| `Enabled` | 3 | property, read/write — whether the field and button take input |
| `Refresh` | 4 | method — repaint everything |
| `Change` | 1 | event — somebody typed in the field |
| `Click` | -600 | event — the button was pressed (`DISPID_CLICK`) |

Hosts that bind names late — VBScript, JScript, VB6 through an `Object` variable,
MFC's `COleDispatchDriver` — never needed a type library. PowerShell did: it builds
its member table from type information and will not ask for a name it has not been
told about.

## Safe for scripting

The control claims it, through `IObjectSafety` **and** the two component categories
in the registry, because hosts are split on which one they ask.

The claim is worth stating precisely, since claiming it carelessly is how ActiveX
earned its reputation. The scriptable surface is the six rows above: two strings, a
boolean and a repaint. Nothing in it opens a file, spawns a process, reads the
registry, resolves a host name, or takes a pointer or a window handle from the
caller — and `Load` reads nothing, so untrusted *data* has nothing to be untrusted
with. A script that drives this control as far as it goes has changed some text on a
panel.

That is a claim about the surface as it stands. Add a member that reaches outside the
control and it has to be argued again rather than inherited; the `safety` module is
where the argument lives, next to the code that makes it.

## Testing what Windows cannot check

`registry`, `himetric`, `dispatch` and `view` are the halves that can be tested
without Windows — and they are also the halves that most often go wrong. A control
fails to appear in a host's toolbox for one of about four reasons, all of them a
missing or wrong registry value, and none of them producing an error anywhere. So
those lists are **data**, and the tests check them as data.

## Platform

Windows only; elsewhere the crate compiles to almost nothing.
`crate-type = ["cdylib", "rlib"]`. `unsafe` is necessarily permitted here; every
block carries a `// SAFETY:` comment.

## Status

**M5 complete.** Registers with `regsvr32`, instantiates through `CoCreateInstance`,
sites, activates in place and renders on Windows 11 ARM64. What is *not* proven is a
real form designer hosting it — the pieces are there, but no version of VB6 or the
MFC dialog editor has been run against them.

MIT licensed. Part of [Denise][Denise] — see the [repository README][Denise] and
[docs/design.md] for the whole picture.

[Denise]: https://github.com/bisand/denise
[docs/design.md]: https://github.com/bisand/denise/blob/main/docs/design.md
