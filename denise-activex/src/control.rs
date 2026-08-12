//! The OLE control object: what a container actually holds.
//!
//! A windowed, inside-out, activate-when-visible control, which is the shape a
//! VB6 form or an MFC dialog expects. The container calls `SetClientSite`, then
//! `DoVerb(OLEIVERB_INPLACEACTIVATE)`, and at that point this creates a
//! [`DeniseControl`] child window inside the container's own. From then on
//! Windows delivers input straight to it and the container is out of the path.
//!
//! # Why `RefCell`
//!
//! COM methods take `&self`, and every one of these has to mutate something. The
//! class is registered `ThreadingModel=Apartment`, so the object only ever runs
//! on the thread that created it — which is also the thread its window procedure
//! runs on. That is what makes a `RefCell` the right tool rather than a lock.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use denise::{InputEvent, Rect, Role, Size, Surface, Theme};
use denise_ui::widgets::{Button, Label, Panel, TextInput};
use denise_ui::{NodeId, Ui};
use denise_win32::{ControlDelegate, DeniseControl, DibSurface};
use windows::Win32::Foundation::{
    DV_E_DVASPECT, E_FAIL, E_NOTIMPL, E_POINTER, HWND, RECT, RECTL, SIZE,
};
use windows::Win32::Graphics::Gdi::{
    HALFTONE, HDC, LOGPALETTE, RestoreDC, SRCCOPY, SaveDC, SetBrushOrgEx, SetStretchBltMode,
    StretchBlt,
};
use windows::Win32::System::Com::{
    ADVF_ONLYONCE, CoTaskMemAlloc, DISPATCH_METHOD, DISPPARAMS, DVASPECT, DVASPECT_CONTENT,
    DVTARGETDEVICE, IAdviseSink, IDataObject, IEnumSTATDATA, IMoniker, IPersist_Impl,
    IPersistStreamInit_Impl, IStream, ITypeInfo,
};
use windows::Win32::System::Ole::{
    IEnumOLEVERB, IOleClientSite, IOleControl_Impl, IOleInPlaceObject_Impl, IOleInPlaceSite,
    IOleObject_Impl, IOleWindow_Impl, IViewObject_Impl, IViewObject2_Impl, OLECLOSE, OLEGETMONIKER,
    OLEIVERB_HIDE, OLEIVERB_INPLACEACTIVATE, OLEIVERB_SHOW, OLEIVERB_UIACTIVATE, OLEMISC,
    OLEWHICHMK, USERCLASSTYPE,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, SWP_NOZORDER, SetWindowPos};
use windows_core::{BOOL, GUID, HRESULT, Interface, OutRef, Ref, implement};

use crate::dispatch;
use crate::himetric::{himetric_to_pixels, pixels_to_himetric};
use crate::model::{Model, Shared};
use crate::registry::MISC_STATUS;
use crate::server::CLSID_DENISE_PANEL;
use crate::view;

/// The message the panel's button emits. One button and one message, because an
/// automation surface without a type library is late-bound: every addition is
/// something a host has to discover by reading documentation rather than by
/// pressing `.`.
const MSG_ACTIVATED: u32 = 1;

/// The panel a container embeds.
#[implement(
    windows::Win32::System::Ole::IOleObject,
    windows::Win32::System::Ole::IOleInPlaceObject,
    windows::Win32::System::Ole::IOleControl,
    windows::Win32::System::Com::IPersistStreamInit,
    windows::Win32::System::Com::IDispatch,
    windows::Win32::System::Com::IConnectionPointContainer,
    windows::Win32::System::Com::IConnectionPoint,
    windows::Win32::System::Ole::IViewObject2
)]
pub struct DenisePanel {
    pub(crate) state: RefCell<PanelState>,
    /// The scriptable half, shared with the tree inside the child window. A
    /// separate cell from `state` on purpose: a property put reaches both, and
    /// one lock over the two would deadlock against itself the first time an
    /// event handler assigned to a property.
    pub(crate) model: Shared,
}

pub(crate) struct PanelState {
    /// The container's site, from `SetClientSite`. `None` before it arrives and
    /// after `Close`.
    site: Option<IOleClientSite>,
    /// The child window, once activated in place.
    control: Option<DeniseControl>,
    /// Extent in HIMETRIC units, which is what OLE asks for and reports.
    extent: SIZE,
    /// Where the container put us, in its client coordinates.
    position: RECT,
    /// Whether persisted state has been initialised, for `IPersistStreamInit`.
    initialised: bool,
    /// The sink from `IViewObject::SetAdvise`, told when the picture changes.
    ///
    /// This is how a form editor learns that a design-time property was assigned:
    /// there is no window to invalidate, so the only way its drawing ever
    /// refreshes is if the control says so.
    view_sink: Option<IAdviseSink>,
    /// The aspects that sink asked about, echoed back in the notification.
    view_aspects: u32,
    /// The `ADVF_` flags it registered with. Only `ONLYONCE` changes anything.
    view_advf: u32,
    /// The description handed to `IDispatch::GetTypeInfo`, loaded on first use.
    pub(crate) type_info: Option<ITypeInfo>,
}

impl Default for DenisePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DenisePanel {
    /// A control with no site and no window, which is what the class factory
    /// hands back. Everything else happens when the container calls in.
    pub fn new() -> Self {
        Self {
            state: RefCell::new(PanelState {
                site: None,
                control: None,
                // 200x120 pixels at 96 DPI: a reasonable footprint for a control
                // dropped on a form, and the container overrides it anyway.
                extent: SIZE {
                    cx: pixels_to_himetric(200),
                    cy: pixels_to_himetric(120),
                },
                position: RECT::default(),
                initialised: false,
                view_sink: None,
                view_aspects: 0,
                view_advf: 0,
                type_info: None,
            }),
            model: Model::new(),
        }
    }
}

impl DenisePanel_Impl {
    /// Creates the child window inside the container's, if it does not exist.
    ///
    /// The container's `HWND` comes from its `IOleInPlaceSite`, which is the only
    /// way an in-process control learns where it lives.
    fn activate(&self) -> windows_core::Result<()> {
        if self.state.borrow().control.is_some() {
            return Ok(());
        }
        let site = self
            .state
            .borrow()
            .site
            .clone()
            .ok_or_else(|| windows_core::Error::from(E_FAIL))?;
        let in_place: IOleInPlaceSite = site.cast()?;
        // SAFETY: `in_place` is the container's own object; these are its
        // documented calls and take no arguments needing validation.
        let parent: HWND = unsafe { in_place.GetWindow() }?;
        // SAFETY: telling the container we are activating, which the protocol
        // requires before putting a window in its client area.
        unsafe { in_place.OnInPlaceActivate() }?;

        // A container that activates before calling `SetObjectRects` leaves the
        // position empty, and a 1x1 control is indistinguishable from a broken
        // one. The extent is what it told us in `SetExtent`, in HIMETRIC, so fall
        // back to that rather than to nothing.
        let (position, extent) = {
            let state = self.state.borrow();
            (state.position, state.extent)
        };
        let width = position.right - position.left;
        let height = position.bottom - position.top;
        let bounds = if width > 0 && height > 0 {
            Rect::new(position.left, position.top, width, height)
        } else {
            Rect::new(
                position.left,
                position.top,
                himetric_to_pixels(extent.cx).max(1),
                himetric_to_pixels(extent.cy).max(1),
            )
        };
        let size = Size::new(bounds.width as u32, bounds.height as u32);
        let tree = Tree::new(size, self.model.clone());
        let control = DeniseControl::new(parent, bounds, 1.0, Box::new(tree))
            .map_err(|_| windows_core::Error::from(E_FAIL))?;

        self.state.borrow_mut().control = Some(control);
        control.update();
        Ok(())
    }

    /// Destroys the child window, if there is one.
    fn deactivate(&self) {
        let control = self.state.borrow_mut().control.take();
        if let Some(control) = control {
            // SAFETY: the window was created by `activate` and is destroyed once.
            unsafe {
                let _ = DestroyWindow(control.hwnd());
            }
        }
    }

    /// Pushes a property change into the live tree.
    ///
    /// Called from every property put, which arrives from two places that look
    /// identical from here: a script, and an event handler the tree itself is in
    /// the middle of calling. The second must not reach the tree.
    /// [`DeniseControl::update`] holds the control's own `RefCell` across the
    /// whole delegate call, so running it again from inside would borrow it twice
    /// and panic out through a COM method into the host. The delegate holds the
    /// flag for its entire pass — including while an event is being raised — and
    /// applies whatever a handler left behind before it returns.
    pub(crate) fn sync(&self) {
        if self.model.borrow().inside {
            return;
        }
        // Copied out before the call: `update` runs the tree, which can raise an
        // event, whose handler can land straight back in this object.
        let control = self.state.borrow().control;
        match control {
            // A panic in the tree must not unwind into a host's script engine,
            // which is what a `catch_unwind` at this boundary buys. The window
            // procedure has the same wrapper for input; this is the other door in.
            Some(control) => {
                let _ = catch_unwind(AssertUnwindSafe(|| control.update()));
            }
            // No window, so nothing to repaint — but somebody may be drawing the
            // control themselves through `IViewObject::Draw`, and a picture is
            // only as current as the last time its owner was told to redraw it.
            None => self.notify_view(),
        }
    }

    /// Tells the view sink, if there is one, that what it drew is now stale.
    fn notify_view(&self) {
        let (sink, aspects, once) = {
            let state = self.state.borrow();
            let once = state.view_advf & ADVF_ONLYONCE.0 as u32 != 0;
            (state.view_sink.clone(), state.view_aspects, once)
        };
        let Some(sink) = sink else {
            return;
        };
        // Dropped before the call rather than after: `OnViewChange` is entitled
        // to draw, and drawing lands back in this object.
        if once {
            self.state.borrow_mut().view_sink = None;
        }
        // SAFETY: `sink` is the container's object, kept alive by the clone.
        // `-1` is every index, which for a control with one view is the only one.
        unsafe { sink.OnViewChange(aspects, -1) };
    }

    /// Draws the control as it currently stands onto somebody else's device
    /// context.
    ///
    /// The whole point is that this works with no window and no site: a form
    /// editor asks for the picture before the control is ever activated, and a
    /// control that cannot answer is a blank rectangle on the form. So the tree
    /// is built from the model, painted once into a surface of its own, and
    /// blitted — none of which touches the live control, if there even is one.
    fn render(&self, target: HDC, bounds: *const RECTL) -> windows_core::Result<()> {
        if bounds.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the container promises a readable RECTL.
        let bounds = unsafe { *bounds };
        let Some(plan) = view::plan(bounds.left, bounds.top, bounds.right, bounds.bottom) else {
            // Nothing asked for, so nothing drawn. Not a failure: see `view`.
            return Ok(());
        };

        let size = Size::new(plan.source_width, plan.source_height);
        let mut surface =
            DibSurface::new(size, 1.0).map_err(|_| windows_core::Error::from(E_FAIL))?;
        let (mut ui, _nodes) = build(size, &self.model);
        // No input, no elapsed time and nothing focused, so this is the control
        // at rest: no hover, no pressed button and no caret. Which is what a
        // design-time picture should be.
        ui.tick(0);
        ui.invalidate_all();
        {
            let mut frame = surface
                .acquire()
                .map_err(|_| windows_core::Error::from(E_FAIL))?;
            ui.paint(&mut frame);
        }

        // SAFETY: `target` is the container's device context, live for the call;
        // the source DC holds the DIB just painted and the source rectangle is
        // the whole of it.
        //
        // The device context belongs to the container, so its state is put back
        // afterwards. Changing a stretch mode and leaving it changed is the kind
        // of thing that makes somebody else's later drawing wrong for no reason
        // they can trace. `HALFTONE` is documented as requiring the brush origin
        // to be set after it, which is the only reason that call is here — and
        // neither is touched at all unless the blit actually scales.
        let drawn = unsafe {
            let saved = SaveDC(target);
            if plan.stretches() {
                SetStretchBltMode(target, HALFTONE);
                let _ = SetBrushOrgEx(target, 0, 0, None);
            }
            let drawn = StretchBlt(
                target,
                plan.x,
                plan.y,
                plan.width,
                plan.height,
                Some(surface.dc()),
                0,
                0,
                size.width as i32,
                size.height as i32,
                SRCCOPY,
            );
            if saved != 0 {
                let _ = RestoreDC(target, saved);
            }
            drawn
        };
        if drawn.as_bool() {
            Ok(())
        } else {
            Err(E_FAIL.into())
        }
    }
}

// ------------------------------------------------------------------ IOleObject

impl IOleObject_Impl for DenisePanel_Impl {
    fn SetClientSite(&self, site: Ref<'_, IOleClientSite>) -> windows_core::Result<()> {
        self.state.borrow_mut().site = site.cloned();
        Ok(())
    }

    fn GetClientSite(&self) -> windows_core::Result<IOleClientSite> {
        self.state
            .borrow()
            .site
            .clone()
            .ok_or_else(|| windows_core::Error::from(E_FAIL))
    }

    fn SetHostNames(
        &self,
        _container: &windows_core::PCWSTR,
        _object: &windows_core::PCWSTR,
    ) -> windows_core::Result<()> {
        // The names are for a title bar this control does not have.
        Ok(())
    }

    fn Close(&self, _save: &OLECLOSE) -> windows_core::Result<()> {
        self.deactivate();
        {
            let mut state = self.state.borrow_mut();
            state.site = None;
            // The same cycle as the event sinks below, one interface along.
            state.view_sink = None;
        }
        // A sink holds the control and the control holds the sink. A container
        // that unadvises has already broken that cycle; one that forgot has just
        // said it is finished, and this is the last chance to break it for them.
        self.model.borrow_mut().clear_sinks();
        Ok(())
    }

    fn SetMoniker(
        &self,
        _which: &OLEWHICHMK,
        _moniker: Ref<'_, IMoniker>,
    ) -> windows_core::Result<()> {
        // Linking, which this control is registered as not supporting —
        // OLEMISC_CANTLINKINSIDE.
        Err(E_FAIL.into())
    }

    fn GetMoniker(
        &self,
        _assign: &OLEGETMONIKER,
        _which: &OLEWHICHMK,
    ) -> windows_core::Result<IMoniker> {
        Err(E_FAIL.into())
    }

    fn InitFromData(
        &self,
        _data: Ref<'_, IDataObject>,
        _creation: BOOL,
        _reserved: u32,
    ) -> windows_core::Result<()> {
        Err(E_FAIL.into())
    }

    fn GetClipboardData(&self, _reserved: u32) -> windows_core::Result<IDataObject> {
        Err(E_FAIL.into())
    }

    fn DoVerb(
        &self,
        verb: i32,
        _message: *const windows::Win32::UI::WindowsAndMessaging::MSG,
        _site: Ref<'_, IOleClientSite>,
        _index: i32,
        _parent: HWND,
        position: *const RECT,
    ) -> windows_core::Result<()> {
        if !position.is_null() {
            // SAFETY: the container promises a readable RECT when it passes one.
            self.state.borrow_mut().position = unsafe { *position };
        }
        // SHOW, INPLACEACTIVATE and UIACTIVATE all mean the same thing for an
        // inside-out control: put a live window on screen. Treating them
        // differently is how a control ends up visible but inert.
        if verb == OLEIVERB_SHOW.0
            || verb == OLEIVERB_INPLACEACTIVATE.0
            || verb == OLEIVERB_UIACTIVATE.0
        {
            self.activate()
        } else if verb == OLEIVERB_HIDE.0 {
            self.deactivate();
            Ok(())
        } else {
            Err(E_FAIL.into())
        }
    }

    fn EnumVerbs(&self) -> windows_core::Result<IEnumOLEVERB> {
        // The container falls back to the registry's verb list, which for a
        // control with only "show" is the right amount of ceremony.
        Err(E_FAIL.into())
    }

    fn Update(&self) -> windows_core::Result<()> {
        // Through `sync` rather than straight to the control: a container is
        // entitled to call this from inside an event handler, and that is the one
        // moment the tree must not be run again.
        self.sync();
        Ok(())
    }

    fn IsUpToDate(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn GetUserClassID(&self) -> windows_core::Result<GUID> {
        Ok(CLSID_DENISE_PANEL)
    }

    fn GetUserType(&self, _form: &USERCLASSTYPE) -> windows_core::Result<windows_core::PWSTR> {
        // The caller frees this with `CoTaskMemFree`, so it has to come from
        // `CoTaskMemAlloc` and not from Rust's allocator.
        let wide: Vec<u16> = crate::registry::FRIENDLY_NAME
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let bytes = core::mem::size_of_val(wide.as_slice());
        // SAFETY: allocating `bytes` and then writing exactly that many.
        let buffer = unsafe { CoTaskMemAlloc(bytes) } as *mut u16;
        if buffer.is_null() {
            return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
        }
        // SAFETY: `buffer` is a fresh allocation of `bytes`, and `wide` holds
        // exactly that many bytes and cannot overlap it.
        unsafe { core::ptr::copy_nonoverlapping(wide.as_ptr(), buffer, wide.len()) };
        Ok(windows_core::PWSTR(buffer))
    }

    fn SetExtent(&self, _aspect: DVASPECT, size: *const SIZE) -> windows_core::Result<()> {
        if size.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the container promises a readable SIZE.
        self.state.borrow_mut().extent = unsafe { *size };
        Ok(())
    }

    fn GetExtent(&self, _aspect: DVASPECT) -> windows_core::Result<SIZE> {
        Ok(self.state.borrow().extent)
    }

    fn Advise(&self, _sink: Ref<'_, IAdviseSink>) -> windows_core::Result<u32> {
        // No advisory connections: nothing here changes behind the container's
        // back, so there is nothing to notify about.
        Ok(0)
    }

    fn Unadvise(&self, _token: u32) -> windows_core::Result<()> {
        Ok(())
    }

    fn EnumAdvise(&self) -> windows_core::Result<IEnumSTATDATA> {
        Err(E_FAIL.into())
    }

    fn GetMiscStatus(&self, _aspect: DVASPECT) -> windows_core::Result<OLEMISC> {
        // The same flags the registry carries. A container may ask either way,
        // and the two disagreeing is a control that behaves differently
        // depending on which one it happened to read.
        Ok(OLEMISC(MISC_STATUS as i32))
    }

    fn SetColorScheme(
        &self,
        _palette: *const windows::Win32::Graphics::Gdi::LOGPALETTE,
    ) -> windows_core::Result<()> {
        Ok(())
    }
}

// ------------------------------------------------------------------ IOleWindow

impl IOleWindow_Impl for DenisePanel_Impl {
    fn GetWindow(&self) -> windows_core::Result<HWND> {
        self.state
            .borrow()
            .control
            .map(|c| c.hwnd())
            .ok_or_else(|| windows_core::Error::from(E_FAIL))
    }

    fn ContextSensitiveHelp(&self, _entering: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

// ----------------------------------------------------------- IOleInPlaceObject

impl IOleInPlaceObject_Impl for DenisePanel_Impl {
    fn InPlaceDeactivate(&self) -> windows_core::Result<()> {
        self.deactivate();
        Ok(())
    }

    fn UIDeactivate(&self) -> windows_core::Result<()> {
        // Nothing to hand back: this control merges no menus, no toolbars and no
        // accelerators into the container's.
        Ok(())
    }

    fn SetObjectRects(
        &self,
        position: *const RECT,
        _clip: *const RECT,
    ) -> windows_core::Result<()> {
        if position.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the container promises a readable RECT.
        let position = unsafe { *position };
        self.state.borrow_mut().position = position;

        let control = self.state.borrow().control;
        if let Some(control) = control {
            // SAFETY: the control's window is live while `control` is `Some`.
            unsafe {
                let _ = SetWindowPos(
                    control.hwnd(),
                    None,
                    position.left,
                    position.top,
                    position.right - position.left,
                    position.bottom - position.top,
                    SWP_NOZORDER,
                );
            }
        }
        // The clip rectangle is the container's business: a child window is
        // already clipped to its parent, which is what makes it a child window.
        Ok(())
    }

    fn ReactivateAndUndo(&self) -> windows_core::Result<()> {
        Err(E_FAIL.into())
    }
}

// --------------------------------------------------------------- IOleControl

impl IOleControl_Impl for DenisePanel_Impl {
    fn GetControlInfo(
        &self,
        _info: *mut windows::Win32::System::Ole::CONTROLINFO,
    ) -> windows_core::Result<()> {
        // No mnemonics to register with the container.
        Err(E_FAIL.into())
    }

    fn OnMnemonic(
        &self,
        _message: *const windows::Win32::UI::WindowsAndMessaging::MSG,
    ) -> windows_core::Result<()> {
        Err(E_FAIL.into())
    }

    fn OnAmbientPropertyChange(&self, _dispid: i32) -> windows_core::Result<()> {
        Ok(())
    }

    fn FreezeEvents(&self, _freeze: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

// ------------------------------------------------- IViewObject / IViewObject2

/// Drawing without a window.
///
/// Everything else in this file assumes a live control: the container sites it,
/// activates it, and Windows delivers input to a real `HWND`. A form editor does
/// none of that. It drops the control on a design surface, sets properties on it,
/// and asks for a picture — and a control with no answer is the blank rectangle
/// that this interface exists to avoid.
///
/// The container's device context is the whole interface, which also makes this
/// the path a print preview and a copy-to-metafile take.
impl IViewObject_Impl for DenisePanel_Impl {
    fn Draw(
        &self,
        aspect: DVASPECT,
        _index: i32,
        _aspect_info: *mut core::ffi::c_void,
        _target_device: *const DVTARGETDEVICE,
        _target_dc: HDC,
        draw_dc: HDC,
        bounds: *const RECTL,
        _window_bounds: *const RECTL,
        _continue_fn: isize,
        _continue_arg: usize,
    ) -> windows_core::Result<()> {
        // CONTENT is the control itself. THUMBNAIL, ICON and DOCPRINT are the
        // other three, and answering one of them with the content is worse than
        // declining: a container asked for a 32x32 icon and would scale a panel
        // into it rather than fall back to the class's registered default.
        if aspect != DVASPECT_CONTENT {
            return Err(DV_E_DVASPECT.into());
        }
        // A panic must not unwind out of a COM method into a form editor, and
        // this one runs the tree — the same boundary `sync` guards.
        catch_unwind(AssertUnwindSafe(|| self.render(draw_dc, bounds)))
            .unwrap_or_else(|_| Err(E_FAIL.into()))
    }

    fn GetColorSet(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _aspect_info: *mut core::ffi::c_void,
        _target_device: *const DVTARGETDEVICE,
        _target_dc: HDC,
        _colours: *mut *mut LOGPALETTE,
    ) -> windows_core::Result<()> {
        // The surface is 32-bit RGB, so there is no palette to negotiate. A
        // container on a palettised display would want one; there are none left.
        Err(E_NOTIMPL.into())
    }

    fn Freeze(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _aspect_info: *mut core::ffi::c_void,
        _token: *mut u32,
    ) -> windows_core::Result<()> {
        // Freezing pins a view so a container can draw it repeatedly and know it
        // has not changed underneath. Every `Draw` here renders from the model on
        // the spot, so there is no cached view to pin and nothing honest to
        // promise.
        Err(E_NOTIMPL.into())
    }

    fn Unfreeze(&self, _token: u32) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetAdvise(
        &self,
        aspects: DVASPECT,
        advf: u32,
        sink: Ref<'_, IAdviseSink>,
    ) -> windows_core::Result<()> {
        // One sink, replaced rather than added to: `SetAdvise` is documented as
        // supporting exactly one, which is what distinguishes it from the
        // `IOleObject::Advise` list.
        let mut state = self.state.borrow_mut();
        state.view_sink = sink.cloned();
        state.view_aspects = aspects.0;
        state.view_advf = advf;
        Ok(())
    }

    fn GetAdvise(
        &self,
        aspects: *mut u32,
        advf: *mut u32,
        sink: OutRef<'_, IAdviseSink>,
    ) -> windows_core::Result<()> {
        // Each of the three is optional, and a container that only wants one
        // passes null for the others. Writing through those is the crash.
        let state = self.state.borrow();
        if !aspects.is_null() {
            // SAFETY: non-null, and the caller owns a `u32` behind it.
            unsafe { *aspects = state.view_aspects };
        }
        if !advf.is_null() {
            // SAFETY: as above.
            unsafe { *advf = state.view_advf };
        }
        if !sink.is_null() {
            sink.write(state.view_sink.clone())?;
        }
        Ok(())
    }
}

impl IViewObject2_Impl for DenisePanel_Impl {
    fn GetExtent(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _target_device: *const DVTARGETDEVICE,
    ) -> windows_core::Result<SIZE> {
        // Deliberately the same value `IOleObject::GetExtent` reports. The whole
        // reason `IViewObject2` exists is to save a container a `QueryInterface`
        // for that answer, so the two disagreeing would be a control that changes
        // size depending on which interface was asked.
        Ok(self.state.borrow().extent)
    }
}

// ---------------------------------------------------- IPersist / StreamInit

impl IPersist_Impl for DenisePanel_Impl {
    fn GetClassID(&self) -> windows_core::Result<GUID> {
        Ok(CLSID_DENISE_PANEL)
    }
}

impl IPersistStreamInit_Impl for DenisePanel_Impl {
    fn IsDirty(&self) -> HRESULT {
        // Nothing is persisted yet, so nothing is ever unsaved. `S_FALSE` is
        // "clean"; returning `S_OK` would make a container prompt to save a
        // control that has no state.
        windows::Win32::Foundation::S_FALSE
    }

    fn Load(&self, _stream: Ref<'_, IStream>) -> windows_core::Result<()> {
        // No properties yet, so a saved form has nothing for this to read. It
        // must still succeed: VB6 calls it on every load and treats a failure as
        // a broken control.
        self.state.borrow_mut().initialised = true;
        Ok(())
    }

    fn Save(&self, _stream: Ref<'_, IStream>, _clear_dirty: BOOL) -> windows_core::Result<()> {
        Ok(())
    }

    fn GetSizeMax(&self) -> windows_core::Result<u64> {
        Ok(0)
    }

    fn InitNew(&self) -> windows_core::Result<()> {
        self.state.borrow_mut().initialised = true;
        Ok(())
    }
}

/// The nodes a script can reach, once they exist.
///
/// `Option` because [`Ui::add`] can refuse, and a control whose tree failed to
/// build should draw nothing rather than panic inside a host's message loop.
#[derive(Clone, Copy, Default)]
struct Nodes {
    label: Option<NodeId>,
    input: Option<NodeId>,
    button: Option<NodeId>,
}

/// What a container sees, and what a script drives: a heading, a field and a
/// button.
struct Tree {
    ui: Ui<u32>,
    nodes: Nodes,
    model: Shared,
    started: Instant,
}

impl Tree {
    fn new(size: Size, model: Shared) -> Self {
        let (ui, nodes) = build(size, &model);
        Self {
            ui,
            nodes,
            model,
            started: Instant::now(),
        }
    }

    /// Writes anything a script assigned into the widgets.
    ///
    /// Runs with `inside` already set, so a property put from an event handler
    /// cannot reach back in behind it.
    fn apply(&mut self) {
        let (text, caption, enabled, dirty, refresh) = {
            let mut model = self.model.borrow_mut();
            let pending = (
                model.text.clone(),
                model.caption.clone(),
                model.enabled,
                model.dirty,
                model.refresh,
            );
            model.dirty = false;
            model.refresh = false;
            pending
        };

        if refresh {
            self.ui.invalidate_all();
        }
        if !dirty {
            return;
        }

        if let Some(label) = self
            .nodes
            .label
            .and_then(|id| self.ui.widget_mut::<Label>(id))
        {
            label.set_text(caption);
        }

        // Only when it differs. `set_text` puts the caret at the end, so writing
        // the same string on every pass would move the caret out from under
        // anyone editing in the middle of a word.
        let stale = self.nodes.input.filter(|id| {
            self.ui
                .widget::<TextInput<u32>>(*id)
                .is_some_and(|field| field.text() != text)
        });
        if let Some(field) = stale.and_then(|id| self.ui.widget_mut::<TextInput<u32>>(id)) {
            field.set_text(text);
        }

        for id in [self.nodes.input, self.nodes.button].into_iter().flatten() {
            self.ui.set_enabled(id, enabled);
        }
    }

    /// Mirrors the field back into the model and decides what to raise.
    fn collect(&mut self) -> Vec<i32> {
        // `any` short-circuits, and the queue is still emptied: a `Drain` removes
        // its whole range when it is dropped, whether or not it was consumed. The
        // queue growing without bound is the failure this has to avoid.
        let clicked = self
            .ui
            .drain_messages()
            .any(|message| message == MSG_ACTIVATED);

        let current = self
            .nodes
            .input
            .and_then(|id| self.ui.widget::<TextInput<u32>>(id))
            .map(|field| field.text().to_string());

        match current {
            Some(current) => {
                let raised = dispatch::events_raised(&self.model.borrow().text, &current, clicked);
                self.model.borrow_mut().text = current;
                raised
            }
            // No field to read, so nothing can have changed in one.
            None => dispatch::events_raised("", "", clicked),
        }
    }

    /// Calls every advised sink, with no borrows held.
    fn raise(&self, dispid: i32) {
        let sinks = self.model.borrow().sinks();
        let params = DISPPARAMS::default();
        for sink in sinks {
            // SAFETY: `sink` is the host's object, kept alive by the clone;
            // `params` is a live local describing no arguments. Neither event
            // takes any or returns anything, so there is nothing to marshal.
            //
            // The result is dropped on purpose: a handler that fails is the
            // host's problem, and it is not a reason for the panel to stop
            // drawing.
            unsafe {
                let _ = sink.Invoke(
                    dispid,
                    &GUID::zeroed(),
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                );
            }
        }
    }
}

/// Marks the tree as running for as long as it exists.
///
/// A guard rather than two assignments, because the flag has to come down even
/// when the pass ends badly. The window procedure turns a panic into
/// `DefWindowProc` and carries on, and a flag left standing after one would
/// silently stop every later property put from ever reaching the tree — a control
/// that quietly stops responding to script, with nothing in the logs.
struct Running(Shared);

impl Running {
    /// Holds an `Rc` rather than a borrow so the caller keeps its `&mut self`.
    fn enter(model: &Shared) -> Self {
        model.borrow_mut().inside = true;
        Self(model.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.borrow_mut().inside = false;
    }
}

/// Builds the tree from whatever the model currently holds.
///
/// Reads the model rather than hard-coding the strings, so a resize — which
/// rebuilds everything — does not throw away what a script assigned.
fn build(size: Size, model: &Shared) -> (Ui<u32>, Nodes) {
    let (text, caption, enabled) = {
        let model = model.borrow();
        (model.text.clone(), model.caption.clone(), model.enabled)
    };

    let mut ui: Ui<u32> = Ui::new(size, Theme::DARK);
    // The container's window system draws a pointer already.
    ui.show_cursor(false);
    let root = ui.root();
    let width = size.width as i32;
    let height = size.height as i32;
    let mut nodes = Nodes::default();

    if let Some(card) = ui.add(
        root,
        Panel::default(),
        Rect::new(8, 8, (width - 16).max(1), (height - 16).max(1)),
    ) {
        nodes.label = ui.add(
            card,
            Label::new(caption),
            Rect::new(12, 10, (width - 40).max(1), 22),
        );
        let mut field = TextInput::<u32>::new().with_placeholder("Tekst");
        field.set_text(text);
        nodes.input = ui.add(card, field, Rect::new(12, 38, (width - 40).max(1), 32));
        nodes.button = ui.add(
            card,
            Button::new("OK", MSG_ACTIVATED).with_role(Role::Primary),
            Rect::new(12, 78, 96, 30),
        );
    }

    for id in [nodes.input, nodes.button].into_iter().flatten() {
        ui.set_enabled(id, enabled);
    }

    (ui, nodes)
}

impl ControlDelegate for Tree {
    fn update(&mut self, surface: &mut DibSurface, events: &[InputEvent], damage: &mut Vec<Rect>) {
        if surface.size() != self.ui.size() {
            let (ui, nodes) = build(surface.size(), &self.model);
            self.ui = ui;
            self.nodes = nodes;
            self.ui.invalidate_all();
        }

        // Held for the whole pass, raising included. `DeniseControl::update` has
        // the control's `RefCell` borrowed around this call, so anything a handler
        // does that would run the tree again has to be turned away here rather
        // than panic there.
        let _running = Running::enter(&self.model);

        self.apply();
        self.ui.handle(events);
        self.ui.tick(self.started.elapsed().as_millis() as u64);

        for dispid in self.collect() {
            self.raise(dispid);
        }

        // A handler is allowed to assign to a property, and could not reach the
        // tree while it ran. One further pass, deliberately not a loop: a handler
        // that assigns on every event would otherwise never hand control back.
        self.apply();

        // Painted last, so a handler that set `Caption` sees it drawn in the same
        // frame as the click that called it.
        if !self.ui.needs_paint() {
            return;
        }
        if let Ok(mut frame) = surface.acquire() {
            self.ui.paint(&mut frame);
            drop(frame);
            damage.extend_from_slice(self.ui.damage());
            self.ui.presented();
        }
    }

    fn next_wake_ms(&self) -> Option<u64> {
        self.ui.next_wake_ms()
    }
}

/// Referenced so the flags cannot drift from the registry's copy.
const _: u32 = MISC_STATUS;
