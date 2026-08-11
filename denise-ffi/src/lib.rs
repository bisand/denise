//! Denise's C ABI: a `cdylib` for hosts that are not written in Rust.
//!
//! Denise's own backends are Rust and need none of this. This crate exists for
//! the other direction — a Win32 control inside an MFC application, a WinForms or
//! VB6 host reaching it through the ActiveX shim, an `NSView` in a Cocoa app, a
//! Python or C# panel on an embedded box. All of them speak C.
//!
//! # The shape of it
//!
//! The host owns the window and the pixel buffer; Denise owns the widget tree and
//! draws into whatever it is handed:
//!
//! ```c
//! DeniseUi *ui = denise_ui_new(800, 480, DENISE_THEME_DARK);
//! uint64_t root = denise_ui_root(ui);
//! denise_ui_add_button(ui, root, (DeniseRect){20, 20, 160, 44}, "Save", 1, DENISE_ROLE_PRIMARY);
//!
//! /* per frame */
//! denise_ui_tick(ui, now_ms);
//! if (denise_ui_needs_paint(ui)) {
//!     DeniseFrame frame = { pixels, len, w, h, stride, DENISE_FORMAT_XRGB8888, age };
//!     denise_ui_paint(ui, &frame);
//!     DeniseRect damage[16];
//!     intptr_t n = denise_ui_damage(ui, damage, 16);
//!     /* BitBlt only those rectangles */
//!     denise_ui_presented(ui);
//! }
//! uint32_t message;
//! while (denise_ui_poll_message(ui, &message)) { /* ... */ }
//! ```
//!
//! There is no `Surface` here and no event loop. Both belong to the host, and a
//! library that tried to own either would be unembeddable in exactly the places
//! this is for.
//!
//! # Rules the whole ABI keeps
//!
//! - **Handles are opaque.** A `DeniseUi *` comes from [`denise_ui_new`] and goes
//!   to [`denise_ui_free`]. Nothing else may free it.
//! - **A node is a `uint64_t`**, and `0` is never a valid node. Ids carry a
//!   generation, so an id kept past a [`denise_ui_remove`] fails to resolve rather
//!   than addressing whoever took the slot.
//! - **A message is a `uint32_t`**, chosen by the host, and `0` means *no
//!   message*. A button given `0` emits nothing, and [`denise_ui_poll_message`]
//!   never yields it. That is what lets a widget be created without one.
//! - **Strings are NUL-terminated UTF-8** going in and coming out. Invalid UTF-8
//!   is [`DENISE_ERR_INVALID`], not a replacement character: silently mangling a
//!   host's text is worse than refusing it.
//! - **A negative return is a status**, and every status has a message from
//!   [`denise_status_message`].
//! - **Nothing is thread-safe.** One `DeniseUi` belongs to one thread, which for
//!   every host this targets is the UI thread it was created on.
//!
//! # Panics do not cross
//!
//! Every entry point catches unwinding and returns [`DENISE_ERR_PANIC`]. A panic
//! is a bug in Denise, and a bug in Denise should not take down a host process
//! that has unsaved work in three other windows. The call did nothing; the `Ui`
//! it was called on should be treated as suspect and freed.
//!
//! # The header is the contract
//!
//! `include/denise.h` is written by hand, not generated. A generated header
//! follows whatever the Rust happens to say this week, which is the opposite of
//! what a stable ABI means — the header is the thing that must not move, and the
//! Rust is what gets checked against it. [`tests/header.rs`](../tests/header.rs)
//! does the checking: every exported symbol appears in both, with the same
//! numbers for every key, role and constant.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

use denise_ui::Ui;

pub mod input;
pub mod keys;
pub mod paint;
pub mod tree;
pub mod types;

// Flattened so a Rust caller of the `rlib` sees the same names in the same shape
// as the header, rather than having to learn which module a symbol lives in when
// C never has to. `keys` stays namespaced: 102 constants at the crate root would
// bury everything else.
pub use input::*;
pub use paint::*;
pub use tree::*;
pub use types::*;

/// Version of this ABI.
///
/// Bumped when a signature, a constant or the meaning of one changes. Added
/// functions do not bump it. A host that checks nothing else should check this.
pub const DENISE_ABI_VERSION: u32 = 1;

/// The call succeeded.
pub const DENISE_OK: i32 = 0;
/// A required pointer was `NULL`.
pub const DENISE_ERR_NULL: i32 = -1;
/// An argument was out of range, or a string was not valid UTF-8.
pub const DENISE_ERR_INVALID: i32 = -2;
/// The node id does not name a live node.
pub const DENISE_ERR_NO_NODE: i32 = -3;
/// The buffer supplied is too small for the result.
pub const DENISE_ERR_BUFFER_TOO_SMALL: i32 = -4;
/// The widget named is not of the kind this call needs.
pub const DENISE_ERR_WRONG_WIDGET: i32 = -5;
/// A panic was caught. The call did nothing; treat the `DeniseUi` as suspect.
pub const DENISE_ERR_PANIC: i32 = -6;

/// The crate version, NUL-terminated and statically allocated.
///
/// The trailing NUL is concatenated in rather than written, so this cannot drift
/// from `Cargo.toml`.
static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");

/// Denise's version as a NUL-terminated string. Never `NULL`, never freed.
#[unsafe(no_mangle)]
pub extern "C" fn denise_version() -> *const c_char {
    VERSION.as_ptr().cast()
}

/// The ABI version this library was built with. See [`DENISE_ABI_VERSION`].
#[unsafe(no_mangle)]
pub extern "C" fn denise_abi_version() -> u32 {
    DENISE_ABI_VERSION
}

/// A short description of a status code, NUL-terminated. Never `NULL`, never
/// freed. An unrecognised code gets a generic message rather than `NULL`, because
/// a host logging an error should not have to handle an error from the logger.
#[unsafe(no_mangle)]
pub extern "C" fn denise_status_message(status: i32) -> *const c_char {
    let text: &CStr = match status {
        DENISE_OK => c"ok",
        DENISE_ERR_NULL => c"a required pointer was NULL",
        DENISE_ERR_INVALID => c"an argument was out of range, or a string was not UTF-8",
        DENISE_ERR_NO_NODE => c"the node id does not name a live node",
        DENISE_ERR_BUFFER_TOO_SMALL => c"the buffer supplied is too small",
        DENISE_ERR_WRONG_WIDGET => c"the node is not a widget of that kind",
        DENISE_ERR_PANIC => c"denise panicked; the call did nothing",
        _ => c"unknown status",
    };
    text.as_ptr()
}

/// An opaque user interface. Create with [`denise_ui_new`], destroy with
/// [`denise_ui_free`].
///
/// Messages are `u32` because C has no generics and a host needs to choose its
/// own vocabulary. They are queued here rather than read from
/// [`Ui::drain_messages`] on demand so that `0` can be dropped on the way in and
/// [`denise_ui_poll_message`] can keep its promise never to yield one.
pub struct DeniseUi {
    ui: Ui<u32>,
    pending: VecDeque<u32>,
}

impl DeniseUi {
    /// Moves anything the widgets emitted into the queue, discarding the `0`s.
    fn collect(&mut self) {
        self.pending
            .extend(self.ui.drain_messages().filter(|&m| m != 0));
    }
}

/// Runs `body`, turning a panic into `fallback`.
///
/// Unwinding into C is undefined behaviour. `extern "C"` aborts rather than
/// unwind, which is sound but takes the host down with it; a status code lets the
/// host log the bug and carry on with its other windows.
///
/// The `AssertUnwindSafe` is deliberate and is the reason [`DENISE_ERR_PANIC`]'s
/// documentation says to treat the `Ui` as suspect: a panic part-way through a
/// tree mutation can leave state a later call would observe. Better observable
/// and recoverable than an aborted process.
pub(crate) fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => fallback,
    }
}

/// Borrows a handle, or `None` if it is `NULL`.
///
/// # Safety
///
/// `ui` must be `NULL`, or a pointer from [`denise_ui_new`] that has not been
/// passed to [`denise_ui_free`].
pub(crate) unsafe fn handle<'a>(ui: *mut DeniseUi) -> Option<&'a mut DeniseUi> {
    // SAFETY: the caller promises the pointer is either null or one we handed out
    // from `Box::into_raw` and still own. `as_mut` handles the null case, and the
    // ABI's one-thread-per-`DeniseUi` rule is what makes the exclusive borrow
    // sound.
    unsafe { ui.as_mut() }
}

/// Reads a NUL-terminated UTF-8 string, or `None` if it is `NULL` or not UTF-8.
///
/// # Safety
///
/// `text` must be `NULL`, or point at a NUL-terminated byte string that stays
/// valid and unwritten for the duration of the call.
pub(crate) unsafe fn utf8<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    // SAFETY: the caller promises a NUL-terminated string valid for the call, and
    // the returned lifetime is only ever used within one.
    unsafe { CStr::from_ptr(text) }.to_str().ok()
}

/// Creates a user interface `width` by `height` pixels, in one of the built-in
/// themes ([`DENISE_THEME_DARK`] and friends).
///
/// Returns `NULL` if the size is empty or the theme is not one of them.
#[unsafe(no_mangle)]
pub extern "C" fn denise_ui_new(width: u32, height: u32, theme: u32) -> *mut DeniseUi {
    guard(std::ptr::null_mut(), || {
        let Some(theme) = types::theme(theme) else {
            return std::ptr::null_mut();
        };
        if width == 0 || height == 0 {
            return std::ptr::null_mut();
        }
        Box::into_raw(Box::new(DeniseUi {
            ui: Ui::new(denise::Size::new(width, height), theme),
            pending: VecDeque::new(),
        }))
    })
}

/// Destroys a user interface. `NULL` is accepted and does nothing, as `free`
/// does.
///
/// # Safety
///
/// `ui` must be `NULL`, or a pointer from [`denise_ui_new`] not already freed.
/// Every node id taken from it is dead afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_free(ui: *mut DeniseUi) {
    if ui.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: the caller promises this came from `denise_ui_new`, which built
        // it with `Box::into_raw`, and that it has not been freed already.
        drop(unsafe { Box::from_raw(ui) });
    });
}

/// Switches to one of the built-in themes and repaints everything.
///
/// # Safety
///
/// `ui` must be a live handle from [`denise_ui_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_set_theme(ui: *mut DeniseUi, theme: u32) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        let Some(theme) = types::theme(theme) else {
            return DENISE_ERR_INVALID;
        };
        handle.ui.set_theme(theme);
        DENISE_OK
    })
}

/// Writes the surface size. Either output may be `NULL`.
///
/// # Safety
///
/// `ui` must be a live handle; `width` and `height` must be `NULL` or point at
/// writable `uint32_t`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_size(
    ui: *mut DeniseUi,
    width: *mut u32,
    height: *mut u32,
) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        let size = handle.ui.size();
        if !width.is_null() {
            // SAFETY: the caller promises a writable `uint32_t` when not null.
            unsafe { width.write(size.width) };
        }
        if !height.is_null() {
            // SAFETY: as above.
            unsafe { height.write(size.height) };
        }
        DENISE_OK
    })
}

/// Takes the next message, or returns `false` if there is none.
///
/// Never yields `0`; see the crate documentation for why.
///
/// # Safety
///
/// `ui` must be a live handle; `out` must be `NULL` or point at a writable
/// `uint32_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_poll_message(ui: *mut DeniseUi, out: *mut u32) -> bool {
    guard(false, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return false;
        };
        handle.collect();
        let Some(message) = handle.pending.pop_front() else {
            return false;
        };
        if !out.is_null() {
            // SAFETY: the caller promises a writable `uint32_t` when not null.
            unsafe { out.write(message) };
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_string_is_nul_terminated_and_matches_the_crate() {
        assert!(VERSION.ends_with('\0'));
        // SAFETY: `VERSION` is a static `&str` with exactly one NUL, at the end.
        let text = unsafe { CStr::from_ptr(denise_version()) };
        assert_eq!(text.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn every_status_has_its_own_message() {
        let codes = [
            DENISE_OK,
            DENISE_ERR_NULL,
            DENISE_ERR_INVALID,
            DENISE_ERR_NO_NODE,
            DENISE_ERR_BUFFER_TOO_SMALL,
            DENISE_ERR_WRONG_WIDGET,
            DENISE_ERR_PANIC,
        ];
        let mut seen: Vec<&str> = Vec::new();
        for code in codes {
            // SAFETY: `denise_status_message` returns a static C string.
            let text = unsafe { CStr::from_ptr(denise_status_message(code)) }
                .to_str()
                .unwrap();
            assert!(
                !seen.contains(&text),
                "{code} shares a message with another"
            );
            seen.push(text);
        }
        // SAFETY: as above.
        let unknown = unsafe { CStr::from_ptr(denise_status_message(-999)) };
        assert_eq!(unknown.to_str().unwrap(), "unknown status");
    }

    /// Every entry point must survive a `NULL` handle, because in a host that
    /// reaches Denise through three layers of marshalling it will get one.
    #[test]
    fn a_null_handle_is_refused_rather_than_dereferenced() {
        // SAFETY: passing null is exactly what the ABI documents as accepted.
        unsafe {
            denise_ui_free(std::ptr::null_mut());
            assert_eq!(
                denise_ui_set_theme(std::ptr::null_mut(), 0),
                DENISE_ERR_NULL
            );
            assert_eq!(
                denise_ui_size(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ),
                DENISE_ERR_NULL
            );
            assert!(!denise_ui_poll_message(
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ));
        }
    }

    #[test]
    fn a_bad_size_or_theme_produces_no_handle() {
        assert!(denise_ui_new(0, 480, DENISE_THEME_DARK).is_null());
        assert!(denise_ui_new(800, 0, DENISE_THEME_DARK).is_null());
        assert!(denise_ui_new(800, 480, 99).is_null());

        let ui = denise_ui_new(800, 480, DENISE_THEME_DARK);
        assert!(!ui.is_null());
        // SAFETY: `ui` came from `denise_ui_new` and is freed exactly once.
        unsafe { denise_ui_free(ui) };
    }
}
