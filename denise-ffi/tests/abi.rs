//! The ABI driven the way a C host drives it.
//!
//! Written against the exported symbols rather than the Rust behind them, and
//! deliberately using raw pointers and `char *` throughout: the point is to
//! exercise the crossing, not the widgets, which `denise-ui` already tests.

use std::ffi::{CStr, CString, c_char};
use std::ptr;

use denise_ffi::keys::{DENISE_KEY_A, DENISE_KEY_O};
use denise_ffi::*;

const W: u32 = 320;
const H: u32 = 200;
/// Deliberately not `W`. A host that assumes rows are contiguous works on a
/// desktop and shears on a pitch-aligned framebuffer, so every paint here is
/// through a padded stride and the padding is checked.
const STRIDE: u32 = W + 7;

/// A `DeniseUi` that frees itself, so a failing assertion does not leak into the
/// next test under the same process.
struct Host(*mut DeniseUi);

impl Host {
    fn new() -> Self {
        let ui = denise_ui_new(W, H, DENISE_THEME_DARK);
        assert!(!ui.is_null(), "denise_ui_new returned NULL");
        Self(ui)
    }

    fn ptr(&self) -> *mut DeniseUi {
        self.0
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `denise_ui_new` and is freed exactly once.
        unsafe { denise_ui_free(self.0) };
    }
}

fn c(text: &str) -> CString {
    CString::new(text).expect("no interior NUL")
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> DeniseRect {
    DeniseRect {
        x,
        y,
        width,
        height,
    }
}

/// Clicks at a point, as a host delivers it: down then up, both with a position.
///
/// # Safety
///
/// `ui` must be a live handle.
unsafe fn click(ui: *mut DeniseUi, x: i32, y: i32) {
    // SAFETY: forwarding the caller's promise.
    unsafe {
        assert_eq!(denise_ui_pointer_moved(ui, x, y), DENISE_OK);
        assert_eq!(
            denise_ui_pointer_button(ui, DENISE_BUTTON_LEFT, true, x, y, 0),
            DENISE_OK
        );
        assert_eq!(
            denise_ui_pointer_button(ui, DENISE_BUTTON_LEFT, false, x, y, 0),
            DENISE_OK
        );
    }
}

/// Reads a widget's text back through the two-call protocol the header
/// documents: measure with a `NULL` buffer, then fill.
///
/// # Safety
///
/// `ui` must be a live handle.
unsafe fn text_of(ui: *mut DeniseUi, node: u64) -> String {
    // SAFETY: forwarding the caller's promise.
    let needed = unsafe { denise_ui_get_text(ui, node, ptr::null_mut(), 0) };
    assert!(needed >= 0, "measuring failed with {needed}");
    let mut buffer = vec![0 as c_char; needed as usize + 1];
    // SAFETY: `buffer` is writable for exactly the length being passed.
    let written = unsafe { denise_ui_get_text(ui, node, buffer.as_mut_ptr().cast(), buffer.len()) };
    assert_eq!(written, needed, "the two calls disagreed about the length");
    // SAFETY: the ABI promises a NUL-terminated string within `buffer`.
    unsafe { CStr::from_ptr(buffer.as_ptr().cast()) }
        .to_str()
        .expect("UTF-8 back out")
        .to_owned()
}

#[test]
fn a_button_press_produces_its_message() {
    let host = Host::new();
    // SAFETY: `host` is live for the whole test.
    unsafe {
        let root = denise_ui_root(host.ptr());
        assert_ne!(root, DENISE_NODE_NONE);

        let save = denise_ui_add_button(
            host.ptr(),
            root,
            rect(20, 20, 160, 44),
            c("Lagre").as_ptr(),
            7,
            DENISE_ROLE_PRIMARY,
        );
        assert_ne!(save, DENISE_NODE_NONE);

        click(host.ptr(), 100, 42);

        let mut message = 0u32;
        assert!(denise_ui_poll_message(host.ptr(), &mut message));
        assert_eq!(message, 7);
        assert!(
            !denise_ui_poll_message(host.ptr(), &mut message),
            "one press produced more than one message"
        );
    }
}

/// The rule that lets a widget exist without a message at all.
#[test]
fn a_button_with_message_zero_stays_silent() {
    let host = Host::new();
    // SAFETY: `host` is live for the whole test.
    unsafe {
        let root = denise_ui_root(host.ptr());
        let inert = denise_ui_add_button(
            host.ptr(),
            root,
            rect(20, 20, 160, 44),
            c("Nothing").as_ptr(),
            0,
            DENISE_ROLE_NEUTRAL,
        );
        assert_ne!(inert, DENISE_NODE_NONE, "an inert button still exists");

        click(host.ptr(), 100, 42);
        assert!(!denise_ui_poll_message(host.ptr(), ptr::null_mut()));
    }
}

#[test]
fn typing_reaches_a_focused_field_and_comes_back_as_utf8() {
    let host = Host::new();
    // SAFETY: `host` is live for the whole test.
    unsafe {
        let root = denise_ui_root(host.ptr());
        let field = denise_ui_add_text_input(
            host.ptr(),
            root,
            rect(20, 20, 260, 40),
            c("Navn").as_ptr(),
            0,
            0,
            false,
        );
        assert_eq!(denise_ui_focus(host.ptr(), field), DENISE_OK);

        // A key and its character, the way the header says to send them: the
        // position first, then what the layout finally committed. 'æ' is not on
        // any US position, which is the point.
        for (key, ch) in [
            (DENISE_KEY_A, 'æ'),
            (DENISE_KEY_O, 'ø'),
            (DENISE_KEY_A, 'å'),
        ] {
            assert_eq!(denise_ui_key(host.ptr(), key, true, false, 0), DENISE_OK);
            assert_eq!(denise_ui_text(host.ptr(), ch as u32), DENISE_OK);
            assert_eq!(denise_ui_key(host.ptr(), key, false, false, 0), DENISE_OK);
        }

        assert_eq!(text_of(host.ptr(), field), "æøå");

        // Six bytes for three characters. A host that sized the buffer by
        // counting characters would be one short of the NUL, and this is where
        // it finds out.
        assert_eq!(denise_ui_get_text(host.ptr(), field, ptr::null_mut(), 0), 6);
        let mut small = [0 as c_char; 6];
        assert_eq!(
            denise_ui_get_text(host.ptr(), field, small.as_mut_ptr(), small.len()),
            DENISE_ERR_BUFFER_TOO_SMALL as isize,
            "a buffer with no room for the NUL must be refused"
        );
    }
}

/// Control characters are keys, never text. A host that forwards `WM_CHAR`
/// wholesale sends `\r` for Enter, and a field that accepted it would hold a
/// character it can never draw.
#[test]
fn control_characters_are_refused_as_text() {
    let host = Host::new();
    // SAFETY: `host` is live for the whole test.
    unsafe {
        assert_eq!(denise_ui_text(host.ptr(), '\r' as u32), DENISE_ERR_INVALID);
        assert_eq!(denise_ui_text(host.ptr(), '\t' as u32), DENISE_ERR_INVALID);
        assert_eq!(denise_ui_text(host.ptr(), 0x08), DENISE_ERR_INVALID);
        // A lone surrogate is not a scalar value and never becomes one.
        assert_eq!(denise_ui_text(host.ptr(), 0xD800), DENISE_ERR_INVALID);
        assert_eq!(denise_ui_text(host.ptr(), 'ø' as u32), DENISE_OK);
    }
}

#[test]
fn painting_writes_pixels_inside_the_damage_and_never_the_padding() {
    let host = Host::new();
    let mut pixels = vec![0u32; (STRIDE * H) as usize];

    // SAFETY: `host` is live, and `pixels` outlives every call below.
    unsafe {
        let root = denise_ui_root(host.ptr());
        denise_ui_add_panel(
            host.ptr(),
            root,
            rect(10, 10, 100, 60),
            DENISE_ROLE_PRIMARY,
            DENISE_ROLE_NONE,
            0,
        );

        assert!(denise_ui_needs_paint(host.ptr()), "a new tree is dirty");

        let frame = DeniseFrame {
            pixels: pixels.as_mut_ptr(),
            len: pixels.len(),
            width: W,
            height: H,
            stride: STRIDE,
            format: DENISE_FORMAT_XRGB8888,
            // The first frame of any surface. Undefined contents, full repaint.
            buffer_age: -1,
        };
        assert_eq!(denise_ui_paint(host.ptr(), &frame), DENISE_OK);

        let count = denise_ui_damage(host.ptr(), ptr::null_mut(), 0);
        assert!(count > 0, "a painted frame reported no damage");
        let mut rects = [rect(0, 0, 0, 0); 16];
        assert_eq!(denise_ui_damage(host.ptr(), rects.as_mut_ptr(), 16), count);

        assert_eq!(denise_ui_presented(host.ptr()), DENISE_OK);
        assert!(
            !denise_ui_needs_paint(host.ptr()),
            "damage survived being presented"
        );
    }

    // The panel is drawn, so the buffer is not still zero.
    let inside = pixels[(30 * STRIDE + 30) as usize];
    assert_ne!(inside, 0, "nothing was drawn where the panel is");

    // And the columns past `width` belong to the host, not to Denise.
    for row in 0..H {
        for column in W..STRIDE {
            assert_eq!(
                pixels[(row * STRIDE + column) as usize],
                0,
                "row {row} column {column} is past the visible width"
            );
        }
    }
}

/// The one call with a real memory-safety obligation, and the one place a host
/// can get it wrong by arithmetic rather than by carelessness.
#[test]
fn a_frame_that_lies_about_its_buffer_is_refused() {
    let host = Host::new();
    let mut pixels = vec![0u32; (STRIDE * H) as usize];

    let honest = DeniseFrame {
        pixels: pixels.as_mut_ptr(),
        len: pixels.len(),
        width: W,
        height: H,
        stride: STRIDE,
        format: DENISE_FORMAT_XRGB8888,
        buffer_age: -1,
    };

    // SAFETY: `host` is live and every descriptor below points at `pixels`,
    // which is large enough for the honest one; the rest are rejected before the
    // buffer is touched, which is what is being tested.
    unsafe {
        // The buffer has to reach the last *visible* pixel, not the end of the
        // last row: the padding after it is the host's and Denise never touches
        // it. Both sides of that boundary, because getting it wrong in the
        // generous direction reads past the allocation.
        let required = (STRIDE * (H - 1) + W) as usize;
        let mut exact = honest;
        exact.len = required;
        assert_eq!(denise_ui_paint(host.ptr(), &exact), DENISE_OK);

        let mut short = honest;
        short.len = required - 1;
        assert_eq!(
            denise_ui_paint(host.ptr(), &short),
            DENISE_ERR_BUFFER_TOO_SMALL
        );

        let mut narrow = honest;
        narrow.stride = W - 1;
        assert_eq!(denise_ui_paint(host.ptr(), &narrow), DENISE_ERR_INVALID);

        let mut empty = honest;
        empty.height = 0;
        assert_eq!(denise_ui_paint(host.ptr(), &empty), DENISE_ERR_INVALID);

        let mut unknown = honest;
        unknown.format = 99;
        assert_eq!(denise_ui_paint(host.ptr(), &unknown), DENISE_ERR_INVALID);

        let mut nowhere = honest;
        nowhere.pixels = ptr::null_mut();
        assert_eq!(denise_ui_paint(host.ptr(), &nowhere), DENISE_ERR_NULL);

        assert_eq!(denise_ui_paint(host.ptr(), ptr::null()), DENISE_ERR_NULL);
        assert_eq!(denise_ui_paint(host.ptr(), &honest), DENISE_OK);
    }
}

/// Generational ids are the reason a C host can hold node handles at all.
#[test]
fn an_id_kept_past_a_remove_resolves_to_nothing() {
    let host = Host::new();
    // SAFETY: `host` is live for the whole test.
    unsafe {
        let root = denise_ui_root(host.ptr());
        let stale = denise_ui_add_label(
            host.ptr(),
            root,
            rect(0, 0, 80, 20),
            c("gone").as_ptr(),
            DENISE_ROLE_BASE_CONTENT,
        );
        assert_eq!(denise_ui_remove(host.ptr(), stale), DENISE_OK);

        // The slot is reused by the next node, and the old id must not follow it.
        let fresh = denise_ui_add_label(
            host.ptr(),
            root,
            rect(0, 0, 80, 20),
            c("here").as_ptr(),
            DENISE_ROLE_BASE_CONTENT,
        );
        assert_ne!(fresh, stale);

        assert_eq!(denise_ui_remove(host.ptr(), stale), DENISE_ERR_NO_NODE);
        assert_eq!(
            denise_ui_set_visible(host.ptr(), stale, false),
            DENISE_ERR_NO_NODE
        );
        assert_eq!(
            denise_ui_get_text(host.ptr(), stale, ptr::null_mut(), 0),
            DENISE_ERR_NO_NODE as isize
        );
        assert_eq!(text_of(host.ptr(), fresh), "here");
    }
}

#[test]
fn a_modal_scene_takes_every_click_from_what_is_under_it() {
    let host = Host::new();
    // SAFETY: `host` is live for the whole test.
    unsafe {
        let root = denise_ui_root(host.ptr());
        denise_ui_add_button(
            host.ptr(),
            root,
            rect(0, 0, W as i32, H as i32),
            c("Behind").as_ptr(),
            1,
            DENISE_ROLE_NEUTRAL,
        );

        assert_eq!(denise_ui_scene_count(host.ptr()), 1);
        let dialog = denise_ui_push_scene(host.ptr(), 110);
        assert_ne!(dialog, DENISE_NODE_NONE);
        assert_eq!(denise_ui_scene_count(host.ptr()), 2);
        assert_eq!(denise_ui_top_root(host.ptr()), dialog);

        denise_ui_add_button(
            host.ptr(),
            dialog,
            rect(60, 60, 120, 40),
            c("OK").as_ptr(),
            2,
            DENISE_ROLE_PRIMARY,
        );

        // Well away from the dialog, squarely on the button underneath it.
        click(host.ptr(), 10, 180);
        assert!(
            !denise_ui_poll_message(host.ptr(), ptr::null_mut()),
            "a click reached a widget beneath a modal scene"
        );

        click(host.ptr(), 100, 80);
        let mut message = 0u32;
        assert!(denise_ui_poll_message(host.ptr(), &mut message));
        assert_eq!(message, 2);

        assert!(denise_ui_pop_scene(host.ptr()));
        assert_eq!(denise_ui_scene_count(host.ptr()), 1);
        assert!(
            !denise_ui_pop_scene(host.ptr()),
            "the base scene must not be removable"
        );

        // And now the same click lands.
        click(host.ptr(), 10, 180);
        assert!(denise_ui_poll_message(host.ptr(), &mut message));
        assert_eq!(message, 1);
    }
}

#[test]
fn the_library_reports_a_version_a_host_can_check() {
    assert_eq!(denise_abi_version(), DENISE_ABI_VERSION);
    // SAFETY: both return static NUL-terminated strings and never NULL.
    unsafe {
        assert!(!CStr::from_ptr(denise_version()).to_bytes().is_empty());
        assert_eq!(
            CStr::from_ptr(denise_status_message(DENISE_OK))
                .to_str()
                .unwrap(),
            "ok"
        );
    }
}
