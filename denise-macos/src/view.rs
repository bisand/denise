//! `DeniseView`: an `NSView` subclass a Cocoa application can drop into its own
//! window.
//!
//! The split here is the same one `Ui::render` makes, and for the same reason.
//! [`DeniseView::update`] consumes input, repaints the surface and tells AppKit
//! which rectangles changed; `drawRect:` only blits. Doing both in `drawRect:`
//! would mean the tree could not damage anything, because by then AppKit has
//! already decided what it is going to composite.

use std::cell::RefCell;

use denise::{ElementState, InputEvent, Modifiers, Point, PointerButton, Rect, Size, Surface};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSGraphicsContext, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use objc2_io_surface::IOSurfaceRef;
use objc2_quartz_core::CATransaction;

use crate::Error;
use crate::keymap::key_code;
use crate::surface::ViewSurface;

/// Nominal pixels per wheel notch, for the coarse scroll a mouse produces. A
/// trackpad reports precise deltas already in points and needs no scaling.
const LINE_HEIGHT_PX: f32 = 16.0;

/// What the application implements to put something in the view.
///
/// Deliberately not "here is a `Ui`": a signage application drawing its own scene
/// with `denise-render` has no tree at all, and this backend has
/// no business requiring one.
pub trait ViewDelegate {
    /// Handles `events`, repaints `surface`, and appends what changed to `damage`.
    ///
    /// `damage` arrives empty. Leaving it empty means nothing changed and AppKit
    /// is told nothing, which is what makes an idle panel cost nothing.
    fn update(&mut self, surface: &mut ViewSurface, events: &[InputEvent], damage: &mut Vec<Rect>);

    /// Milliseconds until this delegate wants updating again, or `None` if it is
    /// waiting only on input.
    ///
    /// A blinking caret is the reason this exists. The host turns it into an
    /// `NSTimer` — see the `embed` example.
    fn next_wake_ms(&self) -> Option<u64> {
        None
    }
}

/// The view's mutable state. Reachable through [`DeniseView::state`].
pub struct ViewState {
    /// The pixels, and the geometry that describes them.
    pub surface: ViewSurface,
    delegate: Box<dyn ViewDelegate>,
    events: Vec<InputEvent>,
    damage: Vec<Rect>,
    /// Modifiers as of the last event, because AppKit reports them per event and
    /// Denise's `InputEvent::Text` has nowhere to carry them.
    modifiers: Modifiers,
    tracking: Option<Retained<NSTrackingArea>>,
}

impl ViewState {
    /// Queues an event for the next [`DeniseView::update`].
    fn push(&mut self, event: InputEvent) {
        self.events.push(event);
    }
}

define_class!(
    /// An `NSView` that draws a Denise surface and forwards Cocoa input to it.
    ///
    /// Create with [`DeniseView::new`] and add it to a host view as usual. It is
    /// an ordinary `NSView`: it can be autoresized, put in a split view, or made
    /// the content view of a window.
    // SAFETY:
    // - `NSView` has no subclassing requirement beyond being used on the main
    //   thread, which `MainThreadOnly` enforces.
    // - `DeniseView` does not implement `Drop`; the ivars do their own.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeniseView"]
    #[ivars = RefCell<ViewState>]
    pub struct DeniseView;

    impl DeniseView {
        /// Top-left origin, running downwards — the same convention Denise uses,
        /// so nothing between here and hit testing has to flip a coordinate.
        ///
        /// It also decides how `ViewSurface::draw_into` orients the image, which
        /// is why that function says so in its safety contract.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Keyboard input goes to the first responder, and a control that cannot
        /// become one cannot be typed into.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// The click that focuses a window should also reach the widget under it.
        /// Without this the first click on an unfocused window is swallowed, which
        /// users read as the control being broken.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// Take the `updateLayer` path rather than the `drawRect:` one.
        ///
        /// This is the whole of the zero-copy present. Answering `true` tells
        /// AppKit not to allocate a backing store and not to ask for drawing;
        /// it calls `updateLayer` instead, and what that assigns is the buffer
        /// the rasteriser has already written. Nothing is copied on the way to
        /// the screen.
        ///
        /// `drawRect:` remains below for hosts that drive the view themselves.
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        /// Hands the compositor the surface. Called instead of `drawRect:`.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            let Some(layer) = self.layer() else {
                return;
            };
            let state = self.ivars().borrow();
            let surface = state.surface.io_surface();

            // A layer's contents must be told the scale it is in, or a Retina
            // surface is drawn at twice its size and the bottom right of the
            // panel goes missing.
            layer.setContentsScale(CGFloat::from(state.surface.scale_factor()));

            // SAFETY: `IOSurfaceRef` is toll-free bridged to the `IOSurface`
            // class — documented, and the reason `contents` accepts one at all.
            // The pointer is valid while `state.surface` lives, and the layer
            // retains what it is given.
            let contents: &AnyObject =
                unsafe { &*(surface as *const IOSurfaceRef).cast::<AnyObject>() };
            // Actions off: `contents` is animatable, and its default action
            // cross-fades over a quarter of a second. A view redrawing at sixty
            // frames a second would spend all of them fading between buffers.
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            // SAFETY: assigning `contents` on the main thread, which is where
            // AppKit calls `updateLayer`.
            unsafe { layer.setContents(Some(contents)) };
            CATransaction::commit();
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let Some(context) = NSGraphicsContext::currentContext() else {
                return;
            };
            let state = self.ivars().borrow();
            let bounds = self.bounds();
            // SAFETY: this is AppKit's context for a flipped view — `isFlipped`
            // above is what makes that true — and it is live for the duration of
            // `drawRect:`.
            unsafe { state.surface.draw_into(&context.CGContext(), bounds) };
        }

        /// AppKit calls this when the view is resized or moves between screens,
        /// which is also when the backing scale can change.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            // SAFETY: calling the superclass implementation, which the docs
            // require before installing a replacement.
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            self.install_tracking_area();
        }

        /// A target for an `NSTimer`, for a host that wants a heartbeat rather
        /// than its own run-loop source.
        ///
        /// Anything that animates on its own — a blinking caret, a progress bar —
        /// needs waking without input. Ask [`DeniseView::next_wake_ms`] how soon,
        /// or just fire at a fixed rate and accept the wasted wakeups: an update
        /// with nothing to do costs one empty event list and no invalidation.
        #[unsafe(method(deniseTick:))]
        fn denise_tick(&self, _timer: *mut objc2::runtime::AnyObject) {
            self.update();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn backing_properties_changed(&self) {
            self.sync_surface_size();
        }

        // ----------------------------------------------------------- pointer

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        /// A drag is a move with a button held, and AppKit reports it separately.
        /// A control that only handles `mouseMoved:` loses the pointer the moment
        /// anyone presses on it.
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.pointer_moved(event);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().borrow_mut().push(InputEvent::PointerLeft);
            self.update();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.pointer_button(event, PointerButton::Left, ElementState::Down);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.pointer_button(event, PointerButton::Left, ElementState::Up);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.pointer_button(event, PointerButton::Right, ElementState::Down);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.pointer_button(event, PointerButton::Right, ElementState::Up);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            let button = other_button(event);
            self.pointer_button(event, button, ElementState::Down);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            let button = other_button(event);
            self.pointer_button(event, button, ElementState::Up);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let position = self.event_position(event);
            // A trackpad reports precise deltas already in points; a wheel reports
            // notches, which are only meaningful multiplied by a line height.
            let precise = event.hasPreciseScrollingDeltas();
            let scale = if precise { 1.0 } else { LINE_HEIGHT_PX };
            let backing = self.ivars().borrow().surface.scale_factor();
            let delta_x = -(event.scrollingDeltaX() as f32) * scale * backing;
            // AppKit's positive y is content moving up under the fingers; Denise's
            // positive y scrolls content down. They are opposites, and a backend
            // that forwards the sign unchanged scrolls the wrong way.
            let delta_y = -(event.scrollingDeltaY() as f32) * scale * backing;
            self.ivars().borrow_mut().push(InputEvent::PointerScroll {
                delta_x,
                delta_y,
                position,
            });
            self.update();
        }

        // ---------------------------------------------------------- keyboard

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.key(event, ElementState::Down);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            self.key(event, ElementState::Up);
        }

        /// Shift, control, option and command produce no key events of their own;
        /// AppKit reports them as a flags change. Without this, holding shift and
        /// pressing Tab would look like a plain Tab.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let modifiers = modifiers_of(event);
            let code = key_code(event.keyCode());
            let was = self.ivars().borrow().modifiers;
            // Whether the key that changed went down or up is not reported, so it
            // is inferred: more modifiers held than before means down.
            let state = if bit_count(modifiers) > bit_count(was) {
                ElementState::Down
            } else {
                ElementState::Up
            };
            let mut state_ref = self.ivars().borrow_mut();
            state_ref.modifiers = modifiers;
            state_ref.push(InputEvent::Key {
                code,
                state,
                repeat: false,
                modifiers,
            });
            drop(state_ref);
            self.update();
        }
    }
);

impl DeniseView {
    /// Creates a view of `frame` points, drawing whatever `delegate` paints.
    ///
    /// `scale_factor` is the backing scale to start with. AppKit corrects it
    /// through `viewDidChangeBackingProperties` once the view has a window, so a
    /// host that does not know it yet can pass `1.0`.
    pub fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        scale_factor: f32,
        delegate: Box<dyn ViewDelegate>,
    ) -> Result<Retained<Self>, Error> {
        let size = physical_size(frame.size, scale_factor);
        let surface = ViewSurface::new(size, scale_factor)?;

        let this = Self::alloc(mtm).set_ivars(RefCell::new(ViewState {
            surface,
            delegate,
            events: Vec::new(),
            damage: Vec::new(),
            modifiers: Modifiers::NONE,
            tracking: None,
        }));
        // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser and the
        // ivars are set before it runs, as `define_class!` requires.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // Layer-backed, because `updateLayer` above is only ever called for a
        // view that has a layer to update.
        this.setWantsLayer(true);
        this.install_tracking_area();
        Ok(this)
    }

    /// The view's state, including the surface.
    ///
    /// A `RefCell` because AppKit calls in re-entrantly and Rust has no way to
    /// know that; every borrow here is short and none spans a call back into
    /// AppKit.
    pub fn state(&self) -> &RefCell<ViewState> {
        self.ivars()
    }

    /// Runs the delegate over the queued input and invalidates what changed.
    ///
    /// Called after every event, and from the host's timer for anything that
    /// animates on its own. Safe to call when nothing has happened: an empty
    /// event list and no damage means no work and no invalidation.
    pub fn update(&self) {
        let mut borrow = self.ivars().borrow_mut();
        let state = &mut *borrow;

        state.damage.clear();
        let events = core::mem::take(&mut state.events);
        state
            .delegate
            .update(&mut state.surface, &events, &mut state.damage);

        // Published here rather than by the delegate, so that a delegate written
        // against the single-buffered version keeps working. With two surfaces
        // alternating, `present` is what makes the frame just drawn the one the
        // layer shows — a delegate that painted and never presented would draw
        // for ever into a buffer nobody is looking at.
        //
        // A no-op when the delegate did not acquire a frame: `present` only
        // swaps if there was something to swap, so an update that changed
        // nothing leaves the current frame on screen.
        let _ = state.surface.present(&state.damage);
        // Reuse the allocation rather than the contents.
        state.events = events;
        state.events.clear();

        // Collected before the borrow ends, because `setNeedsDisplayInRect:` can
        // re-enter and a live `RefMut` would panic if it did.
        let rects: Vec<NSRect> = state
            .damage
            .iter()
            .map(|rect| state.surface.damage_to_points(*rect))
            .map(|cg| {
                NSRect::new(
                    NSPoint::new(cg.origin.x, cg.origin.y),
                    NSSize::new(cg.size.width, cg.size.height),
                )
            })
            .collect();
        drop(borrow);

        for rect in rects {
            self.setNeedsDisplayInRect(rect);
        }
    }

    /// Milliseconds until the delegate next wants updating, or `None`.
    pub fn next_wake_ms(&self) -> Option<u64> {
        self.ivars().borrow().delegate.next_wake_ms()
    }

    /// Resizes the surface to match the view's current bounds and backing scale.
    ///
    /// Returns `true` if anything changed, in which case everything on screen is
    /// gone and the delegate owes a full repaint. The view cannot produce one on
    /// its own: damage belongs to whatever owns the scene.
    pub fn sync_surface_size(&self) -> bool {
        let bounds = self.bounds();
        let scale = self.backing_scale();
        let size = physical_size(bounds.size, scale);
        if size.is_empty() {
            return false;
        }
        let changed = self
            .ivars()
            .borrow_mut()
            .surface
            .resize(size, scale)
            .unwrap_or(false);
        if changed {
            self.setNeedsDisplay(true);
        }
        changed
    }

    /// The window's backing scale, or 1.0 before the view has a window.
    fn backing_scale(&self) -> f32 {
        let unit = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        let backing = self.convertRectToBacking(unit);
        if backing.size.width > 0.0 {
            backing.size.width as f32
        } else {
            1.0
        }
    }

    /// Installs a tracking area covering the whole view, so hover works without
    /// the host having to set `acceptsMouseMovedEvents` on its window.
    ///
    /// An embedded control that needed the host to configure the window would be
    /// a control that stops working the day somebody reuses the window.
    fn install_tracking_area(&self) {
        if let Some(old) = self.ivars().borrow_mut().tracking.take() {
            self.removeTrackingArea(&old);
        }
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveInKeyWindow
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: the owner outlives the tracking area — it is the view holding
        // it — and `userInfo` is allowed to be null.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                self.bounds(),
                options,
                Some(self),
                None,
            )
        };
        self.addTrackingArea(&area);
        self.ivars().borrow_mut().tracking = Some(area);
    }

    /// An event's location in physical pixels, top-left origin.
    fn event_position(&self, event: &NSEvent) -> Point {
        let window_point = event.locationInWindow();
        let local = self.convertPoint_fromView(window_point, None);
        let scale = self.backing_scale();
        Point::new(
            (local.x as f32 * scale).round() as i32,
            (local.y as f32 * scale).round() as i32,
        )
    }

    fn pointer_moved(&self, event: &NSEvent) {
        let position = self.event_position(event);
        self.ivars()
            .borrow_mut()
            .push(InputEvent::PointerMoved { position });
        self.update();
    }

    fn pointer_button(&self, event: &NSEvent, button: PointerButton, state: ElementState) {
        let position = self.event_position(event);
        let modifiers = modifiers_of(event);
        let mut ivars = self.ivars().borrow_mut();
        ivars.modifiers = modifiers;
        ivars.push(InputEvent::PointerButton {
            button,
            state,
            position,
            modifiers,
        });
        drop(ivars);
        self.update();
    }

    fn key(&self, event: &NSEvent, state: ElementState) {
        let code = key_code(event.keyCode());
        let modifiers = modifiers_of(event);
        let repeat = state == ElementState::Down && event.isARepeat();

        let mut ivars = self.ivars().borrow_mut();
        ivars.modifiers = modifiers;
        ivars.push(InputEvent::Key {
            code,
            state,
            repeat,
            modifiers,
        });

        // AppKit has already run the layout, the dead keys and any input method
        // by the time it gets here, so this is committed text and nothing else
        // needs to know which layout produced it. Control characters are dropped:
        // Enter, Tab and Backspace are keys, and a field that inserted a `\r`
        // would hold a character it can never draw.
        if state == ElementState::Down
            && !modifiers.contains(Modifiers::CTRL)
            && !modifiers.contains(Modifiers::SUPER)
            && let Some(characters) = event.characters()
        {
            for ch in characters.to_string().chars().filter(|c| !c.is_control()) {
                ivars.push(InputEvent::Text { ch });
            }
        }
        drop(ivars);
        self.update();
    }
}

/// The physical pixel extent of a view that is `size` points across.
fn physical_size(size: NSSize, scale_factor: f32) -> Size {
    let scale = scale_factor.max(0.01);
    Size::new(
        (size.width as f32 * scale).round().max(0.0) as u32,
        (size.height as f32 * scale).round().max(0.0) as u32,
    )
}

fn modifiers_of(event: &NSEvent) -> Modifiers {
    let flags = event.modifierFlags();
    let mut out = Modifiers::NONE;
    for (flag, modifier) in [
        (NSEventModifierFlags::Shift, Modifiers::SHIFT),
        (NSEventModifierFlags::Control, Modifiers::CTRL),
        (NSEventModifierFlags::Option, Modifiers::ALT),
        (NSEventModifierFlags::Command, Modifiers::SUPER),
    ] {
        if flags.contains(flag) {
            out |= modifier;
        }
    }
    out
}

fn bit_count(modifiers: Modifiers) -> u32 {
    [
        Modifiers::SHIFT,
        Modifiers::CTRL,
        Modifiers::ALT,
        Modifiers::SUPER,
    ]
    .into_iter()
    .filter(|m| modifiers.contains(*m))
    .count() as u32
}

/// AppKit's button 2 is the wheel; anything above that is a side button and keeps
/// its own index.
fn other_button(event: &NSEvent) -> PointerButton {
    match event.buttonNumber() {
        2 => PointerButton::Middle,
        other => PointerButton::Other(other.max(0) as u16),
    }
}
