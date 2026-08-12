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
//!
//! # What a real container does that this does not
//!
//! Menus, accelerators, ambient properties, an undo stack, and `IDispatch` for
//! events and scripting. Every one of those methods is a stub here, and they are
//! stubs a container is allowed to have — `E_NOTIMPL` from `GetMoniker` is a
//! perfectly ordinary answer.

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

    use std::cell::RefCell;

    use denise_activex::CLSID_DENISE_PANEL;
    use windows::Win32::Foundation::{E_NOTIMPL, HWND, LPARAM, LRESULT, RECT, S_OK, SIZE, WPARAM};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, IMoniker, IPersistStreamInit,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Ole::{
        IOleClientSite, IOleClientSite_Impl, IOleContainer, IOleInPlaceActiveObject,
        IOleInPlaceFrame, IOleInPlaceFrame_Impl, IOleInPlaceSite, IOleInPlaceSite_Impl,
        IOleInPlaceUIWindow, IOleInPlaceUIWindow_Impl, IOleObject, IOleWindow_Impl,
        OLECLOSE_NOSAVE, OLEGETMONIKER, OLEINPLACEFRAMEINFO, OLEIVERB_INPLACEACTIVATE,
        OLEMENUGROUPWIDTHS, OLEWHICHMK,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
        GetMessageW, HMENU, MSG, PostQuitMessage, RegisterClassExW, SW_SHOW, ShowWindow,
        TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    };
    use windows_core::{BOOL, IUnknownImpl, Interface, OutRef, PCWSTR, Ref, implement, w};

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
        println!("close the window to quit");

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

        // A container that drops a control without closing it leaves the control's
        // window parented to a destroyed one.
        // SAFETY: the control is still live here.
        unsafe {
            let _ = object.Close(OLECLOSE_NOSAVE);
            let _ = object.SetClientSite(None);
        }
        Ok(())
    }

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
