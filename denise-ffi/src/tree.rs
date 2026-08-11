//! Building and mutating the widget tree from C.
//!
//! Every widget Denise ships gets one constructor here rather than a generic
//! "create widget of kind N" call. A host that passes the wrong argument to
//! `denise_ui_add_button` fails at its own compiler; one that passes the wrong
//! member of a tagged union fails at run time on a panel in a factory.

use std::ffi::c_char;

use denise_ui::widgets::{Button, Label, Panel, TextInput};
use denise_ui::{NodeId, Widget};

use crate::types::{DeniseRect, role};
use crate::{
    DENISE_ERR_INVALID, DENISE_ERR_NO_NODE, DENISE_ERR_NULL, DENISE_ERR_PANIC,
    DENISE_ERR_WRONG_WIDGET, DENISE_OK, DeniseUi, guard, handle, utf8,
};

/// The node id C uses for "no node": the parent of the root, the absence of
/// focus, the failure of a constructor.
pub const DENISE_NODE_NONE: u64 = 0;

/// Adds `widget` under `parent`, returning its id or [`DENISE_NODE_NONE`].
///
/// Factored out because all four constructors differ only in the widget.
///
/// # Safety
///
/// `ui` must be `NULL` or a live handle.
unsafe fn add(ui: *mut DeniseUi, parent: u64, layout: DeniseRect, widget: impl Widget<u32>) -> u64 {
    // SAFETY: forwarding the caller's promise about `ui` from the `extern` fn
    // that called this.
    let Some(handle) = (unsafe { handle(ui) }) else {
        return DENISE_NODE_NONE;
    };
    let parent = NodeId::from_ffi(parent);
    handle
        .ui
        .add(parent, widget, layout.into())
        .map_or(DENISE_NODE_NONE, NodeId::as_ffi)
}

/// The root of the base scene. Never [`DENISE_NODE_NONE`] for a live handle.
///
/// # Safety
///
/// `ui` must be a live handle from [`denise_ui_new`](crate::denise_ui_new).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_root(ui: *mut DeniseUi) -> u64 {
    guard(DENISE_NODE_NONE, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        match unsafe { handle(ui) } {
            Some(handle) => handle.ui.root().as_ffi(),
            None => DENISE_NODE_NONE,
        }
    })
}

/// The root of the topmost scene — the one input reaches. Equal to
/// [`denise_ui_root`] until a scene is pushed.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_top_root(ui: *mut DeniseUi) -> u64 {
    guard(DENISE_NODE_NONE, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        match unsafe { handle(ui) } {
            Some(handle) => handle.ui.top_root().as_ffi(),
            None => DENISE_NODE_NONE,
        }
    })
}

/// Adds a themed rectangle: the background other widgets sit on.
///
/// `fill` and `border` are role numbers, or `-1` for none. Panels are invisible
/// to hit testing, so putting a button on one does not cost the click.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_add_panel(
    ui: *mut DeniseUi,
    parent: u64,
    layout: DeniseRect,
    fill: i32,
    border: i32,
    border_width: i32,
) -> u64 {
    guard(DENISE_NODE_NONE, || {
        let panel = Panel {
            fill: role(fill),
            border: role(border),
            border_width,
            ..Panel::default()
        };
        // SAFETY: forwarding the caller's promise about `ui`.
        unsafe { add(ui, parent, layout, panel) }
    })
}

/// Adds static text in the role named by `role_value`.
///
/// # Safety
///
/// `ui` must be a live handle; `text` must be a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_add_label(
    ui: *mut DeniseUi,
    parent: u64,
    layout: DeniseRect,
    text: *const c_char,
    role_value: i32,
) -> u64 {
    guard(DENISE_NODE_NONE, || {
        // SAFETY: forwarding the caller's promise about `text`.
        let Some(text) = (unsafe { utf8(text) }) else {
            return DENISE_NODE_NONE;
        };
        let Some(role) = role(role_value) else {
            return DENISE_NODE_NONE;
        };
        // SAFETY: forwarding the caller's promise about `ui`.
        unsafe { add(ui, parent, layout, Label::new(text).with_role(role)) }
    })
}

/// Adds a button that emits `message` when activated.
///
/// A `message` of `0` makes the button inert: it still draws and still shows
/// press and hover states, and it emits nothing.
///
/// # Safety
///
/// `ui` must be a live handle; `label` must be a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_add_button(
    ui: *mut DeniseUi,
    parent: u64,
    layout: DeniseRect,
    label: *const c_char,
    message: u32,
    role_value: i32,
) -> u64 {
    guard(DENISE_NODE_NONE, || {
        // SAFETY: forwarding the caller's promise about `label`.
        let Some(label) = (unsafe { utf8(label) }) else {
            return DENISE_NODE_NONE;
        };
        let Some(role) = role(role_value) else {
            return DENISE_NODE_NONE;
        };
        let button = if message == 0 {
            Button::inert(label)
        } else {
            Button::new(label, message)
        };
        // SAFETY: forwarding the caller's promise about `ui`.
        unsafe { add(ui, parent, layout, button.with_role(role)) }
    })
}

/// Adds an editable single-line field.
///
/// `placeholder` may be `NULL` for none. `submit` is emitted on Enter, or `0` for
/// nothing. `max_chars` of `0` means unlimited. `password` draws bullets.
///
/// # Safety
///
/// `ui` must be a live handle; `placeholder` must be `NULL` or a NUL-terminated
/// UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_add_text_input(
    ui: *mut DeniseUi,
    parent: u64,
    layout: DeniseRect,
    placeholder: *const c_char,
    submit: u32,
    max_chars: u32,
    password: bool,
) -> u64 {
    guard(DENISE_NODE_NONE, || {
        let mut field = TextInput::<u32>::new().with_password(password);
        if !placeholder.is_null() {
            // SAFETY: forwarding the caller's promise about `placeholder`.
            let Some(text) = (unsafe { utf8(placeholder) }) else {
                return DENISE_NODE_NONE;
            };
            field = field.with_placeholder(text);
        }
        if submit != 0 {
            field = field.with_submit(submit);
        }
        if max_chars != 0 {
            field = field.with_max_chars(max_chars as usize);
        }
        // SAFETY: forwarding the caller's promise about `ui`.
        unsafe { add(ui, parent, layout, field) }
    })
}

/// Pushes a modal scene over everything below it, dimmed by `dim` (0 for none,
/// 255 for opaque). Returns the new scene's root.
///
/// Input only reaches the topmost scene, so nothing underneath is clickable,
/// focusable or reachable by Tab while it is open.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_push_scene(ui: *mut DeniseUi, dim: u8) -> u64 {
    guard(DENISE_NODE_NONE, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        match unsafe { handle(ui) } {
            Some(handle) => handle.ui.push_scene(dim).as_ffi(),
            None => DENISE_NODE_NONE,
        }
    })
}

/// Closes the topmost scene and everything in it. Returns `false` if only the
/// base scene is left, which is never removable.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_pop_scene(ui: *mut DeniseUi) -> bool {
    guard(false, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        match unsafe { handle(ui) } {
            Some(handle) => handle.ui.pop_scene(),
            None => false,
        }
    })
}

/// How many scenes are stacked. At least 1.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_scene_count(ui: *mut DeniseUi) -> u32 {
    guard(0, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        match unsafe { handle(ui) } {
            Some(handle) => handle.ui.scene_count() as u32,
            None => 0,
        }
    })
}

/// Removes a node and its subtree.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_remove(ui: *mut DeniseUi, node: u64) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        if handle.ui.remove(NodeId::from_ffi(node)) {
            DENISE_OK
        } else {
            DENISE_ERR_NO_NODE
        }
    })
}

/// Moves or resizes a node, relative to its parent's origin.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_set_layout(
    ui: *mut DeniseUi,
    node: u64,
    layout: DeniseRect,
) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        with_node(ui, node, |handle, id| {
            handle.ui.set_layout(id, layout.into());
            DENISE_OK
        })
    }
}

/// Writes a node's absolute bounds, already clipped by its ancestors.
///
/// # Safety
///
/// `ui` must be a live handle; `out` must point at a writable `DeniseRect`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_bounds(
    ui: *mut DeniseUi,
    node: u64,
    out: *mut DeniseRect,
) -> i32 {
    if out.is_null() {
        return DENISE_ERR_NULL;
    }
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        with_node(ui, node, |handle, id| match handle.ui.bounds(id) {
            // SAFETY (`out.write`): checked non-null above, and the caller
            // promises it is writable. Already inside the block above.
            Some(bounds) => {
                out.write(bounds.into());
                DENISE_OK
            }
            None => DENISE_ERR_NO_NODE,
        })
    }
}

/// Shows or hides a node and its subtree. A hidden node is not hit-tested and
/// cannot be focused.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_set_visible(ui: *mut DeniseUi, node: u64, visible: bool) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        with_node(ui, node, |handle, id| {
            handle.ui.set_visible(id, visible);
            DENISE_OK
        })
    }
}

/// Enables or disables a node. A disabled widget still draws, in its disabled
/// colours, and takes no input.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_set_enabled(ui: *mut DeniseUi, node: u64, enabled: bool) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        with_node(ui, node, |handle, id| {
            handle.ui.set_enabled(id, enabled);
            DENISE_OK
        })
    }
}

/// Sets a node's z-order within its parent. Higher draws later.
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_set_z(ui: *mut DeniseUi, node: u64, z: i32) -> i32 {
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        with_node(ui, node, |handle, id| {
            handle.ui.set_z(id, z);
            DENISE_OK
        })
    }
}

/// Gives keyboard focus to a node, or clears it with [`DENISE_NODE_NONE`].
///
/// # Safety
///
/// `ui` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_focus(ui: *mut DeniseUi, node: u64) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        if node == DENISE_NODE_NONE {
            handle.ui.focus(None);
            return DENISE_OK;
        }
        let id = NodeId::from_ffi(node);
        if !handle.ui.contains(id) {
            return DENISE_ERR_NO_NODE;
        }
        handle.ui.focus(Some(id));
        DENISE_OK
    })
}

/// Replaces the text of a label, a button's caption or a field's contents.
///
/// # Safety
///
/// `ui` must be a live handle; `text` must be a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_set_text(
    ui: *mut DeniseUi,
    node: u64,
    text: *const c_char,
) -> i32 {
    // SAFETY: forwarding the caller's promise about `text`.
    let Some(text) = (unsafe { utf8(text) }) else {
        return if text.is_null() {
            DENISE_ERR_NULL
        } else {
            DENISE_ERR_INVALID
        };
    };
    // SAFETY: forwarding the caller's promise about `ui`.
    unsafe {
        with_node(ui, node, |handle, id| {
            if let Some(label) = handle.ui.widget_mut::<Label>(id) {
                label.set_text(text);
                DENISE_OK
            } else if let Some(button) = handle.ui.widget_mut::<Button<u32>>(id) {
                button.set_label(text);
                DENISE_OK
            } else if let Some(field) = handle.ui.widget_mut::<TextInput<u32>>(id) {
                field.set_text(text);
                DENISE_OK
            } else {
                DENISE_ERR_WRONG_WIDGET
            }
        })
    }
}

/// Copies a widget's text into `out` as a NUL-terminated UTF-8 string.
///
/// Returns the length in bytes excluding the NUL, or a negative status. Call it
/// with `out` as `NULL` and `cap` as `0` to ask how much room the text needs; the
/// answer excludes the NUL, so allocate one more than it says.
///
/// # Safety
///
/// `ui` must be a live handle; `out` must be `NULL`, or writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn denise_ui_get_text(
    ui: *mut DeniseUi,
    node: u64,
    out: *mut c_char,
    cap: usize,
) -> isize {
    guard(DENISE_ERR_PANIC as isize, || {
        // SAFETY: forwarding the caller's promise about `ui`.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL as isize;
        };
        let id = NodeId::from_ffi(node);
        if !handle.ui.contains(id) {
            return DENISE_ERR_NO_NODE as isize;
        }

        let text = if let Some(label) = handle.ui.widget::<Label>(id) {
            label.text()
        } else if let Some(button) = handle.ui.widget::<Button<u32>>(id) {
            button.label()
        } else if let Some(field) = handle.ui.widget::<TextInput<u32>>(id) {
            field.text()
        } else {
            return DENISE_ERR_WRONG_WIDGET as isize;
        };

        let needed = text.len();
        if out.is_null() {
            // The measuring call. Not an error, however small `cap` is.
            return needed as isize;
        }
        if cap < needed + 1 {
            return DENISE_ERR_BUFFER_TOO_SMALL_ISIZE;
        }
        // SAFETY: `out` is non-null and the caller promises `cap` writable bytes,
        // which the check above proved is at least `needed + 1`. The source is a
        // `str` in the widget, which cannot overlap the caller's buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr().cast::<c_char>(), out, needed);
            out.add(needed).write(0);
        }
        needed as isize
    })
}

/// [`DENISE_ERR_BUFFER_TOO_SMALL`](crate::DENISE_ERR_BUFFER_TOO_SMALL) widened,
/// because the length-returning calls are `isize`.
const DENISE_ERR_BUFFER_TOO_SMALL_ISIZE: isize = crate::DENISE_ERR_BUFFER_TOO_SMALL as isize;

/// Resolves a handle and a node, then runs `body`. The shape of half this file.
///
/// # Safety
///
/// `ui` must be `NULL` or a live handle.
unsafe fn with_node(
    ui: *mut DeniseUi,
    node: u64,
    body: impl FnOnce(&mut DeniseUi, NodeId) -> i32,
) -> i32 {
    guard(DENISE_ERR_PANIC, || {
        // SAFETY: every caller is an `extern` fn whose own contract requires `ui`
        // to be null or a live handle.
        let Some(handle) = (unsafe { handle(ui) }) else {
            return DENISE_ERR_NULL;
        };
        let id = NodeId::from_ffi(node);
        if !handle.ui.contains(id) {
            return DENISE_ERR_NO_NODE;
        }
        body(handle, id)
    })
}
