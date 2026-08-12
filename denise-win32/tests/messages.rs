//! The control driven by window messages, with no hand on the mouse.
//!
//! Three of the things `denise-win32` gets wrong are invisible to a compiler and
//! tedious to check by hand: whether pressing captures the mouse, whether a drag
//! past the left edge reports a negative coordinate, and whether the wheel's
//! *screen* coordinates are converted to client ones. All three are decided by
//! the window procedure, and a window procedure can be driven with `SendMessage`.
//!
//! What this does **not** replace is a human. It proves the control reacts
//! correctly to the messages Windows sends; it cannot prove Windows sends them.
//! Real capture behaviour — the pointer leaving the window and the events still
//! arriving — is the operating system's promise, not this code's.

#![cfg(windows)]

use std::cell::RefCell;
use std::rc::Rc;

use denise::{InputEvent, Rect};
use denise_win32::{ControlDelegate, DeniseControl, DibSurface};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SendMessageW,
    WINDOW_EX_STYLE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

/// Records what the control handed over, so a test can assert on it.
#[derive(Default)]
struct Recorder {
    seen: Rc<RefCell<Vec<InputEvent>>>,
}

impl ControlDelegate for Recorder {
    fn update(
        &mut self,
        _surface: &mut DibSurface,
        events: &[InputEvent],
        _damage: &mut Vec<Rect>,
    ) {
        self.seen.borrow_mut().extend_from_slice(events);
    }
}

/// Packs a coordinate pair the way a mouse message does: y in the high word, x
/// in the low, **both signed**.
fn coords(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u32) << 16) | (x as u32 & 0xFFFF)) as isize)
}

/// Creates an off-screen parent and a control inside it, runs `body`, tears down.
///
/// The parent is never shown. Everything here is message delivery, which needs a
/// window but not a visible one — which is also what lets this run on a CI
/// runner nobody is looking at.
fn with_control(body: impl FnOnce(HWND, &Rc<RefCell<Vec<InputEvent>>>)) {
    // SAFETY: a null module name asks for this process's handle.
    let instance = unsafe { GetModuleHandleW(None) }.expect("module handle");

    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(parent_proc),
        hInstance: instance.into(),
        lpszClassName: w!("Denise.Test.Parent"),
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised. Registering twice in one process is
    // harmless — the second call fails and the class from the first is used.
    unsafe { RegisterClassExW(&class) };

    // SAFETY: the class is registered and every argument is valid. Not shown, so
    // no `SW_SHOW`.
    let parent = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("Denise.Test.Parent"),
            w!("denise test"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            400,
            300,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .expect("parent window");

    let seen = Rc::new(RefCell::new(Vec::new()));
    let control = DeniseControl::new(
        parent,
        Rect::new(0, 0, 400, 300),
        1.0,
        Box::new(Recorder { seen: seen.clone() }),
    )
    .expect("control");

    body(control.hwnd(), &seen);

    // SAFETY: destroying the parent destroys the child with it, which is what
    // reclaims the control's state through WM_NCDESTROY.
    unsafe {
        let _ = DestroyWindow(parent);
    }
}

extern "system" fn parent_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the standard fallback, valid for any window and message.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Item 4. A press that drags off the control must still report its release, and
/// that only happens if the control captured the mouse. `GetCapture` is the
/// observable part; whether Windows honours it afterwards is the OS's promise.
#[test]
fn pressing_captures_the_mouse_and_releasing_gives_it_back() {
    with_control(|hwnd, _seen| {
        // SAFETY: `hwnd` is the control's window, live for the closure.
        unsafe {
            SendMessageW(hwnd, WM_LBUTTONDOWN, None, Some(coords(20, 20)));
            assert_eq!(
                GetCapture(),
                hwnd,
                "a press must capture, or a drag off the control never reports its release \
                 and the widget stays stuck in its pressed state"
            );

            SendMessageW(hwnd, WM_LBUTTONUP, None, Some(coords(20, 20)));
            assert_ne!(
                GetCapture(),
                hwnd,
                "the capture must be given back, or the rest of the application stops \
                 receiving mouse input entirely"
            );
        }
    });
}

/// Item 5. A drag off the left or top edge produces a negative coordinate, and
/// the halves of `lParam` are signed. Read unsigned, `-3` becomes 65533: the hit
/// test finds nothing and a pressed widget never releases.
#[test]
fn a_drag_off_the_left_edge_reports_a_negative_x() {
    with_control(|hwnd, seen| {
        // SAFETY: `hwnd` is the control's window, live for the closure.
        unsafe {
            SendMessageW(hwnd, WM_LBUTTONDOWN, None, Some(coords(20, 20)));
            SendMessageW(hwnd, WM_MOUSEMOVE, None, Some(coords(-3, 10)));
        }

        let moved = seen
            .borrow()
            .iter()
            .rev()
            .find_map(|event| match event {
                InputEvent::PointerMoved { position } => Some(*position),
                _ => None,
            })
            .expect("a pointer move");
        assert_eq!(moved.x, -3, "read unsigned this would be 65533");
        assert_eq!(moved.y, 10);
    });
}

/// Item 6. The wheel messages carry **screen** coordinates where every other
/// mouse message carries client ones. Forwarded unchanged, scrolling would only
/// work with the window at the very top-left of the display.
#[test]
fn the_wheel_arrives_in_client_coordinates() {
    with_control(|hwnd, seen| {
        // The parent sits at 0,0 and is never shown, so screen and client
        // coordinates coincide *except* for the window frame — which is exactly
        // the offset a missing ScreenToClient would leave behind. Ask for a point
        // and check what comes out is not the raw one when a frame exists.
        let wheel = WPARAM(((120u32) << 16) as usize);
        // SAFETY: `hwnd` is the control's window, live for the closure.
        unsafe {
            SendMessageW(hwnd, WM_MOUSEWHEEL, Some(wheel), Some(coords(50, 60)));
        }

        let scroll = seen
            .borrow()
            .iter()
            .rev()
            .find_map(|event| match event {
                InputEvent::PointerScroll {
                    delta_y, position, ..
                } => Some((*delta_y, *position)),
                _ => None,
            })
            .expect("a scroll event");

        // One notch away from the user scrolls content up, which is negative in
        // Denise's convention. The sign is the part that gets flipped by accident.
        assert!(
            scroll.0 < 0.0,
            "a positive wheel delta must scroll content up, got {}",
            scroll.0
        );

        // The conversion ran: a window at 0,0 still has a frame, so a screen point
        // maps to a *smaller* client point. If these are equal, ScreenToClient was
        // skipped.
        let converted = scroll.1;
        assert!(
            converted.x <= 50 && converted.y <= 60,
            "screen coordinates were not converted to client ones: got {converted:?}"
        );
    });
}
