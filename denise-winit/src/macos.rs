//! The macOS present path, which is not softbuffer's.
//!
//! Every other platform softbuffer supports is damage-aware: win32 `BitBlt`s the
//! rectangles it is handed into a persistent DIB section, x11, wayland and kms do
//! the equivalent, and all of them report a real buffer age so only what changed
//! is copied. The CoreGraphics backend is the exception on all three counts — a
//! freshly allocated and zeroed buffer per `buffer_mut`, an age of 0, and a
//! `present_with_damage` that discards its damage — and the cost is the whole
//! surface, three times, per present.
//!
//! It shows. One spinner on a 2560×1600 window cost 48.8% of a core before any of
//! this, and 22% after the loop stopped presenting frames nobody asked for.
//!
//! So on macOS the pixels live in an `IOSurface` and the window's layer is handed
//! that surface as its contents, which is what [`denise-macos`] already does for
//! an embedded view. CoreAnimation reads the buffer where it lies. Nothing is
//! copied, and the cost stops scaling with the size of the window.
//!
//! [`denise-macos`]: https://crates.io/crates/denise-macos

use std::sync::Arc;

use denise::{Frame, InputEvent, InputSource, PixelFormat, Rect, Size, Surface, SurfaceError};
use denise_macos::ViewSurface;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSView;
use objc2_core_foundation::CGFloat;
use objc2_io_surface::IOSurfaceRef;
use objc2_quartz_core::{CALayer, CATransaction};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::Error;

/// A [`denise::Surface`] over a winit window's `CALayer`.
pub struct MacSurface {
    window: Arc<Window>,
    layer: Retained<CALayer>,
    surface: ViewSurface,
    events: Vec<InputEvent>,
    /// Frames presented, for the read-back diagnostic only.
    frames: u64,
}

impl MacSurface {
    /// Makes the window's view layer-backed and binds a surface to its layer.
    pub fn new(window: Arc<Window>) -> Result<Self, Error> {
        let size = window.inner_size();
        let size = Size::new(size.width, size.height);
        let scale = window.scale_factor() as f32;

        let layer = layer_of(&window)?;
        let surface = ViewSurface::new(size, scale).map_err(|e| Error::Present(e.to_string()))?;

        let this = Self {
            window,
            layer,
            surface,
            events: Vec::new(),
            frames: 0,
        };
        this.attach();
        Ok(this)
    }

    /// The window this surface draws to.
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// Reallocates for a new size or DPI. Discards all damage history.
    pub fn resize(&mut self, size: Size, scale_factor: f32) {
        match self.surface.resize(size, scale_factor) {
            // Unchanged, so the layer still points at the right buffer.
            Ok(false) => {}
            Ok(true) => self.attach(),
            // A surface that cannot be reallocated leaves the old one in place,
            // which is wrong by exactly one frame and visible for none: the
            // resize event that follows will try again.
            Err(_) => {}
        }
    }

    /// Queues an event for the next [`InputSource::poll`].
    pub(crate) fn push_event(&mut self, event: InputEvent) {
        self.events.push(event);
    }

    /// Points the layer at the current buffer.
    ///
    /// Called once at construction and again whenever the surface is
    /// reallocated, which is the only time the pointer changes.
    fn attach(&self) {
        self.layer
            .setContentsScale(CGFloat::from(self.surface.scale_factor()));
        self.set_contents();
    }

    /// Hands the layer the `IOSurface`, which is also how it is told to look at
    /// it again: assigning `contents` is what makes CoreAnimation pick up what
    /// has been written into the buffer since the last commit.
    ///
    /// The transaction around it is not ceremony, and both halves are load
    /// bearing:
    ///
    /// - **`setDisableActions`.** Changing `contents` is an animatable property,
    ///   so CoreAnimation's default action cross-fades from the old contents to
    ///   the new over a quarter of a second. At sixty frames a second that is
    ///   fifteen fades running at once over the same buffer, which does not look
    ///   like animation at all — it looks like two frames a second of smeared
    ///   nonsense.
    /// - **`flush`.** An implicit transaction is committed by the run loop's
    ///   observer, and this loop spends its time blocked in `WaitUntil` rather
    ///   than in AppKit's own cycle. Without an explicit flush the frame sits
    ///   uncommitted until something else wakes the run loop, which is what
    ///   turns a redraw into "the window updates when you wiggle the mouse".
    fn set_contents(&self) {
        let surface: &IOSurfaceRef = self.surface.io_surface();
        // SAFETY: `IOSurfaceRef` is toll-free bridged to the `IOSurface` class,
        // which is what lets it be a layer's contents at all. It stays alive as
        // long as `self.surface`, and the layer retains what it is given.
        let contents: &AnyObject =
            unsafe { &*(surface as *const IOSurfaceRef).cast::<AnyObject>() };

        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // SAFETY: the event loop runs on the main thread, so this does too.
        unsafe { self.layer.setContents(Some(contents)) };
        CATransaction::commit();
        CATransaction::flush();
    }
}

/// The `CALayer` behind a winit window, made if the view has not got one.
fn layer_of(window: &Window) -> Result<Retained<CALayer>, Error> {
    let handle = window
        .window_handle()
        .map_err(|e| Error::Present(e.to_string()))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(Error::Present("not an AppKit window".into()));
    };

    // SAFETY: winit promises the handle points at a live `NSView` for as long as
    // the window is alive, and the window outlives this surface.
    let view: &NSView = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    // A view that is already layer-backed keeps the layer it has.
    view.setWantsLayer(true);
    view.layer()
        .ok_or_else(|| Error::Present("the view has no layer".into()))
}

impl MacSurface {
    /// Reads back what the layer is actually holding, and whether anything else
    /// has taken the buffer.
    ///
    /// Set `DENISE_MACOS_DEBUG=1` to print it for the first few frames. This
    /// exists because "the window is frozen" and "the window is drawing very
    /// efficiently" produce identical CPU figures, and every other way of
    /// telling them apart involves a human looking at a screen.
    ///
    /// What each line means:
    ///
    /// - `contents: ours` — the layer is holding the surface we assigned. If it
    ///   says anything else, AppKit has replaced it, which is what a view that
    ///   draws itself does to a layer somebody else was using.
    /// - `in_use: true` — something outside this process has the surface open,
    ///   which for a layer's contents means the compositor took it. `false` on
    ///   every frame means nothing is reading what we draw.
    /// - `seed` — IOSurface's own change counter. It moves when the buffer is
    ///   written, so a stuck seed would mean we are not drawing at all.
    fn read_back(&self, frame: u64) {
        // The first few, then one a second: the question is not only what the
        // layer holds but whether frames keep arriving at all.
        if std::env::var_os("DENISE_MACOS_DEBUG").is_none()
            || (frame > 3 && !frame.is_multiple_of(60))
        {
            return;
        }
        let ours: *const AnyObject = (self.surface.io_surface() as *const IOSurfaceRef).cast();

        // SAFETY: main thread, and `contents` is a plain property read.
        let contents = unsafe { self.layer.contents() };
        let held = match contents.as_deref() {
            None => "nil".to_owned(),
            // The pointer, so an alternating pair is visible as two values
            // rather than as one word that never changes.
            Some(object) if core::ptr::eq(object as *const AnyObject, ours) => {
                format!("ours @ {ours:p}")
            }
            Some(object) => format!("{:?} (not ours)", object.class()),
        };

        // And whether the view still hands out the layer we are talking to.
        let same_layer = layer_of(&self.window)
            .map(|current| core::ptr::eq(&*current, &*self.layer))
            .unwrap_or(false);

        eprintln!(
            "denise-macos frame {frame}: contents: {held}, in_use: {}, use_count: {}, seed: {}, same layer: {same_layer}",
            self.surface.io_surface().is_in_use(),
            self.surface.io_surface().use_count(),
            self.surface.io_surface().seed(),
        );
    }
}

impl Surface for MacSurface {
    fn size(&self) -> Size {
        self.surface.size()
    }

    fn scale_factor(&self) -> f32 {
        self.surface.scale_factor()
    }

    fn format(&self) -> PixelFormat {
        self.surface.format()
    }

    fn acquire(&mut self) -> Result<Frame<'_>, SurfaceError> {
        self.surface.acquire()
    }

    fn present(&mut self, damage: &[Rect]) -> Result<(), SurfaceError> {
        self.surface.present(damage)?;
        // The buffer the compositor is already holding now has new pixels in it.
        // This is the whole of the present: no blit, no upload, no copy.
        self.set_contents();
        self.frames += 1;
        self.read_back(self.frames);
        Ok(())
    }
}

impl InputSource for MacSurface {
    fn poll(&mut self, out: &mut Vec<InputEvent>) {
        out.append(&mut self.events);
    }
}
