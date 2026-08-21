//! `DeniseControl`: a child window an existing Win32 application can host.
//!
//! The split is the same one `Ui::render` makes. [`DeniseControl::update`]
//! consumes input, repaints the surface and invalidates what changed; `WM_PAINT`
//! only blits. Doing both in `WM_PAINT` would mean the tree could not damage
//! anything, because by then Windows has already decided what it will ask for.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use denise::{ElementState, InputEvent, Modifiers, Point, PointerButton, Rect, Size, Surface};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
// Named one by one rather than glob-imported. A `use ...::*` turns a constant
// this version of the bindings puts somewhere else — `WM_MOUSELEAVE` lives under
// `UI::Controls`, not here — into a binding that matches every message, and the
// only symptom is that every arm after it stops running.
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DLGC_WANTALLKEYS,
    DLGC_WANTARROWS, DLGC_WANTCHARS, DLGC_WANTTAB, DefWindowProcW, GWLP_USERDATA, RegisterClassExW,
    WHEEL_DELTA, WINDOW_EX_STYLE, WM_CHAR, WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE,
};
#[cfg(target_pointer_width = "64")]
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW};
#[cfg(not(target_pointer_width = "64"))]
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, SetWindowLongW};
use windows::core::{PCWSTR, w};

use crate::Error;
use crate::keymap::key_code;
use crate::surface::{DibSurface, screen_to_client};

/// Nominal pixels per wheel notch. `WHEEL_DELTA` is 120 and means one notch.
const LINE_HEIGHT_PX: f32 = 16.0;

/// The window class this control registers. Registered once per process, on
/// first use; a host that creates twenty of these registers nothing twice.
const CLASS_NAME: PCWSTR = w!("Denise.Control");

static REGISTER: Once = Once::new();

/// What the application implements to put something in the control.
///
/// Deliberately not "here is a `Ui`": a signage application drawing its own scene
/// with `denise-render` has no tree at all, and this backend has
/// no business requiring one.
pub trait ControlDelegate {
    /// Handles `events`, repaints `surface`, and appends what changed to `damage`.
    ///
    /// `damage` arrives empty. Leaving it empty means nothing changed and Windows
    /// is told nothing, which is what makes an idle panel cost nothing.
    fn update(&mut self, surface: &mut DibSurface, events: &[InputEvent], damage: &mut Vec<Rect>);

    /// Milliseconds until this delegate wants updating again, or `None` if it is
    /// waiting only on input.
    ///
    /// A blinking caret is why this exists. The host turns it into a `SetTimer`
    /// interval.
    fn next_wake_ms(&self) -> Option<u64> {
        None
    }
}

/// Everything one control owns. Lives behind the window's user data pointer.
struct ControlState {
    surface: DibSurface,
    delegate: Box<dyn ControlDelegate>,
    events: Vec<InputEvent>,
    damage: Vec<Rect>,
    /// Whether `TrackMouseEvent` is armed. Windows sends `WM_MOUSELEAVE` exactly
    /// once per arming, so this has to be re-armed after every leave or hover
    /// sticks on the last widget the pointer touched.
    tracking: bool,
    /// A `WM_CHAR` high surrogate waiting for its partner. Windows sends the two
    /// halves of a non-BMP character as separate messages, and a build that
    /// converted each one alone would drop every emoji and every rare CJK glyph.
    high_surrogate: Option<u16>,
}

impl ControlState {
    fn push(&mut self, event: InputEvent) {
        self.events.push(event);
    }
}

/// A Denise panel in a child window.
///
/// Does not own the window: Windows does, and the parent destroys it. Dropping
/// this is not a destroy, which is what lets a host keep the `HWND` in its own
/// structures the way it keeps every other control's.
#[derive(Clone, Copy, Debug)]
pub struct DeniseControl {
    hwnd: HWND,
}

impl DeniseControl {
    /// Creates a child window inside `parent`, at `bounds` in physical pixels.
    ///
    /// `scale_factor` is the DPI over 96. A host that does not know it yet can
    /// pass `1.0` and correct it with [`DeniseControl::set_scale_factor`] on
    /// `WM_DPICHANGED`.
    pub fn new(
        parent: HWND,
        bounds: Rect,
        scale_factor: f32,
        delegate: Box<dyn ControlDelegate>,
    ) -> Result<Self, Error> {
        register_class()?;

        let size = Size::new(bounds.width.max(0) as u32, bounds.height.max(0) as u32);
        let surface = DibSurface::new(size, scale_factor)?;
        let state = Box::new(RefCell::new(ControlState {
            surface,
            delegate,
            events: Vec::new(),
            damage: Vec::new(),
            tracking: false,
            high_surrogate: None,
        }));

        // SAFETY: `CLASS_NAME` is registered above; `parent` is the caller's
        // window; and the state pointer is claimed by `WM_NCCREATE`, which runs
        // inside this call, so it is either adopted or leaked-and-reclaimed on the
        // error path below.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS_NAME,
                PCWSTR::null(),
                // No WS_TABSTOP: whether this control takes part in a dialog's tab
                // order is the host's decision, not ours, and it can add the style.
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                Some(parent),
                None,
                None,
                Some(Box::into_raw(state).cast()),
            )
        }
        .map_err(|_| Error::CreateWindow)?;

        Ok(Self { hwnd })
    }

    /// The control's window handle, for `SetWindowPos`, `ShowWindow` and every
    /// other thing a host does to a control.
    #[inline]
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Runs the delegate over the queued input and invalidates what changed.
    ///
    /// Called after every message that produces input, and from the host's timer
    /// for anything that animates on its own. Safe to call with nothing pending:
    /// an empty event list and no damage means no work and no invalidation.
    pub fn update(&self) {
        // SAFETY: `hwnd` is this control's window, created by `new`.
        let Some(state) = (unsafe { state_of(self.hwnd) }) else {
            return;
        };
        let rects = {
            let mut borrow = state.borrow_mut();
            let state = &mut *borrow;

            state.damage.clear();
            let events = core::mem::take(&mut state.events);
            state
                .delegate
                .update(&mut state.surface, &events, &mut state.damage);
            state.events = events;
            state.events.clear();

            // Collected before the borrow ends: `InvalidateRect` can re-enter, and
            // a live `RefMut` would panic if it did.
            state.damage.clone()
        };

        for rect in rects {
            let native = RECT {
                left: rect.x,
                top: rect.y,
                right: rect.x + rect.width,
                bottom: rect.y + rect.height,
            };
            // SAFETY: `hwnd` is this control's window and `native` is a live local.
            // `false` for erase: the panel writes every pixel it owns, and letting
            // Windows fill the background first is a flash of grey on every frame.
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), Some(&native), false);
            }
        }
    }

    /// Milliseconds until the delegate next wants updating, or `None`.
    pub fn next_wake_ms(&self) -> Option<u64> {
        // SAFETY: `hwnd` is this control's window, created by `new`.
        let state = unsafe { state_of(self.hwnd) }?;
        state.borrow().delegate.next_wake_ms()
    }

    /// Tells the control its DPI changed. Reallocates and repaints everything.
    ///
    /// Windows sends `WM_DPICHANGED` to top-level windows only, so a child control
    /// finds out from its parent or not at all.
    pub fn set_scale_factor(&self, scale_factor: f32) {
        // SAFETY: `hwnd` is this control's window.
        let Some(state) = (unsafe { state_of(self.hwnd) }) else {
            return;
        };
        let size = state.borrow().surface.size();
        let changed = state
            .borrow_mut()
            .surface
            .resize(size, scale_factor)
            .unwrap_or(false);
        if changed {
            // SAFETY: as above; a null rectangle means the whole client area.
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
    }
}

/// Registers the window class, once per process.
fn register_class() -> Result<(), Error> {
    let mut result = Ok(());
    REGISTER.call_once(|| {
        // SAFETY: a null module name asks for the handle of the current process,
        // which is what a class registered by this library wants.
        let instance = match unsafe { GetModuleHandleW(None) } {
            Ok(handle) => handle,
            Err(_) => {
                result = Err(Error::RegisterClass);
                return;
            }
        };

        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            // Redraw on either kind of resize, and take double clicks: a control
            // that swallows the second click of a pair looks broken to anyone who
            // clicks quickly.
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            // No background brush at all. The panel paints every pixel it owns,
            // and a brush means Windows fills the client area first — a grey flash
            // on every resize, and tearing on every repaint.
            hbrBackground: Default::default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };

        // SAFETY: `class` is fully initialised and `wnd_proc` has the required
        // signature.
        if unsafe { RegisterClassExW(&class) } == 0 {
            result = Err(Error::RegisterClass);
        }
    });
    result
}

/// The state behind a control window, or `None` if there is none yet.
///
/// # Safety
///
/// `hwnd` must be a window of this library's class, or null.
unsafe fn state_of<'a>(hwnd: HWND) -> Option<&'a RefCell<ControlState>> {
    if hwnd.is_invalid() {
        return None;
    }
    // SAFETY: the caller promises the window is ours, so its user data is either
    // null or the pointer `WM_NCCREATE` stored.
    let pointer = unsafe { get_user_data(hwnd) } as *const RefCell<ControlState>;
    // SAFETY: as above; the box outlives the window, which `WM_NCDESTROY` is what
    // ends.
    unsafe { pointer.as_ref() }
}

// `SetWindowLongPtrW` does not exist on 32-bit Windows; `SetWindowLongW` is the
// same call with a narrower word. Both matter here: the ActiveX shim this control
// exists for is hosted by 32-bit applications more often than 64-bit ones.
#[cfg(target_pointer_width = "64")]
unsafe fn get_user_data(hwnd: HWND) -> isize {
    // SAFETY: forwarding the caller's promise about `hwnd`.
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
}

#[cfg(target_pointer_width = "64")]
unsafe fn set_user_data(hwnd: HWND, value: isize) {
    // SAFETY: forwarding the caller's promise about `hwnd`.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, value) };
}

#[cfg(not(target_pointer_width = "64"))]
unsafe fn get_user_data(hwnd: HWND) -> isize {
    // SAFETY: forwarding the caller's promise about `hwnd`.
    unsafe { GetWindowLongW(hwnd, GWLP_USERDATA) as isize }
}

#[cfg(not(target_pointer_width = "64"))]
unsafe fn set_user_data(hwnd: HWND, value: isize) {
    // SAFETY: forwarding the caller's promise about `hwnd`.
    unsafe { SetWindowLongW(hwnd, GWLP_USERDATA, value as i32) };
}

/// The window procedure.
///
/// Every path through this is wrapped in [`catch_unwind`]. A Rust panic unwinding
/// into `DispatchMessage` is undefined behaviour, and a bug in a panel has no
/// business taking down a host application with three other windows open.
extern "system" fn wnd_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Windows only calls this for windows of our class.
        unsafe { handle(hwnd, message, wparam, lparam) }
    }));
    match handled {
        Ok(Some(result)) => result,
        // Either unhandled or a panic. Both want the default behaviour, which for
        // a panic means the window keeps working even if this message did not.
        Ok(None) | Err(_) => {
            // SAFETY: the standard fallback, valid for any window and message.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

/// Handles one message, or returns `None` to fall through to `DefWindowProc`.
///
/// # Safety
///
/// Called only by [`wnd_proc`], for a window of this library's class.
unsafe fn handle(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_NCCREATE => {
            // SAFETY: for WM_NCCREATE, lParam is the CREATESTRUCTW Windows built
            // from the CreateWindowEx arguments, and `lpCreateParams` is the
            // pointer passed there — a leaked `Box<RefCell<ControlState>>`.
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            // SAFETY: as above.
            unsafe { set_user_data(hwnd, create.lpCreateParams as isize) };
            // Fall through: DefWindowProc has to see WM_NCCREATE or the window is
            // never created at all.
            None
        }
        WM_NCDESTROY => {
            // SAFETY: the pointer was stored by WM_NCCREATE and nothing else has
            // taken it; clearing it first means a late message finds no state
            // rather than a freed box.
            let pointer = unsafe { get_user_data(hwnd) } as *mut RefCell<ControlState>;
            // SAFETY: as above.
            unsafe { set_user_data(hwnd, 0) };
            if !pointer.is_null() {
                // SAFETY: the box came from `Box::into_raw` in `new` and is
                // reclaimed exactly here.
                drop(unsafe { Box::from_raw(pointer) });
            }
            None
        }

        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: `hwnd` is our window and `paint` is a live local.
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                let clip = Rect::new(
                    paint.rcPaint.left,
                    paint.rcPaint.top,
                    paint.rcPaint.right - paint.rcPaint.left,
                    paint.rcPaint.bottom - paint.rcPaint.top,
                );
                // SAFETY: `dc` is live between BeginPaint and EndPaint.
                unsafe { state.borrow().surface.blit(dc, &[clip]) };
            }
            // SAFETY: paired with the BeginPaint above.
            unsafe {
                let _ = EndPaint(hwnd, &paint);
            };
            Some(LRESULT(0))
        }

        // The panel paints every pixel of its client area, so letting Windows
        // erase first is a grey flash and nothing else.
        WM_ERASEBKGND => Some(LRESULT(1)),

        WM_SIZE => {
            let width = (lparam.0 & 0xFFFF) as u32;
            let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                let scale = state.borrow().surface.scale_factor();
                let changed = state
                    .borrow_mut()
                    .surface
                    .resize(Size::new(width, height), scale)
                    .unwrap_or(false);
                if changed {
                    state.borrow_mut().push(InputEvent::SurfaceResized {
                        size: Size::new(width, height),
                        scale_factor: scale,
                    });
                    // SAFETY: our window; null rectangle means the whole client
                    // area, which is what a resize invalidates.
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    };
                }
            }
            // SAFETY: our window.
            unsafe { control(hwnd).update() };
            Some(LRESULT(0))
        }

        // Without this, a control inside a dialog never sees Tab, Enter or the
        // arrow keys: the dialog manager takes them for its own navigation. A
        // keyboard-only panel that cannot receive Tab is not a panel.
        WM_GETDLGCODE => Some(LRESULT(
            (DLGC_WANTALLKEYS | DLGC_WANTCHARS | DLGC_WANTARROWS | DLGC_WANTTAB) as isize,
        )),

        // Clicking a control should focus it, the way every other control behaves.
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            // SAFETY: our window.
            unsafe {
                let _ = SetFocus(Some(hwnd));
            };
            // Capture, so a press that drags off the control still reports its
            // release. Without it the widget stays stuck in its pressed state.
            // SAFETY: our window.
            unsafe { SetCapture(hwnd) };
            let button = match message {
                WM_RBUTTONDOWN => PointerButton::Right,
                WM_MBUTTONDOWN => PointerButton::Middle,
                _ => PointerButton::Left,
            };
            // SAFETY: our window.
            unsafe { pointer_button(hwnd, lparam, button, ElementState::Down) };
            Some(LRESULT(0))
        }

        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
            // SAFETY: paired with the SetCapture above.
            unsafe {
                let _ = ReleaseCapture();
            };
            let button = match message {
                WM_RBUTTONUP => PointerButton::Right,
                WM_MBUTTONUP => PointerButton::Middle,
                _ => PointerButton::Left,
            };
            // SAFETY: our window.
            unsafe { pointer_button(hwnd, lparam, button, ElementState::Up) };
            Some(LRESULT(0))
        }

        WM_MOUSEMOVE => {
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                if !state.borrow().tracking {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` is fully initialised and `hwnd` is ours.
                    // Windows fires WM_MOUSELEAVE once per arming, so this is
                    // re-armed after every leave.
                    if unsafe { TrackMouseEvent(&mut track) }.is_ok() {
                        state.borrow_mut().tracking = true;
                    }
                }
                let position = client_point(lparam);
                state
                    .borrow_mut()
                    .push(InputEvent::PointerMoved { position });
            }
            // SAFETY: our window.
            unsafe { control(hwnd).update() };
            Some(LRESULT(0))
        }

        WM_MOUSELEAVE => {
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                state.borrow_mut().tracking = false;
                state.borrow_mut().push(InputEvent::PointerLeft);
            }
            // SAFETY: our window.
            unsafe { control(hwnd).update() };
            Some(LRESULT(0))
        }

        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            // The wheel messages carry *screen* coordinates where every other
            // mouse message carries client ones. A backend that forwards them
            // unchanged scrolls correctly only when the window is at the top-left
            // of the display.
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let (x, y) = screen_to_client(hwnd, x, y);
            let notches = ((wparam.0 >> 16) & 0xFFFF) as i16 as f32 / WHEEL_DELTA as f32;
            // Positive wparam is away from the user, which scrolls content up.
            // Denise's positive y scrolls content down; they are opposites.
            let (delta_x, delta_y) = if message == WM_MOUSEHWHEEL {
                (notches * LINE_HEIGHT_PX, 0.0)
            } else {
                (0.0, -notches * LINE_HEIGHT_PX)
            };
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                state.borrow_mut().push(InputEvent::PointerScroll {
                    delta_x,
                    delta_y,
                    position: Point::new(x, y),
                });
            }
            // SAFETY: our window.
            unsafe { control(hwnd).update() };
            Some(LRESULT(0))
        }

        // WM_SYSKEYDOWN as well as WM_KEYDOWN: alt-modified keys arrive as the
        // system variant, and a panel that ignores them cannot see AltGr at all.
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let extended = lparam.0 & (1 << 24) != 0;
            // Bit 30 is the previous key state: set means it was already down, so
            // this is auto-repeat.
            let repeat = down && lparam.0 & (1 << 30) != 0;
            let code = key_code(wparam.0 as u16, extended);
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                state.borrow_mut().push(InputEvent::Key {
                    code,
                    state: if down {
                        ElementState::Down
                    } else {
                        ElementState::Up
                    },
                    repeat,
                    modifiers: current_modifiers(),
                });
            }
            // SAFETY: our window.
            unsafe { control(hwnd).update() };
            // Falling through would let DefWindowProc turn Alt+key into a menu
            // beep, but it is also what generates WM_CHAR. So: handled for the
            // system variants, passed on for the plain ones.
            if message == WM_SYSKEYDOWN || message == WM_SYSKEYUP {
                Some(LRESULT(0))
            } else {
                None
            }
        }

        WM_CHAR => {
            let unit = wparam.0 as u16;
            // SAFETY: our window.
            if let Some(state) = unsafe { state_of(hwnd) } {
                let mut borrow = state.borrow_mut();
                // Windows sends a non-BMP character as two messages, a high
                // surrogate then a low one. Converting each alone drops every
                // emoji and every rare CJK glyph.
                let scalar = match (borrow.high_surrogate.take(), unit) {
                    (_, 0xD800..=0xDBFF) => {
                        borrow.high_surrogate = Some(unit);
                        None
                    }
                    (Some(high), 0xDC00..=0xDFFF) => {
                        let combined =
                            0x1_0000 + ((high as u32 - 0xD800) << 10) + (unit as u32 - 0xDC00);
                        char::from_u32(combined)
                    }
                    // A low surrogate with no partner is a broken sequence, not a
                    // character. Dropping it is the only sound answer.
                    (None, 0xDC00..=0xDFFF) => None,
                    _ => char::from_u32(unit as u32),
                };
                // Control characters are keys, never text: Enter, Tab and
                // Backspace already arrived as `Key`, and a field that inserted a
                // `\r` would hold a character it can never draw.
                if let Some(ch) = scalar.filter(|c| !c.is_control()) {
                    borrow.push(InputEvent::Text { ch });
                }
            }
            // SAFETY: our window.
            unsafe { control(hwnd).update() };
            Some(LRESULT(0))
        }

        _ => None,
    }
}

/// Wraps a window handle so the message handlers can call `update`.
///
/// # Safety
///
/// `hwnd` must be a window of this library's class.
unsafe fn control(hwnd: HWND) -> DeniseControl {
    DeniseControl { hwnd }
}

/// The client-relative position packed into a mouse message's `lParam`.
///
/// The halves are signed: a captured drag off the left edge reports a negative x,
/// and reading them as unsigned turns that into 65,000-odd.
fn client_point(lparam: LPARAM) -> Point {
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    Point::new(x, y)
}

/// # Safety
///
/// `hwnd` must be a window of this library's class.
unsafe fn pointer_button(hwnd: HWND, lparam: LPARAM, button: PointerButton, element: ElementState) {
    // SAFETY: forwarding the caller's promise about `hwnd`.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.borrow_mut().push(InputEvent::PointerButton {
            button,
            state: element,
            position: client_point(lparam),
            modifiers: current_modifiers(),
        });
    }
    // SAFETY: as above.
    unsafe { control(hwnd).update() };
}

/// The modifiers held right now.
///
/// Asked of Windows rather than tracked, because a control that did not have
/// focus while shift went down never saw the key event and would otherwise
/// believe it is still up.
fn current_modifiers() -> Modifiers {
    let mut out = Modifiers::NONE;
    for (key, modifier) in [
        (VK_SHIFT, Modifiers::SHIFT),
        (VK_CONTROL, Modifiers::CTRL),
        (VK_MENU, Modifiers::ALT),
    ] {
        // SAFETY: `GetKeyState` takes a virtual key code and nothing else.
        if unsafe { GetKeyState(key.0 as i32) } < 0 {
            out |= modifier;
        }
    }
    for key in [VK_LWIN, VK_RWIN] {
        // SAFETY: as above.
        if unsafe { GetKeyState(key.0 as i32) } < 0 {
            out |= Modifiers::SUPER;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The halves of a mouse `lParam` are signed. A captured drag off the left
    /// edge reports a negative x, and reading it as unsigned turns that into
    /// 65,000-odd — a hit test that then finds nothing, and a button that stays
    /// pressed forever.
    #[test]
    fn a_drag_off_the_left_edge_reports_a_negative_x() {
        let packed = LPARAM(((10i32 as u32) << 16 | (-3i32 as u32 & 0xFFFF)) as isize);
        assert_eq!(client_point(packed), Point::new(-3, 10));
    }

    #[test]
    fn a_position_inside_the_window_survives_unpacking() {
        let packed = LPARAM(((400i32 as u32) << 16 | 250) as isize);
        assert_eq!(client_point(packed), Point::new(250, 400));
    }
}
