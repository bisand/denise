//! Tying a secondary window to the one that opened it, on the platforms that
//! have an opinion about what that means.
//!
//! Three platforms, three different amounts of help, and the difference is worth
//! knowing before relying on any of it:
//!
//! - **Windows does all of it.** An owned window is always above its owner in the
//!   z-order, hidden when the owner is minimised and destroyed when the owner is
//!   destroyed — and disabling the owner ([`set_enabled`]) is how Win32 has always
//!   built a modal dialog box. This is the real thing, enforced by the window
//!   manager rather than by us.
//! - **macOS has the z-order half.** `-[NSWindow addChildWindow:ordered:]` keeps
//!   the child above its parent and takes it along when the parent moves or
//!   minimises. There is no "disable this window": true application modality is
//!   `-[NSApplication runModalForWindow:]`, which runs a nested run loop of its
//!   own and would fight winit's for control of the process.
//! - **Linux gets nothing.** Neither X11 nor Wayland is reachable from here —
//!   winit exposes no `WM_TRANSIENT_FOR`, no `_NET_WM_STATE_MODAL` and no
//!   `xdg_toplevel.set_parent` — so both functions are no-ops and a secondary
//!   window is an ordinary top-level one the window manager may place wherever it
//!   likes.
//!
//! Which is why modality is **not** built on any of this. The runner blocks input
//! to a modal window's owner itself, on every platform, and these calls are the
//! layer that makes it look native where the platform can. Deleting this whole
//! module would cost appearance and no correctness.
//!
//! [`WindowAttributes::with_parent_window`] is deliberately not used: on both
//! Windows and X11 a *parent* window is a child control clipped to its parent's
//! client area, which is an embedded panel and not a dialog.
//!
//! [`WindowAttributes::with_parent_window`]: winit::window::WindowAttributes::with_parent_window

use winit::window::{Window, WindowAttributes};

/// Declares `owner` the owner of the window these attributes will create.
///
/// Applied at creation because that is when Windows wants it; platforms that tie
/// windows together afterwards do it in [`adopt`] instead.
#[cfg_attr(not(target_os = "windows"), expect(unused_variables))]
pub fn own(attrs: WindowAttributes, owner: &Window) -> WindowAttributes {
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        if let Ok(handle) = owner.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            return attrs.with_owner_window(handle.hwnd.get());
        }
    }
    attrs
}

/// Ties an already-created `child` to `owner`, for platforms that cannot say it
/// at creation.
#[cfg_attr(not(target_os = "macos"), expect(unused_variables))]
pub fn adopt(owner: &Window, child: &Window) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWindowOrderingMode;

        if let (Some(owner), Some(child)) = (ns_window(owner), ns_window(child)) {
            // SAFETY: both windows are alive — the runner holds them — and this is
            // the main thread, which is the only thread winit runs its loop on.
            unsafe { owner.addChildWindow_ordered(&child, NSWindowOrderingMode::Above) };
        }
    }
}

/// Whether `window` accepts input from the user.
///
/// Windows only, where it is half of what makes a dialog modal. Everywhere else
/// the runner's own input blocking is the whole of it.
#[cfg_attr(not(target_os = "windows"), expect(unused_variables))]
pub fn set_enabled(window: &Window, enabled: bool) {
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowExtWindows;
        window.set_enable(enabled);
    }
}

/// The `NSWindow` behind a winit window.
#[cfg(target_os = "macos")]
fn ns_window(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit promises the handle points at a live `NSView` for as long as
    // the window is alive, and the caller holds the window across this call.
    let view: &NSView = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    view.window()
}
