//! A minimal ActiveX container, so the control can be tested without Tstcon32.
//!
//! ```text
//! cargo run -p denise-activex --example host
//! ```
//!
//! The classic tool for this — `Tstcon32.exe`, the ActiveX Control Test
//! Container — shipped with Visual Studio 6 and the old Platform SDKs, and is on
//! no modern machine. Rather than hunt for a twenty-year-old binary, this is the
//! twenty per cent of a container that matters: create the control, site it,
//! activate it in place, give it a rectangle, and pump messages.
//!
//! It talks to the control the same way a real container does — through
//! `CoCreateInstance` and the registry — so it proves the *registered* server
//! works, not just the code in this repository. Register first:
//!
//! ```text
//! cargo build -p denise-activex --release
//! regsvr32 target\release\denise_activex.dll     (elevated)
//! ```
//!
//! # What it proves, in order
//!
//! Each step fails distinctly, and the output says which one:
//!
//! 1. `CoCreateInstance` — registration, the class factory, `IUnknown`.
//! 2. `QueryInterface(IOleObject)` — the control is an OLE object at all.
//! 3. `SetClientSite` — it accepts a container.
//! 4. `InitNew` — `IPersistStreamInit`, which VB6 will not proceed without.
//! 5. `DoVerb(INPLACEACTIVATE)` — the control asks the site for a parent window
//!    and creates its child `HWND` inside it. A window appearing means the whole
//!    path works.
//! 6. `FindConnectionPoint` and `Advise` — the control accepts an event sink.
//! 7. `GetIDsOfNames` and `Invoke` — properties are set and read **by name**,
//!    which is precisely what a script does.
//! 8. `GetTypeInfo` — the type library `regsvr32` installed, read back out of the
//!    registered server. The unit tests build a library into a temporary file and
//!    check it; only this can check that the `.tlb` ended up beside the DLL, that
//!    the registry points at it, and that the control hands it over.
//! 9. `IViewObject2::Draw` — the design-time view, drawn into a memory device
//!    context *before* the control is sited and printed as text. A form editor
//!    asks for this with no window and no container anywhere, so it is the one
//!    step here that deliberately happens before step 3.
//!
//! Then it is interactive: typing in the field raises `Change`, pressing the
//! button raises `Click`, and the click handler assigns to `Caption` — from
//! inside the event the control itself raised, which is the re-entrant case the
//! control is written to survive.
//!
//! # What a real container does that this does not
//!
//! Menus, accelerators, ambient properties and an undo stack. Every one of those
//! methods is a stub here, and they are stubs a container is allowed to have —
//! `E_NOTIMPL` from `GetMoniker` is a perfectly ordinary answer.

#[cfg(not(windows))]
fn main() {
    eprintln!("this example needs Windows and a registered denise_activex.dll");
}

#[cfg(windows)]
fn main() -> windows_core::Result<()> {
    app::start()
}

#[cfg(windows)]
mod app {

    use std::cell::{Cell, RefCell};
    use std::mem::ManuallyDrop;

    use denise_activex::dispatch::{self, DISPID_CHANGE, DISPID_CLICK};
    use denise_activex::himetric::himetric_to_pixels;
    use denise_activex::{CLSID_DENISE_PANEL, DIID_DENISE_PANEL_EVENTS};
    use windows::Win32::Foundation::{
        DISP_E_MEMBERNOTFOUND, E_NOTIMPL, HWND, LPARAM, LRESULT, RECT, RECTL, S_OK, SIZE,
        VARIANT_FALSE, VARIANT_TRUE, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GdiFlush, SelectObject,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET,
        DISPATCH_PROPERTYPUT, DISPPARAMS, DVASPECT_CONTENT, EXCEPINFO, IConnectionPoint,
        IConnectionPointContainer, IDispatch, IDispatch_Impl, IMoniker, IPersistStreamInit,
        ITypeInfo,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Ole::{
        DISPID_PROPERTYPUT, IOleClientSite, IOleClientSite_Impl, IOleContainer,
        IOleInPlaceActiveObject, IOleInPlaceFrame, IOleInPlaceFrame_Impl, IOleInPlaceSite,
        IOleInPlaceSite_Impl, IOleInPlaceUIWindow, IOleInPlaceUIWindow_Impl, IOleObject,
        IOleWindow_Impl, IViewObject2, OLECLOSE_NOSAVE, OLEGETMONIKER, OLEINPLACEFRAMEINFO,
        OLEIVERB_INPLACEACTIVATE, OLEMENUGROUPWIDTHS, OLEWHICHMK,
    };
    use windows::Win32::System::Variant::{
        VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VariantClear,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
        GetMessageW, HMENU, MSG, PostQuitMessage, RegisterClassExW, SW_SHOW, ShowWindow,
        TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    };
    use windows_core::{
        BOOL, BSTR, GUID, IUnknownImpl, Interface, OutRef, PCWSTR, Ref, implement, w,
    };

    pub fn start() -> windows_core::Result<()> {
        // SAFETY: apartment threading, which is what the control is registered for
        // and what a window procedure requires.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        let result = run();
        // SAFETY: paired with the initialise above.
        unsafe { CoUninitialize() };
        result
    }

    fn run() -> windows_core::Result<()> {
        let window = create_window()?;

        // 1. The registry, the class factory and IUnknown, in one call.
        println!("CoCreateInstance...");
        // SAFETY: an in-process server for a class id this example owns.
        let object: IOleObject =
            unsafe { CoCreateInstance(&CLSID_DENISE_PANEL, None, CLSCTX_INPROC_SERVER) }?;
        println!("  ok — the control exists and is an IOleObject");

        // The description, before anything opens a window over the output. It
        // needs only `IDispatch`, so asking here also fails early and clearly when
        // the registered DLL predates the automation surface.
        //
        //    `cargo run` rebuilds this example and rebuilds nothing COM will load:
        //    the server is whatever sits at the registered path.
        let dispatch: IDispatch = object.cast().map_err(|_| stale("IDispatch"))?;
        println!("\nautomation surface:");
        print!("{}", dispatch::describe());
        describe_from_the_library(&dispatch);
        println!();

        // The design-time view, before the control is sited: no container, no
        // window, no activation. See the function.
        draw_the_design_time_view(&object);

        // 2. A container for it to talk back to.
        let site: IOleClientSite = Host {
            window,
            rect: RefCell::new(client_rect(window)),
        }
        .into();
        // SAFETY: `site` is a live object this example owns for the whole run.
        unsafe { object.SetClientSite(&site) }?;
        println!("  ok — SetClientSite");

        // 3. VB6 will not load a control that has not been initialised, and the
        //    order matters: OLEMISC_SETCLIENTSITEFIRST is why this comes after.
        if let Ok(persist) = object.cast::<IPersistStreamInit>() {
            // SAFETY: a live interface on the control.
            unsafe { persist.InitNew() }?;
            println!("  ok — IPersistStreamInit::InitNew");
        }

        // SAFETY: names for a title bar the control does not have; it ignores them.
        unsafe { object.SetHostNames(w!("Denise host"), w!("panel")) }?;

        // 4. The one that creates a window.
        let rect = client_rect(window);
        println!("DoVerb(OLEIVERB_INPLACEACTIVATE)...");
        // SAFETY: `window` is live, `rect` is a live local, and the site is set.
        unsafe {
            object.DoVerb(
                OLEIVERB_INPLACEACTIVATE.0,
                core::ptr::null(),
                &site,
                0,
                window,
                &rect,
            )
        }?;
        println!("  ok — the control should now have a window\n");

        // 5. Events. `FindConnectionPoint` is how every host that sinks a
        //    control's events finds them, and `IDispatch` is the only thing the
        //    control asks of a sink.
        let container: IConnectionPointContainer = object
            .cast()
            .map_err(|_| stale("IConnectionPointContainer"))?;
        // SAFETY: a live interface on the control, and a GUID this example owns
        // for the call.
        let point: IConnectionPoint =
            unsafe { container.FindConnectionPoint(&DIID_DENISE_PANEL_EVENTS) }?;
        let sink: IDispatch = Sink {
            panel: dispatch.clone(),
            clicks: Cell::new(0),
        }
        .into();
        // SAFETY: `sink` is a live object this example owns for the whole run.
        let cookie = unsafe { point.Advise(&sink) }?;
        println!("\n  ok — Advise, cookie {cookie}");

        // 6. Scripting it: by name, through `GetIDsOfNames` and `Invoke`, with no
        //    type library anywhere. The same path VBScript takes.
        put_string(&dispatch, "Caption", "Skrevet av verten")?;
        put_string(&dispatch, "Text", "hallo")?;
        println!("  ok — Caption and Text assigned by name");
        println!(
            "       Text reads back as {:?}",
            get_string(&dispatch, "Text")?
        );

        // And a boolean, the one type Basic spells differently from everyone
        // else: `True` is -1.
        put_bool(&dispatch, "Enabled", false)?;
        println!(
            "  ok — Enabled = False reads back as {}",
            get_bool(&dispatch, "Enabled")?
        );
        put_bool(&dispatch, "Enabled", true)?;

        println!("\ntype in the field and press the button; close the window to quit");

        // SAFETY: `window` is live.
        unsafe {
            let _ = ShowWindow(window, SW_SHOW);
        }

        let mut message = MSG::default();
        // SAFETY: the standard message loop.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            // SAFETY: `message` was just filled by `GetMessageW`.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        // The sink holds the control and the control holds the sink. `Close` would
        // break that cycle anyway, but a container that leans on the control to
        // tidy up after it is a container that leaks against a control that does
        // not.
        // SAFETY: `cookie` is the one `Advise` returned, released once.
        unsafe {
            let _ = point.Unadvise(cookie);
        }

        // A container that drops a control without closing it leaves the control's
        // window parented to a destroyed one.
        // SAFETY: the control is still live here.
        unsafe {
            let _ = object.Close(OLECLOSE_NOSAVE);
            let _ = object.SetClientSite(None);
        }
        Ok(())
    }

    /// Reads the control's own type library back, the way PowerShell does.
    ///
    /// The tests build a library into a temporary file and check it. This asks the
    /// *registered* server for the one `regsvr32` installed, which is the half a
    /// test cannot reach: it proves the `.tlb` was written beside the DLL, that
    /// the registry points at it, and that the control hands it over.
    fn describe_from_the_library(object: &IDispatch) {
        // SAFETY: `object` is the control, live for the call.
        let count = unsafe { object.GetTypeInfoCount() }.unwrap_or(0);
        if count == 0 {
            println!(
                "\n  no type library — either this DLL predates it, or the .tlb is not\n  \
                 beside the DLL where registration put it. PowerShell will show no members."
            );
            return;
        }

        // SAFETY: index zero is the only description a control with one has.
        let info = match unsafe { object.GetTypeInfo(0, 0) } {
            Ok(info) => info,
            Err(e) => {
                println!("\n  GetTypeInfo failed: {e}");
                return;
            }
        };

        println!("\nfrom the registered type library:");
        for member in dispatch::MEMBERS {
            let wide: Vec<u16> = member
                .name
                .encode_utf16()
                .chain(core::iter::once(0))
                .collect();
            let mut dispid = 0i32;
            // SAFETY: `wide` outlives the call; one name in, one dispid out.
            let found = unsafe { info.GetIDsOfNames(&PCWSTR(wide.as_ptr()), 1, &mut dispid) };
            match found {
                Ok(()) => println!("  {:<10} dispid {dispid}", member.name),
                Err(e) => println!("  {:<10} NOT NAMED — {e}", member.name),
            }
        }
    }

    /// The design-time view: what a form editor draws before the control runs.
    ///
    /// Deliberately called before `SetClientSite` below. There is no container,
    /// no window and no activation at this point, and that is the whole claim —
    /// a form editor drops the control on a design surface and asks what it looks
    /// like, and a control with no answer is a blank rectangle until the form is
    /// run.
    ///
    /// It is printed because otherwise there is nothing to look at: this path
    /// never reaches the screen. The unit tests check the same pixels and count
    /// them; a person needs to see that it is a *panel*.
    fn draw_the_design_time_view(object: &IOleObject) {
        let Ok(view) = object.cast::<IViewObject2>() else {
            println!(
                "  no IViewObject2 — a form editor would show a blank rectangle.\n  \
                 Almost certainly an older DLL still at the registered path."
            );
            return;
        };

        // The size the control says it is, which is what a form editor gives it
        // when it is first dropped.
        // SAFETY: `view` is a live interface on the control; a null target device
        // means the screen.
        let extent = unsafe { view.GetExtent(DVASPECT_CONTENT, -1, core::ptr::null()) };
        let Ok(extent) = extent else {
            println!("  GetExtent failed");
            return;
        };
        let width = himetric_to_pixels(extent.cx).clamp(1, 2048);
        let height = himetric_to_pixels(extent.cy).clamp(1, 2048);

        let Some(canvas) = Canvas::new(width, height) else {
            println!("  could not allocate a device context to draw into");
            return;
        };
        let rect = RECTL {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        // SAFETY: `canvas` owns a live memory DC and `rect` is a live local. The
        // target device, the window bounds and the continuation callback are all
        // optional and none of them apply to a screen-compatible memory DC.
        let drawn = unsafe {
            view.Draw(
                DVASPECT_CONTENT,
                -1,
                core::ptr::null_mut(),
                None,
                None,
                canvas.dc,
                Some(&rect),
                None,
                None,
                0,
            )
        };
        match drawn {
            Ok(()) => {
                println!("design-time view — {width}x{height}, no window, no site:");
                println!("{}", as_text(&canvas.pixels(), width, height));
            }
            Err(e) => println!("  Draw failed: {e}"),
        }
    }

    /// Turns a picture into something a terminal can show.
    ///
    /// Point-sampled and by luminance, which is enough to recognise the card, the
    /// field and the button. Characters are about twice as tall as they are wide,
    /// hence the halved row count.
    fn as_text(pixels: &[u32], width: i32, height: i32) -> String {
        const RAMP: &[u8] = b" .:-=+*#%@";
        const COLUMNS: i32 = 64;
        let rows = (COLUMNS * height / width / 2).max(1);

        let mut text = String::new();
        for row in 0..rows {
            text.push_str("  ");
            for column in 0..COLUMNS {
                let x = column * width / COLUMNS;
                let y = row * height / rows;
                let pixel = pixels[(y * width + x) as usize];
                // The DIB is 0x00RRGGBB. The usual weights, in integers.
                let (r, g, b) = ((pixel >> 16) & 0xFF, (pixel >> 8) & 0xFF, pixel & 0xFF);
                let luminance = (r * 54 + g * 183 + b * 19) >> 8;
                let step = (luminance as usize * (RAMP.len() - 1)) / 255;
                text.push(RAMP[step] as char);
            }
            text.push('\n');
        }
        text
    }

    /// A memory device context whose pixels can be read back afterwards.
    struct Canvas {
        dc: windows::Win32::Graphics::Gdi::HDC,
        bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
        previous: windows::Win32::Graphics::Gdi::HGDIOBJ,
        bits: *mut u32,
        count: usize,
    }

    impl Canvas {
        fn new(width: i32, height: i32) -> Option<Self> {
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            // Negative for top-down, so row 0 is the top one — which is what the
            // sampling above assumes.
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;

            let mut bits = core::ptr::null_mut();
            // SAFETY: `info` describes a 32-bit BI_RGB DIB and `bits` receives the
            // allocation. A null DC is valid for `DIB_RGB_COLORS`.
            let bitmap =
                unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
                    .ok()?;
            // SAFETY: a null argument asks for a screen-compatible memory DC.
            let dc = unsafe { CreateCompatibleDC(None) };
            // SAFETY: both handles are live and a bitmap is valid in a memory DC.
            let previous = unsafe { SelectObject(dc, bitmap.into()) };
            Some(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                count: (width * height) as usize,
            })
        }

        /// The pixels, after making GDI finish what it batched.
        ///
        /// `GdiFlush` is not optional: GDI batches drawing per thread, and a DIB
        /// section read without it returns what was there before the blit —
        /// intermittently.
        fn pixels(&self) -> Vec<u32> {
            // SAFETY: no arguments; it drains this thread's own batch.
            unsafe {
                let _ = GdiFlush();
            }
            // SAFETY: `bits` is the DIB's allocation, exactly `count` words, and
            // nothing else holds it.
            unsafe { core::slice::from_raw_parts(self.bits, self.count) }.to_vec()
        }
    }

    impl Drop for Canvas {
        fn drop(&mut self) {
            // SAFETY: every handle was created in `new` and released once. The
            // previous object goes back first, because a DC still holding our
            // bitmap will not release it.
            unsafe {
                SelectObject(self.dc, self.previous);
                let _ = DeleteDC(self.dc);
                let _ = DeleteObject(self.bitmap.into());
            }
        }
    }

    /// The error for an interface the registered server does not have.
    ///
    /// `E_NOINTERFACE` on its own sends a reader looking for a bug in the control.
    /// It is nearly always a stale DLL: `cargo run` rebuilds this example, and
    /// rebuilds nothing that COM will load.
    fn stale(interface: &str) -> windows_core::Error {
        windows_core::Error::new(
            windows::Win32::Foundation::E_NOINTERFACE,
            format!(
                "the registered server has no {interface}. It is almost certainly \
                 an older DLL still at the registered path — `cargo run` rebuilt \
                 this example and not the server. Build it and try again:\n\
                 \n    cargo build -p denise-activex --release\n\
                 \nThe path in the registry does not change, so there is no need \
                 to run regsvr32 again."
            ),
        )
    }

    // ---------------------------------------------------------------- the sink

    /// The whole of what a host needs to receive events: `IDispatch`, and a match
    /// on two numbers.
    ///
    /// There is no type library and no vtable to implement, which is both halves
    /// of the trade the control makes — a sink is this short, and in exchange
    /// nothing but documentation can tell you what the dispids are.
    #[implement(IDispatch)]
    struct Sink {
        /// The control, so a handler can script it back. This is the reference
        /// that makes the cycle `Unadvise` exists to break.
        panel: IDispatch,
        clicks: Cell<u32>,
    }

    impl IDispatch_Impl for Sink_Impl {
        fn GetTypeInfoCount(&self) -> windows_core::Result<u32> {
            Ok(0)
        }

        fn GetTypeInfo(&self, _index: u32, _lcid: u32) -> windows_core::Result<ITypeInfo> {
            Err(E_NOTIMPL.into())
        }

        fn GetIDsOfNames(
            &self,
            _riid: *const GUID,
            _names: *const PCWSTR,
            _count: u32,
            _lcid: u32,
            _out: *mut i32,
        ) -> windows_core::Result<()> {
            // A sink is called, never queried: the control already knows the
            // numbers it is going to invoke.
            Err(E_NOTIMPL.into())
        }

        fn Invoke(
            &self,
            dispid: i32,
            _riid: *const GUID,
            _lcid: u32,
            _flags: DISPATCH_FLAGS,
            _params: *const DISPPARAMS,
            _result: *mut VARIANT,
            _exception: *mut EXCEPINFO,
            _argument_error: *mut u32,
        ) -> windows_core::Result<()> {
            match dispid {
                DISPID_CHANGE => {
                    println!(
                        "  event: Change — Text is now {:?}",
                        get_string(&self.panel, "Text")?
                    );
                    Ok(())
                }
                DISPID_CLICK => {
                    let clicks = self.clicks.get() + 1;
                    self.clicks.set(clicks);
                    println!("  event: Click ({clicks})");
                    // Assigning to the control from inside an event the control
                    // itself raised. This is the re-entrant path, and the reason a
                    // property put made while the tree is running records the
                    // change rather than pushing it straight back in.
                    put_string(&self.panel, "Caption", &format!("Klikket {clicks} ganger"))
                }
                _ => Err(DISP_E_MEMBERNOTFOUND.into()),
            }
        }
    }

    // ----------------------------------------------------- driving it by name

    /// Looks a member up by name, exactly as a script does.
    fn dispid_of(object: &IDispatch, name: &str) -> windows_core::Result<i32> {
        let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
        let name = PCWSTR(wide.as_ptr());
        let mut dispid = 0i32;
        // SAFETY: `wide` outlives the call, one name is passed and one dispid is
        // written. `riid` is reserved and must be `IID_NULL`; the locale is
        // ignored by a control with no type library.
        unsafe { object.GetIDsOfNames(&GUID::zeroed(), &name, 1, 0, &mut dispid) }?;
        Ok(dispid)
    }

    /// Assigns a string to a property by name.
    fn put_string(object: &IDispatch, name: &str, value: &str) -> windows_core::Result<()> {
        let mut argument = variant(
            VT_BSTR,
            VARIANT_0_0_0 {
                bstrVal: ManuallyDrop::new(BSTR::from(value)),
            },
        );
        let result = invoke_put(object, name, &mut argument);
        // SAFETY: `argument` is ours and holds the only reference to that `BSTR`.
        unsafe {
            let _ = VariantClear(&mut argument);
        }
        result
    }

    /// Assigns a boolean to a property by name.
    fn put_bool(object: &IDispatch, name: &str, value: bool) -> windows_core::Result<()> {
        let mut argument = variant(
            VT_BOOL,
            VARIANT_0_0_0 {
                boolVal: if value { VARIANT_TRUE } else { VARIANT_FALSE },
            },
        );
        invoke_put(object, name, &mut argument)
    }

    /// The two fiddly parts of a property put, both of which every hand-written
    /// container gets wrong once: it carries the named argument
    /// `DISPID_PROPERTYPUT` saying which slot holds the value, and `rgvarg` is in
    /// **reverse** order. With one argument the two conventions coincide, which is
    /// exactly why this looks right in every example and is right by accident.
    fn invoke_put(
        object: &IDispatch,
        name: &str,
        argument: &mut VARIANT,
    ) -> windows_core::Result<()> {
        let dispid = dispid_of(object, name)?;
        let mut named = DISPID_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: argument,
            rgdispidNamedArgs: &mut named,
            cArgs: 1,
            cNamedArgs: 1,
        };
        // SAFETY: `params` borrows two live locals for the length of the call and
        // describes them correctly. No result is wanted.
        unsafe {
            object.Invoke(
                dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_PROPERTYPUT,
                &params,
                None,
                None,
                None,
            )
        }
    }

    /// Reads a string property by name.
    fn get_string(object: &IDispatch, name: &str) -> windows_core::Result<String> {
        let mut result = invoke_get(object, name)?;
        // SAFETY: the control answers `Text` and `Caption` with a `VT_BSTR`, and a
        // null `BSTR` reads as the empty string.
        let text = unsafe { result.Anonymous.Anonymous.Anonymous.bstrVal.to_string() };
        // SAFETY: `result` is ours, and the `BSTR` in it is the caller's to free.
        unsafe {
            let _ = VariantClear(&mut result);
        }
        Ok(text)
    }

    /// Reads a boolean property by name.
    fn get_bool(object: &IDispatch, name: &str) -> windows_core::Result<bool> {
        let result = invoke_get(object, name)?;
        // SAFETY: the control answers `Enabled` with a `VT_BOOL`. Compared against
        // zero, not against 1: Basic's `True` is -1.
        Ok(unsafe { result.Anonymous.Anonymous.Anonymous.boolVal.0 != 0 })
    }

    fn invoke_get(object: &IDispatch, name: &str) -> windows_core::Result<VARIANT> {
        let dispid = dispid_of(object, name)?;
        let params = DISPPARAMS::default();
        let mut result = VARIANT::default();
        // SAFETY: `params` describes no arguments, and `result` is a live local
        // initialised to `VT_EMPTY`, which is what the contract asks of a caller.
        unsafe {
            object.Invoke(
                dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_PROPERTYGET,
                &params,
                Some(&mut result),
                None,
                None,
            )
        }?;
        Ok(result)
    }

    /// Assembles a `VARIANT` from its tag and one arm of its union.
    ///
    /// Built whole rather than field by field: a `VARIANT` is three nested unions
    /// deep, and assigning through them writes *over* whatever arm was there
    /// before — which is how a `BSTR` gets leaked or freed twice.
    fn variant(vt: VARENUM, payload: VARIANT_0_0_0) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: payload,
                }),
            },
        }
    }

    /// Keeps `DISPATCH_METHOD` named: it is what the control sends this sink, and
    /// a reader looking for the other side of the call should find it here.
    const _: DISPATCH_FLAGS = DISPATCH_METHOD;

    fn create_window() -> windows_core::Result<HWND> {
        // SAFETY: a null module name asks for this process's handle.
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(host_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Denise.Host"),
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised.
        unsafe { RegisterClassExW(&class) };
        // SAFETY: the class was just registered.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("Denise.Host"),
                w!("Denise — ActiveX container"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                520,
                400,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }
    }

    fn client_rect(window: HWND) -> RECT {
        let mut rect = RECT::default();
        // SAFETY: `window` is live and `rect` is a live local.
        unsafe {
            let _ = GetClientRect(window, &mut rect);
        }
        rect
    }

    extern "system" fn host_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if message == WM_DESTROY {
            // SAFETY: ends the message loop.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        // SAFETY: the standard fallback.
        unsafe { DefWindowProcW(hwnd, message, w, l) }
    }

    /// The container, as far as the control can see: a client site, an in-place site
    /// and a frame, all on one object.
    ///
    /// A real container usually splits these across a document and a frame. One
    /// object is legitimate and much shorter, because `QueryInterface` is what the
    /// control uses to find them and it does not care where they live.
    #[implement(IOleClientSite, IOleInPlaceSite, IOleInPlaceFrame)]
    struct Host {
        window: HWND,
        /// Where the control is allowed to draw, in client coordinates.
        rect: RefCell<RECT>,
    }

    impl IOleClientSite_Impl for Host_Impl {
        fn SaveObject(&self) -> windows_core::Result<()> {
            // Nothing persists here, and a container that cannot save says so.
            Err(E_NOTIMPL.into())
        }

        fn GetMoniker(
            &self,
            _assign: &OLEGETMONIKER,
            _which: &OLEWHICHMK,
        ) -> windows_core::Result<IMoniker> {
            Err(E_NOTIMPL.into())
        }

        fn GetContainer(&self) -> windows_core::Result<IOleContainer> {
            // A control uses this to enumerate its siblings. There are none.
            Err(E_NOTIMPL.into())
        }

        fn ShowObject(&self) -> windows_core::Result<()> {
            Ok(())
        }

        fn OnShowWindow(&self, _show: BOOL) -> windows_core::Result<()> {
            Ok(())
        }

        fn RequestNewObjectLayout(&self) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }
    }

    impl IOleWindow_Impl for Host_Impl {
        fn GetWindow(&self) -> windows_core::Result<HWND> {
            // The one method that actually matters: this is where the control gets
            // the parent for its child window.
            Ok(self.window)
        }

        fn ContextSensitiveHelp(&self, _entering: BOOL) -> windows_core::Result<()> {
            Ok(())
        }
    }

    impl IOleInPlaceSite_Impl for Host_Impl {
        fn CanInPlaceActivate(&self) -> windows_core::Result<()> {
            // `S_OK` means yes. A container that returns `S_FALSE` here gets a
            // control that never activates and never explains why.
            Ok(())
        }

        fn OnInPlaceActivate(&self) -> windows_core::Result<()> {
            println!("  site: OnInPlaceActivate");
            Ok(())
        }

        fn OnUIActivate(&self) -> windows_core::Result<()> {
            println!("  site: OnUIActivate");
            Ok(())
        }

        fn GetWindowContext(
            &self,
            frame: OutRef<'_, IOleInPlaceFrame>,
            doc: OutRef<'_, IOleInPlaceUIWindow>,
            position: *mut RECT,
            clip: *mut RECT,
            info: *mut OLEINPLACEFRAMEINFO,
        ) -> windows_core::Result<()> {
            // The control asks where it may draw and who its frame is. Getting this
            // wrong is the usual reason an in-place control appears at 0x0.
            let rect = *self.rect.borrow();
            if !position.is_null() {
                // SAFETY: the control promises a writable RECT when it passes one.
                unsafe { position.write(rect) };
            }
            if !clip.is_null() {
                // SAFETY: as above.
                unsafe { clip.write(rect) };
            }
            if !info.is_null() {
                // SAFETY: as above. `cb` must be set or the control cannot tell how
                // much of the structure is valid.
                unsafe {
                    (*info).cb = size_of::<OLEINPLACEFRAMEINFO>() as u32;
                    (*info).fMDIApp = false.into();
                    (*info).hwndFrame = self.window;
                    (*info).haccel = Default::default();
                    (*info).cAccelEntries = 0;
                }
            }
            let this: IOleInPlaceFrame = self.to_interface();
            let _ = frame.write(Some(this));
            // No separate document window, which is what `None` means here.
            let _ = doc.write(None);
            Ok(())
        }

        fn Scroll(&self, _extent: &SIZE) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn OnUIDeactivate(&self, _undoable: BOOL) -> windows_core::Result<()> {
            Ok(())
        }

        fn OnInPlaceDeactivate(&self) -> windows_core::Result<()> {
            println!("  site: OnInPlaceDeactivate");
            Ok(())
        }

        fn DiscardUndoState(&self) -> windows_core::Result<()> {
            Ok(())
        }

        fn DeactivateAndUndo(&self) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn OnPosRectChange(&self, position: *const RECT) -> windows_core::Result<()> {
            if !position.is_null() {
                // SAFETY: the control promises a readable RECT.
                *self.rect.borrow_mut() = unsafe { *position };
            }
            Ok(())
        }
    }

    impl IOleInPlaceUIWindow_Impl for Host_Impl {
        fn GetBorder(&self) -> windows_core::Result<RECT> {
            Err(E_NOTIMPL.into())
        }

        fn RequestBorderSpace(&self, _widths: *const RECT) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn SetBorderSpace(&self, _widths: *const RECT) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn SetActiveObject(
            &self,
            _active: Ref<'_, IOleInPlaceActiveObject>,
            _name: &PCWSTR,
        ) -> windows_core::Result<()> {
            Ok(())
        }
    }

    impl IOleInPlaceFrame_Impl for Host_Impl {
        fn InsertMenus(
            &self,
            _shared: HMENU,
            _widths: *mut OLEMENUGROUPWIDTHS,
        ) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn SetMenu(&self, _shared: HMENU, _ole: isize, _active: HWND) -> windows_core::Result<()> {
            // No menu merging. Returning success rather than E_NOTIMPL because a
            // control that merges nothing still calls this with nothing.
            Ok(())
        }

        fn RemoveMenus(&self, _shared: HMENU) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn SetStatusText(&self, _text: &PCWSTR) -> windows_core::Result<()> {
            Ok(())
        }

        fn EnableModeless(&self, _enable: BOOL) -> windows_core::Result<()> {
            Ok(())
        }

        fn TranslateAccelerator(&self, _message: *const MSG, _id: u16) -> windows_core::Result<()> {
            // `S_FALSE` means "not mine", which for a container with no accelerators
            // is every message. The control then handles it itself.
            Err(windows_core::Error::from_hresult(
                windows::Win32::Foundation::S_FALSE,
            ))
        }
    }

    /// Keeps `S_OK` named, since the interesting returns above are all failures.
    const _: windows_core::HRESULT = S_OK;
}
