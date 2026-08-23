//! What the windows have to agree about.
//!
//! Every secondary window in this example is built by the application and handed
//! an `Rc<RefCell<Shared>>` before it opens. That is the entire mechanism for
//! talking to a form, and it is deliberately not something the backend provides:
//! `denise-winit` gives a form a window, a surface and a place in the event loop,
//! and has no opinion about what the form is *for*.

use std::time::Duration;

/// How often a window that is watching another window's edits looks again.
///
/// A window repaints what it changed; nothing tells it that a *different* window
/// changed something it displays, and there is deliberately no mechanism in the
/// backend for one window to invalidate another — that would be a handle to
/// somebody else's tree, which is the thing this design does not have.
///
/// So the watching window polls, and 20 Hz is imperceptible for a settings edit.
/// What it costs is one `update` per interval with no damage, which the tracker
/// turns into no paint, no present and no frame — the same nothing an idle window
/// costs already. A window that watches nothing (the modal here) does not do this
/// and still sleeps until it is spoken to.
pub const WATCH: Duration = Duration::from_millis(50);

/// State the main window displays and the settings form edits.
#[derive(Debug)]
pub struct Shared {
    /// Shown as the main window's heading, and edited in the settings form.
    pub title: String,
    /// Whether the main window shows its second line.
    pub subtitle: bool,
    /// 0 to 100. Nothing dims; it is here to be a value that changes often, which
    /// is what a slider is for.
    pub brightness: f32,
    /// Whether the record is still there. The modal is what deletes it.
    pub record: bool,

    /// Whether the settings form is open.
    ///
    /// [`DeniseApp::take_windows`] opens a window every time it is handed a
    /// request, so "only one settings form" is a rule the application keeps —
    /// which it has to anyway, to know what the second click on the button should
    /// do.
    ///
    /// [`DeniseApp::take_windows`]: denise_winit::DeniseApp::take_windows
    pub settings_open: bool,
    /// Raised by the main window to close the settings form from outside it.
    ///
    /// The form's `exit_requested` reads this. There is no handle to a window
    /// somebody else opened and no call that closes one — a form closes itself,
    /// and this is how it is asked to.
    pub settings_should_close: bool,

    /// Bumped by anything that changes what another window is showing.
    ///
    /// A window repaints what *it* changed, so a window changing somebody else's
    /// state has to leave a mark the other one will notice. A counter is enough,
    /// and it is cheaper than comparing the state itself.
    pub revision: u64,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            title: "Ada Lovelace".into(),
            subtitle: true,
            brightness: 70.0,
            record: true,
            settings_open: false,
            settings_should_close: false,
            revision: 0,
        }
    }
}

impl Shared {
    /// Applies a change and marks it for every other window.
    pub fn change(&mut self, edit: impl FnOnce(&mut Self)) {
        edit(self);
        self.revision += 1;
    }
}
