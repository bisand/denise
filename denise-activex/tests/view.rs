//! The design-time view, drawn into a device context and then read back.
//!
//! `IViewObject2::Draw` is the one interface here that a person cannot easily
//! check by looking: a form editor calls it, and if the answer is wrong the
//! symptom is a blank rectangle on somebody else's design surface with nothing
//! logged anywhere. So these tests do what the form editor does — a memory DC, a
//! rectangle, and a look at the pixels afterwards — with no window, no site and
//! no registration, which is what lets them run on a CI runner with nothing
//! installed.
//!
//! The geometry that decides *where* the picture goes is tested in `src/view.rs`,
//! on every platform. What is left for here is the part that needs GDI.

#![cfg(windows)]

use std::cell::Cell;
use std::mem::ManuallyDrop;
use std::rc::Rc;

use denise_activex::DenisePanel;
use windows::Win32::Foundation::{DV_E_DVASPECT, RECTL};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::System::Com::{
    ADVF_ONLYONCE, DISPATCH_PROPERTYPUT, DISPPARAMS, DVASPECT_CONTENT, DVASPECT_THUMBNAIL,
    FORMATETC, IAdviseSink, IAdviseSink_Impl, IDispatch, IMoniker, STGMEDIUM,
};
use windows::Win32::System::Ole::{DISPID_PROPERTYPUT, IOleObject, IViewObject, IViewObject2};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR, VariantClear,
};
use windows_core::{BSTR, GUID, Interface, implement};

/// The size a container drops the control at, and the one the control reports.
const WIDTH: i32 = 200;
const HEIGHT: i32 = 120;

/// A colour the panel does not contain, so "this pixel was drawn on" is a
/// question with an answer. Bright magenta, which no theme uses.
const SENTINEL: u32 = 0x00FF_00FF;

fn panel() -> IViewObject2 {
    DenisePanel::new().into()
}

/// The whole point of the interface: a picture from a control that has never
/// been sited, never been activated and has no window.
///
/// Every other path in this crate goes through `DoVerb`, which needs a container
/// with an `HWND` to put a child inside. A form editor does none of that — it
/// drops the control on a design surface and asks what it looks like.
#[test]
fn the_control_draws_itself_with_no_window_and_no_container() {
    let canvas = Canvas::new(WIDTH, HEIGHT);
    draw(&panel(), &canvas, bounds(0, 0, WIDTH, HEIGHT)).expect("Draw");

    let pixels = canvas.pixels();
    let untouched = pixels.iter().filter(|&&p| p == SENTINEL).count();
    assert_eq!(untouched, 0, "the whole rectangle should have been covered");

    // Not one flat colour either: a panel that painted its background and gave up
    // before the heading, the field and the button would pass the test above.
    let distinct = {
        let mut seen = pixels.clone();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    };
    assert!(
        distinct > 4,
        "only {distinct} colours: that is a background, not a panel"
    );
}

/// What makes it a *design-time* view rather than a picture of a default.
///
/// A designer sets `Caption` in a property sheet and expects the drawing to
/// change. There is no tree to update — the control has no window — so the only
/// way this can work is if `Draw` renders from the model each time, which is what
/// this pins.
#[test]
fn the_picture_follows_a_property_a_designer_assigned() {
    let panel = panel();
    let canvas = Canvas::new(WIDTH, HEIGHT);

    draw(&panel, &canvas, bounds(0, 0, WIDTH, HEIGHT)).expect("Draw");
    let before = canvas.pixels();

    let dispatch: IDispatch = panel.cast().expect("IDispatch");
    put_caption(&dispatch, "Sett i designeren");

    draw(&panel, &canvas, bounds(0, 0, WIDTH, HEIGHT)).expect("Draw");
    let after = canvas.pixels();

    assert_ne!(
        before, after,
        "the heading changed and the picture did not: Draw is rendering a cached \
         view rather than the model"
    );
}

/// CONTENT is the only aspect this control has a picture for.
///
/// Answering a request for a 32x32 icon with a scaled-down panel is worse than
/// declining: the container would use it instead of falling back to the class's
/// registered default.
#[test]
fn an_aspect_this_control_has_no_picture_for_is_declined() {
    let canvas = Canvas::new(WIDTH, HEIGHT);
    let rect = bounds(0, 0, WIDTH, HEIGHT);
    // SAFETY: `canvas` owns a live DC and `rect` is a live local.
    let result = unsafe {
        panel().Draw(
            DVASPECT_THUMBNAIL,
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
    assert_eq!(
        result.expect_err("a thumbnail").code(),
        DV_E_DVASPECT,
        "the documented refusal, which is what makes a container fall back"
    );
}

/// A container that asks for nothing gets nothing, and an `S_OK` for it. Failing
/// here would put an error in a form editor's log for a control that did as it
/// was told.
#[test]
fn an_empty_rectangle_leaves_the_context_untouched() {
    let canvas = Canvas::new(WIDTH, HEIGHT);
    draw(&panel(), &canvas, bounds(40, 40, 40, 90)).expect("an empty rectangle is not an error");
    assert!(
        canvas.pixels().iter().all(|&p| p == SENTINEL),
        "something was drawn for a rectangle with no area in it"
    );
}

/// The rectangle is a raw pointer from somebody else's code, and a container that
/// passes null is a crash inside a form editor rather than an error in a log.
#[test]
fn null_bounds_are_refused_rather_than_dereferenced() {
    let canvas = Canvas::new(WIDTH, HEIGHT);
    // SAFETY: passing the null the test is about; `canvas` owns a live DC.
    let result = unsafe {
        panel().Draw(
            DVASPECT_CONTENT,
            -1,
            core::ptr::null_mut(),
            None,
            None,
            canvas.dc,
            None,
            None,
            None,
            0,
        )
    };
    assert!(result.is_err(), "a null rectangle must not be dereferenced");
}

/// `IViewObject2::GetExtent` exists to save a container a `QueryInterface` for
/// the answer `IOleObject::GetExtent` already gives. The two disagreeing would be
/// a control whose size depends on which interface was asked.
#[test]
fn the_extent_agrees_with_the_one_ioleobject_reports() {
    let panel = panel();
    let ole: IOleObject = panel.cast().expect("IOleObject");

    // SAFETY: both objects are live and neither call takes a pointer.
    let (view, object) = unsafe {
        (
            panel
                .GetExtent(DVASPECT_CONTENT, -1, core::ptr::null())
                .expect("view"),
            ole.GetExtent(DVASPECT_CONTENT).expect("object"),
        )
    };
    assert_eq!((view.cx, view.cy), (object.cx, object.cy));

    // And it is the default footprint in HIMETRIC, not zero — a container that
    // reads zero draws a control with no area.
    assert_eq!(
        (view.cx, view.cy),
        (
            denise_activex::himetric::pixels_to_himetric(WIDTH),
            denise_activex::himetric::pixels_to_himetric(HEIGHT)
        )
    );
}

/// `IViewObject2` derives from `IViewObject`, and plenty of containers ask for
/// the older one by name. Implementing only the derived interface and finding
/// that the base is unreachable is a control that draws for some hosts.
#[test]
fn the_base_interface_is_reachable_from_the_derived_one() {
    let base: IViewObject = panel().cast().expect("IViewObject from IViewObject2");
    let derived: IViewObject2 = base.cast().expect("and back again");
    // SAFETY: `derived` is live; this is only here to prove the pointer works.
    let _ = unsafe { derived.GetExtent(DVASPECT_CONTENT, -1, core::ptr::null()) }.expect("extent");
}

/// The other half of a design-time view: telling the container it is stale.
///
/// There is no window to invalidate, so a form editor's drawing only ever
/// refreshes because the control said so. Without this, setting a property in a
/// property sheet changes nothing on screen until something else forces a redraw.
#[test]
fn a_view_sink_is_told_when_a_property_changes_the_picture() {
    let panel = panel();
    let changes = Rc::new(Cell::new(0u32));
    let sink: IAdviseSink = Watcher(changes.clone()).into();

    // SAFETY: `panel` and `sink` are live objects owned by this test.
    unsafe { panel.SetAdvise(DVASPECT_CONTENT, 0, &sink) }.expect("SetAdvise");

    let dispatch: IDispatch = panel.cast().expect("IDispatch");
    put_caption(&dispatch, "Ny tittel");
    assert_eq!(changes.get(), 1, "the sink was not told");

    put_caption(&dispatch, "Enda en");
    assert_eq!(changes.get(), 2, "a standing registration fires every time");

    // And it stops when the container says so.
    // SAFETY: as above; `None` withdraws the registration.
    unsafe { panel.SetAdvise(DVASPECT_CONTENT, 0, None::<&IAdviseSink>) }.expect("SetAdvise(None)");
    put_caption(&dispatch, "Etter avmelding");
    assert_eq!(changes.get(), 2, "a withdrawn sink was still called");
}

/// `ADVF_ONLYONCE` means what it says, and the registration has to be gone before
/// the notification goes out rather than after: a sink is entitled to draw, and
/// drawing lands back in the control.
#[test]
fn a_sink_registered_once_is_told_once() {
    let panel = panel();
    let changes = Rc::new(Cell::new(0u32));
    let sink: IAdviseSink = Watcher(changes.clone()).into();

    // SAFETY: both objects are live.
    unsafe { panel.SetAdvise(DVASPECT_CONTENT, ADVF_ONLYONCE.0 as u32, &sink) }.expect("SetAdvise");

    let dispatch: IDispatch = panel.cast().expect("IDispatch");
    put_caption(&dispatch, "Første");
    put_caption(&dispatch, "Andre");
    assert_eq!(changes.get(), 1, "ONLYONCE means once");
}

/// A container may ask for the registration back, and may want only part of it —
/// the three out-parameters are each optional, and writing through a null one is
/// the crash this checks does not happen.
#[test]
fn the_registration_reads_back_and_tolerates_the_parts_not_asked_for() {
    let panel = panel();
    let sink: IAdviseSink = Watcher(Rc::new(Cell::new(0))).into();
    // SAFETY: both objects are live.
    unsafe { panel.SetAdvise(DVASPECT_CONTENT, ADVF_ONLYONCE.0 as u32, &sink) }.expect("SetAdvise");

    let mut aspects = 0u32;
    let mut advf = 0u32;
    let mut back: Option<IAdviseSink> = None;
    // SAFETY: three live locals.
    unsafe { panel.GetAdvise(Some(&mut aspects), Some(&mut advf), &mut back) }.expect("GetAdvise");
    assert_eq!(aspects, DVASPECT_CONTENT.0);
    assert_eq!(advf, ADVF_ONLYONCE.0 as u32);
    assert_eq!(back, Some(sink), "a different object came back");

    // Nothing wanted at all, which is legal and must not write anywhere.
    // SAFETY: passing the nulls the test is about.
    unsafe { panel.GetAdvise(None, None, core::ptr::null_mut()) }
        .expect("GetAdvise with nothing asked for");
}

// ------------------------------------------------------------------- the helpers

/// A sink that counts what it was told.
#[implement(IAdviseSink)]
struct Watcher(Rc<Cell<u32>>);

impl IAdviseSink_Impl for Watcher_Impl {
    fn OnViewChange(&self, _aspect: u32, _index: i32) {
        self.0.set(self.0.get() + 1);
    }

    fn OnDataChange(&self, _format: *const FORMATETC, _medium: *const STGMEDIUM) {}
    fn OnRename(&self, _moniker: windows_core::Ref<'_, IMoniker>) {}
    fn OnSave(&self) {}
    fn OnClose(&self) {}
}

fn bounds(left: i32, top: i32, right: i32, bottom: i32) -> RECTL {
    RECTL {
        left,
        top,
        right,
        bottom,
    }
}

fn draw(panel: &IViewObject2, canvas: &Canvas, rect: RECTL) -> windows_core::Result<()> {
    // SAFETY: `canvas` owns a live memory DC and `rect` is a live local. The two
    // device pointers and the continuation callback are all optional and unused.
    unsafe {
        panel.Draw(
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
    }
}

/// Assigns `Caption` by name, the way a property sheet does.
fn put_caption(object: &IDispatch, text: &str) {
    let name: Vec<u16> = "Caption"
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let mut dispid = 0i32;
    // SAFETY: `riid` is reserved and must be `IID_NULL`; `name` outlives the call.
    unsafe {
        object.GetIDsOfNames(
            &GUID::zeroed(),
            &windows_core::PCWSTR(name.as_ptr()),
            1,
            0,
            &mut dispid,
        )
    }
    .expect("Caption");

    let mut argument = VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VARENUM(VT_BSTR.0),
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 {
                    bstrVal: ManuallyDrop::new(BSTR::from(text)),
                },
            }),
        },
    };
    let mut named = DISPID_PROPERTYPUT;
    let params = DISPPARAMS {
        rgvarg: &mut argument,
        rgdispidNamedArgs: &mut named,
        cArgs: 1,
        cNamedArgs: 1,
    };
    // SAFETY: `params` borrows two live locals and describes them correctly.
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
    .expect("put Caption");
    // SAFETY: `argument` is ours and holds the only reference to its payload.
    unsafe {
        let _ = VariantClear(&mut argument);
    }
}

/// A memory device context whose pixels can be read back.
///
/// The same shape as `DibSurface`, deliberately built by hand here: a test that
/// draws into the thing under test's own surface type would prove rather less.
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u32,
    count: usize,
}

impl Canvas {
    fn new(width: i32, height: i32) -> Self {
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        // Negative for top-down, so row 0 is the top one. Only the reading below
        // cares, and only because it treats the buffer as one flat run.
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;

        let mut bits = core::ptr::null_mut();
        // SAFETY: `info` describes a 32-bit BI_RGB DIB and `bits` receives the
        // allocation. A null DC is valid for `DIB_RGB_COLORS`.
        let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .expect("a DIB section");
        // SAFETY: a null argument asks for a screen-compatible memory DC.
        let dc = unsafe { CreateCompatibleDC(None) };
        // SAFETY: both handles are live and a bitmap is valid in a memory DC.
        let previous = unsafe { SelectObject(dc, bitmap.into()) };

        let canvas = Self {
            dc,
            bitmap,
            previous,
            pixels: bits.cast(),
            count: (width * height) as usize,
        };
        // A colour the panel does not use, so an undrawn pixel is recognisable.
        // SAFETY: `pixels` is the DIB's own allocation, exactly `count` words.
        unsafe { core::slice::from_raw_parts_mut(canvas.pixels, canvas.count) }.fill(SENTINEL);
        canvas
    }

    /// The pixels, after making GDI finish what it batched.
    ///
    /// `GdiFlush` is the whole reason this is a method rather than a field. GDI
    /// batches drawing calls per thread and a DIB section read without it returns
    /// whatever was there before the blit — intermittently, which is the worst
    /// way for a test to be wrong.
    fn pixels(&self) -> Vec<u32> {
        // SAFETY: no arguments, and it only drains this thread's own batch.
        unsafe {
            let _ = GdiFlush();
        };
        // SAFETY: as in `new`, and nothing else holds the buffer.
        unsafe { core::slice::from_raw_parts(self.pixels, self.count) }.to_vec()
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: every handle was created in `new` and is released once. The
        // previous object goes back first, because a DC still holding our bitmap
        // will not release it.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(self.bitmap.into());
        }
    }
}
