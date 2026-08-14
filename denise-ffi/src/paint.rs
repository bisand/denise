//! Drawing into a buffer the host owns.
//!
//! The one place a raw pointer becomes a slice, and the only call in the ABI with
//! a genuine memory-safety obligation on the host. Everything else can be got
//! wrong and produce a status code; get [`DeniseFrame`] wrong and Denise writes
//! where it was told to.

use denise::{BufferAge, Frame, Size};

use crate::types::{DeniseFrame, DeniseRect, pixel_format};
use crate::{
    DENISE_ERR_BUFFER_TOO_SMALL, DENISE_ERR_INVALID, DENISE_ERR_NO_NODE, DENISE_ERR_NULL,
    DENISE_ERR_PANIC, DENISE_OK, DeniseUi, guard, handle,
};

/// Whether anything has been marked dirty since the last
/// [`denise_ui_presented`].
///
/// A host that ignores this and paints unconditionally still gets correct pixels;
/// it just pays for them. On a panel showing an unchanging screen, honouring it is
/// the difference between an idle CPU and a busy one.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_needs_paint(ui: *mut DeniseUi) -> bool {
    guard(false, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        match unsafe { handle(ui) } {
            Some(handle) => handle.ui.needs_paint(),
            None => false,
        }
    })
}

/// Draws every damaged region into the buffer `frame` describes.
///
/// After this, [`denise_ui_damage`] lists what was drawn — blit exactly those
/// rectangles — and [`denise_ui_presented`] retires them.
///
/// # Safety
///
/// `ui` must be a live handle. `frame` must point at a readable [`DeniseFrame`]
/// whose `pixels` is valid and writable for `len` words for the duration of the
/// call, and which nothing else is reading or writing meanwhile.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_paint(ui: *mut DeniseUi, frame: *const DeniseFrame) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        if frame.is_null() {
            return DENISE_ERR_NULL;
        }
        // SAFETY: the caller promises `frame` points at a readable `DeniseFrame`.
        // It is `Copy` and holds no owned memory, so reading it out lets the
        // borrow of the descriptor end before the buffer is touched.
        let description = unsafe { frame.read() };

        if description.pixels.is_null() {
            return DENISE_ERR_NULL;
        }
        let Some(format) = pixel_format(description.format) else {
            return DENISE_ERR_INVALID;
        };

        // Checked here rather than left to `Frame::new` because `from_raw_parts_mut`
        // comes first, and a `len` that lies about the allocation is the one thing
        // no later check can recover from.
        let size = Size::new(description.width, description.height);
        if size.is_empty() || description.stride < description.width {
            return DENISE_ERR_INVALID;
        }
        // In `u64`, because `len` is the host's word and this is the check that
        // decides whether to trust it. See `denise::required_words`.
        let required = denise::required_words(size, description.stride);
        if (description.len as u64) < required {
            return DENISE_ERR_BUFFER_TOO_SMALL;
        }

        let age = if description.buffer_age < 0 {
            BufferAge::Undefined
        } else {
            BufferAge::Frames(description.buffer_age as u32)
        };

        // SAFETY: the caller promises `pixels` is valid and writable for `len`
        // words and unaliased for the call. `len` was just checked to cover every
        // word `Frame` can address given this size and stride.
        let pixels = unsafe { std::slice::from_raw_parts_mut(description.pixels, description.len) };

        let Ok(mut frame) = Frame::new(pixels, size, description.stride, format, age) else {
            return DENISE_ERR_INVALID;
        };
        handle.ui.paint(&mut frame);
        DENISE_OK
    })
}

/// Lists the rectangles [`denise_ui_paint`] last drew.
///
/// Returns how many there are, which may exceed `cap`; the first `cap` are
/// written. Pass `NULL` and `0` to ask the count. A host that blits the whole
/// surface instead is correct and slower — on Win32 or X11 measurably so, on a
/// DRM page flip not at all.
///
/// # Safety
///
/// `ui` must be a live handle; `out` must be `NULL`, or writable for `cap`
/// [`DeniseRect`]s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_damage(
    ui: *mut DeniseUi,
    out: *mut DeniseRect,
    cap: usize,
) -> isize {
    guard(DENISE_ERR_PANIC as isize, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL as isize;
        };
        let damage = handle.ui.damage();
        if !out.is_null() {
            for (index, rect) in damage.iter().take(cap).enumerate() {
                // SAFETY: the caller promises `out` is writable for `cap`
                // rectangles, and `take(cap)` keeps `index` below it.
                unsafe { out.add(index).write((*rect).into()) };
            }
        }
        damage.len() as isize
    })
}

/// Retires this frame's damage. Call after the blit, not before: what it forgets
/// is exactly what a failed present would need to draw again.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_presented(ui: *mut DeniseUi) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        handle.ui.presented();
        DENISE_OK
    })
}

/// Marks the whole surface for repaint. What a host calls when its window was
/// resized, uncovered, or restored from a state Denise cannot see.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_invalidate_all(ui: *mut DeniseUi) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        handle.ui.invalidate_all();
        DENISE_OK
    })
}

/// Marks one node's rectangle for repaint.
///
/// Rarely needed: everything that changes widget state through this ABI
/// invalidates on the way in. It is here for a host that has drawn over Denise's
/// buffer itself.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_invalidate(ui: *mut DeniseUi, node: u64) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        let id = denise_ui::NodeId::from_ffi(node);
        if !handle.ui.contains(id) {
            return DENISE_ERR_NO_NODE;
        }
        handle.ui.invalidate(id);
        DENISE_OK
    })
}
