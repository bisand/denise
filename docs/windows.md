# Running the Win32 control

`denise-win32` was written against the documentation and has never run. It
compiles for `x86_64-pc-windows-msvc`, its keymap is tested everywhere and its
DIB tests pass on a Windows CI runner — none of which says anything about whether
a real window behaves.

This is how to find out, in a UTM VM on a Mac.

## The VM

UTM on Apple Silicon runs **Windows 11 ARM64** natively through Apple's
hypervisor, which is fast enough that the panel's frame timing means something.
An x86-64 Windows VM works too but runs under emulation, so treat any performance
number from it as fiction.

Two things to set in UTM before installing:

- **At least 4 GB of RAM and 4 cores.** `rustc` and the MSVC linker are the load
  here, not Denise.
- **Enable clipboard and directory sharing.** Getting the repository in and the
  screenshots out is otherwise the slowest part of the whole exercise.

## The toolchain

Inside the VM:

1. **Visual Studio Build Tools**, with the *Desktop development with C++*
   workload. Rust's MSVC targets need the linker and the Windows SDK; there is no
   way around it and `rustup` will say so if it is missing.
2. **rustup** from <https://rustup.rs>. On ARM64 Windows the host toolchain is
   `aarch64-pc-windows-msvc`, which is what you want — the code has no
   x86 assumptions, and the one place pointer width matters
   (`SetWindowLongPtrW` versus `SetWindowLongW`) is already handled by
   `cfg(target_pointer_width)`.

```powershell
rustup show          # confirm the host triple
cargo --version
```

### Two traps before any of this works

**`winget` reports success without installing anything.** This looks like it
worked and does not:

```bat
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "..."
```

It downloads a 4.25 MB `vs_BuildTools.exe` bootstrapper, declares success, and
returns in seconds — while the actual workload is several gigabytes. Install
through the **Visual Studio Installer** GUI instead, where the progress bar is
honest, and verify afterwards:

```bat
"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe" -products * -property installationPath
```

Empty output means nothing is installed. A path containing `BuildTools` is what
you want — beware that SQL Server Management Studio registers as a Visual Studio
instance too, because it is built on the VS shell, and it contains no compiler.

**Do not build from an MSYS2 or Git-Bash shell.** A `CLANGARM64` or `MINGW64`
prompt puts its own `/usr/bin` ahead of everything on `PATH`, which is exactly how
GNU coreutils' `link.exe` ends up shadowing the linker. It also eats the
backslashes out of `cd C:\Users\...`. Use the **ARM64 Native Tools Command
Prompt for VS 2022** from the Start menu: it is `cmd`, it has no coreutils, and it
sets `PATH`, `LIB` and `INCLUDE` together — `PATH` alone is not enough, because
the linker needs `LIB` to find `kernel32.lib`.

### When `link.exe` is the wrong `link.exe`

A build that dies like this is **not** a missing C++ workload, whatever rustc's
hint says:

```text
error: linking with `link.exe` failed: exit code: 1
  = note: link: extra operand '...rcgu.o'
          Try 'link --help' for more information.
```

MSVC's linker never produces that. Its errors are all prefixed `LNK####` —
`LNK1181: cannot open input file` and the like. "extra operand" and "Try
'... --help'" is **GNU coreutils**, which ships a `link.exe` of its own: the
hardlink utility. Something is shadowing the real linker on `PATH`, and it is
almost always `C:\Program Files\Git\usr\bin`, which Git for Windows adds when
installed with the "use Unix tools from the Command Prompt" option.

Confirm it:

```powershell
where.exe link.exe
```

If the first hit is under `Git\usr\bin` or `msys64`, that is the problem. Build
from the **"ARM64 Native Tools Command Prompt for VS 2022"** — or the x64 one to
match your toolchain — which puts MSVC first. To stay in your own shell instead,
prepend the MSVC binaries for the session:

```powershell
$vs = "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC"
$ver = (Get-ChildItem $vs | Sort-Object Name -Descending | Select-Object -First 1).Name
$env:PATH = "$vs\$ver\bin\HostARM64\ARM64;$env:PATH"
```

The reason this is worth a section: the error names the right file and the wrong
cause, so the obvious response is to reinstall a toolchain that was never broken.

## Getting the code in

Either clone it:

```powershell
git clone https://github.com/bisand/denise
cd denise
```

or mount the Mac's checkout through UTM's directory sharing. Cloning is usually
less annoying — a shared folder makes `cargo` rebuild constantly because file
timestamps come back different.

## Running it

```powershell
cargo run -p denise-win32 --example embed
```

A 520x400 window with the panel in it. If it opens and draws, the DIB section,
the top-down row order, the `BitBlt` path and `WM_PAINT` are all correct, which is
most of the backend.

## What to actually test

The example is a **diagnostic**, not just a demo: three lines under the panel
report the last key position with its modifiers, the last committed character
with its codepoint, and the last pointer position with the number of damage
rectangles the frame produced. That is the same trick `denise-evdev`'s `keys`
example plays on Linux, and it is what turned "æøå does not work" into a fixed
AltGr bug in M4. A panel that merely *looks* right tells you nothing about which
layer is lying.

In rough order of how likely each is to be broken:

| | What to do | What should happen | If it does not |
|---|---|---|---|
| 1 | Press **Tab** repeatedly | Focus moves between the field and the two buttons | `WM_GETDLGCODE` is not returning `DLGC_WANTALLKEYS` |
| 2 | **AltGr+2** on a Norwegian layout | `key` says `AltRight`, `text` says `'@' U+0040` | The extended bit is being lost — `AltLeft` means the `lParam` bit 24 test is wrong |
| 3 | Type **æ ø å** | Positions `Quote`, `Semicolon`, `BracketLeft`; characters arrive separately | `WM_CHAR` is not reaching the control, or `TranslateMessage` is missing from the loop |
| 4 | **Press a button, drag off it, release** | The button un-presses and emits nothing | `SetCapture` is not working; the widget will stay lit |
| 5 | **Drag off the left edge** while pressed | `mouse` shows a negative x | The `lParam` halves are being read unsigned |
| 6 | **Scroll the wheel** over the panel | `wheel` shows a signed value | Wheel messages carry screen coordinates; if this only works with the window at the top-left, `ScreenToClient` is missing |
| 7 | **Resize the window** | `damage` settles to a small number, not the whole client area every frame | The incremental path is not being used |
| 8 | **Leave it alone** | The caret blinks and `damage` stays at 1 | The tree is over-damaging |
| 9 | **Move the pointer out** | `mouse left the control`, hover clears | `TrackMouseEvent` is not being re-armed after each leave |
| 10 | **Move to a display with different DPI** | Everything rescales | `WM_DPICHANGED` reaches top-level windows only; a child control finds out from its parent or not at all |

The ones I would bet on being wrong are 1, 2 and 10 — message ordering, the
extended bit, and DPI — because they are exactly the parts no compiler checks and
no unit test reaches.

## Reporting back

`key`, `text` and `mouse` lines are the useful thing to copy out. A screenshot of
the window with the diagnostic visible says more than a description, and UTM's
clipboard sharing makes that a paste rather than a file transfer.

## What is not testable this way

The ActiveX shim, because it does not exist yet — only its registration table.
Testing that needs a container that can load a COM control (VB6, an MFC dialog, or
`Tstcon32.exe` from the old Platform SDK), and it should not be written until the
control underneath it has been run at least once. Which is what this page is for.
