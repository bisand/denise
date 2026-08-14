//! A C host, played badly, against the whole `denise-ffi` surface.
//!
//! Every entry point takes pointers, node ids and enum values from a host that
//! may be wrong about all three. `guard` turns a panic into a status code, and
//! null is checked everywhere — this looks for the argument the checks miss.
//!
//! What the fuzzer supplies is a *sequence*, because that is where the
//! interesting bugs are: a node removed and then written to, a scene popped
//! twice, a layout set on an id from a tree that no longer exists. Single calls
//! with silly arguments are what the unit tests already cover.
//!
//! Worth running under Miri's eye as well: the corpus this builds is exactly the
//! call sequence a soundness check wants.

#![no_main]

use arbitrary::Arbitrary;
use denise_ffi::*;
use libfuzzer_sys::fuzz_target;

/// One thing a host can do. Ids are indices into what we have created, so the
/// fuzzer reaches real nodes often rather than only invalid ones — but `%` keeps
/// stale and out-of-range ids reachable too, which is the point.
#[derive(Arbitrary, Debug)]
enum Op {
    AddPanel {
        parent: u8,
        x: i16,
        y: i16,
        w: i16,
        h: i16,
    },
    AddLabel {
        parent: u8,
        x: i16,
        y: i16,
        w: i16,
        h: i16,
        text: String,
    },
    AddButton {
        parent: u8,
        x: i16,
        y: i16,
        w: i16,
        h: i16,
        text: String,
    },
    AddTextInput {
        parent: u8,
        x: i16,
        y: i16,
        w: i16,
        h: i16,
    },
    SetLayout {
        node: u8,
        x: i16,
        y: i16,
        w: i16,
        h: i16,
    },
    SetText {
        node: u8,
        text: String,
    },
    SetVisible {
        node: u8,
        visible: bool,
    },
    SetEnabled {
        node: u8,
        enabled: bool,
    },
    SetZ {
        node: u8,
        z: i32,
    },
    Focus {
        node: u8,
    },
    Remove {
        node: u8,
    },
    Invalidate {
        node: u8,
    },
    InvalidateAll,
    PushScene {
        dim: u8,
    },
    PopScene,
    PointerMoved {
        x: i32,
        y: i32,
    },
    PointerButton {
        button: u32,
        down: bool,
        x: i32,
        y: i32,
        modifiers: u32,
    },
    PointerScroll {
        dx: f32,
        dy: f32,
        x: i32,
        y: i32,
    },
    PointerLeft,
    Key {
        key: u32,
        down: bool,
        repeat: bool,
        modifiers: u32,
    },
    Text {
        codepoint: u32,
    },
    Touch {
        id: u64,
        phase: u32,
        x: i32,
        y: i32,
    },
    Tick {
        now_ms: u64,
    },
    SetTheme {
        theme: u32,
    },
    ShowCursor {
        visible: bool,
    },
    PollMessage,
    Paint,
    Presented,
}

#[derive(Arbitrary, Debug)]
struct Session {
    width: u8,
    height: u8,
    theme: u8,
    ops: Vec<Op>,
}

fuzz_target!(|session: Session| {
    // A surface small enough that painting every frame stays cheap, and never
    // zero — `denise_ui_new` rejects that, and a null handle would make every
    // op below a no-op and waste the run.
    let width = 1 + u32::from(session.width) % 64;
    let height = 1 + u32::from(session.height) % 64;

    let ui = denise_ui_new(width, height, u32::from(session.theme) % 4);
    if ui.is_null() {
        return;
    }

    // SAFETY: `ui` came from `denise_ui_new` above, is freed exactly once at the
    // end, and nothing else touches it — the same contract a C host signs.
    unsafe {
        let mut nodes = vec![denise_ui_root(ui)];
        let mut pixels = vec![0u32; (width * height) as usize];

        // An id the fuzzer picked: usually one we made, sometimes stale, and
        // occasionally never valid at all.
        let pick = |nodes: &Vec<u64>, n: u8| -> u64 {
            if nodes.is_empty() {
                return u64::from(n);
            }
            match n {
                0..=239 => nodes[usize::from(n) % nodes.len()],
                other => u64::from(other),
            }
        };
        let rect = |x: i16, y: i16, w: i16, h: i16| DeniseRect {
            x: i32::from(x),
            y: i32::from(y),
            width: i32::from(w),
            height: i32::from(h),
        };
        // Every string reaches the ABI as the NUL-terminated bytes a C caller
        // would pass; an interior NUL simply ends it early, as it would there.
        let cstring = |s: &str| {
            let mut bytes: Vec<u8> = s.bytes().filter(|&b| b != 0).collect();
            bytes.push(0);
            bytes
        };

        for op in &session.ops {
            match op {
                Op::AddPanel { parent, x, y, w, h } => {
                    let id = denise_ui_add_panel(
                        ui,
                        pick(&nodes, *parent),
                        rect(*x, *y, *w, *h),
                        -1,
                        -1,
                        0,
                    );
                    if id != DENISE_NODE_NONE {
                        nodes.push(id);
                    }
                }
                Op::AddLabel {
                    parent,
                    x,
                    y,
                    w,
                    h,
                    text,
                } => {
                    let text = cstring(text);
                    let id = denise_ui_add_label(
                        ui,
                        pick(&nodes, *parent),
                        rect(*x, *y, *w, *h),
                        text.as_ptr().cast(),
                        -1,
                    );
                    if id != DENISE_NODE_NONE {
                        nodes.push(id);
                    }
                }
                Op::AddButton {
                    parent,
                    x,
                    y,
                    w,
                    h,
                    text,
                } => {
                    let text = cstring(text);
                    let id = denise_ui_add_button(
                        ui,
                        pick(&nodes, *parent),
                        rect(*x, *y, *w, *h),
                        text.as_ptr().cast(),
                        0,
                        -1,
                    );
                    if id != DENISE_NODE_NONE {
                        nodes.push(id);
                    }
                }
                Op::AddTextInput { parent, x, y, w, h } => {
                    let id = denise_ui_add_text_input(
                        ui,
                        pick(&nodes, *parent),
                        rect(*x, *y, *w, *h),
                        core::ptr::null(),
                        0,
                        0,
                        false,
                    );
                    if id != DENISE_NODE_NONE {
                        nodes.push(id);
                    }
                }
                Op::SetLayout { node, x, y, w, h } => {
                    denise_ui_set_layout(ui, pick(&nodes, *node), rect(*x, *y, *w, *h));
                }
                Op::SetText { node, text } => {
                    let text = cstring(text);
                    denise_ui_set_text(ui, pick(&nodes, *node), text.as_ptr().cast());
                }
                Op::SetVisible { node, visible } => {
                    denise_ui_set_visible(ui, pick(&nodes, *node), *visible);
                }
                Op::SetEnabled { node, enabled } => {
                    denise_ui_set_enabled(ui, pick(&nodes, *node), *enabled);
                }
                Op::SetZ { node, z } => {
                    denise_ui_set_z(ui, pick(&nodes, *node), *z);
                }
                Op::Focus { node } => {
                    denise_ui_focus(ui, pick(&nodes, *node));
                }
                Op::Remove { node } => {
                    // Left in `nodes` on purpose: a host that keeps using an id
                    // after removing it is the case worth exercising.
                    denise_ui_remove(ui, pick(&nodes, *node));
                }
                Op::Invalidate { node } => {
                    denise_ui_invalidate(ui, pick(&nodes, *node));
                }
                Op::InvalidateAll => {
                    denise_ui_invalidate_all(ui);
                }
                Op::PushScene { dim } => {
                    denise_ui_push_scene(ui, *dim);
                }
                Op::PopScene => {
                    denise_ui_pop_scene(ui);
                }
                Op::PointerMoved { x, y } => {
                    denise_ui_pointer_moved(ui, *x, *y);
                }
                Op::PointerButton {
                    button,
                    down,
                    x,
                    y,
                    modifiers,
                } => {
                    denise_ui_pointer_button(ui, *button, *down, *x, *y, *modifiers);
                }
                Op::PointerScroll { dx, dy, x, y } => {
                    denise_ui_pointer_scroll(ui, *dx, *dy, *x, *y);
                }
                Op::PointerLeft => {
                    denise_ui_pointer_left(ui);
                }
                Op::Key {
                    key,
                    down,
                    repeat,
                    modifiers,
                } => {
                    denise_ui_key(ui, *key, *down, *repeat, *modifiers);
                }
                Op::Text { codepoint } => {
                    denise_ui_text(ui, *codepoint);
                }
                Op::Touch { id, phase, x, y } => {
                    denise_ui_touch(ui, *id, *phase, *x, *y);
                }
                Op::Tick { now_ms } => {
                    denise_ui_tick(ui, *now_ms);
                }
                Op::SetTheme { theme } => {
                    denise_ui_set_theme(ui, *theme);
                }
                Op::ShowCursor { visible } => {
                    denise_ui_show_cursor(ui, *visible);
                }
                Op::PollMessage => {
                    let mut out = 0u32;
                    denise_ui_poll_message(ui, &mut out);
                }
                Op::Paint => {
                    let frame = DeniseFrame {
                        pixels: pixels.as_mut_ptr(),
                        len: pixels.len(),
                        width,
                        height,
                        stride: width,
                        format: DENISE_FORMAT_XRGB8888,
                        buffer_age: 1,
                    };
                    denise_ui_paint(ui, &frame);
                }
                Op::Presented => {
                    denise_ui_presented(ui);
                }
            }
        }

        denise_ui_free(ui);
    }
});
