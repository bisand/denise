//! Desktop development and preview backend for Denise.
//!
//! This backend exists so the core abstraction can be proven — and iterated on —
//! without a Raspberry Pi on the desk. It is not a deployment target: shipping
//! Denise on a desktop means shipping a compositor you did not need.
//!
//! ```no_run
//! use denise::{Color, DamageTracker, Frame, InputEvent, Rect};
//! use denise_render::Canvas;
//! use denise_winit::{DeniseApp, WindowConfig, run};
//!
//! struct Hello;
//!
//! impl DeniseApp for Hello {
//!     fn update(&mut self, _events: &[InputEvent], _damage: &mut DamageTracker) {}
//!
//!     fn render(&mut self, frame: &mut Frame<'_>, damage: &[Rect]) {
//!         let mut canvas = Canvas::new(frame);
//!         for region in damage {
//!             canvas.with_clip(*region).clear(Color::from_rgb888(0x1E1E2E));
//!         }
//!     }
//! }
//!
//! run(WindowConfig::default(), Hello).unwrap();
//! ```

mod keymap;
#[cfg(target_os = "macos")]
mod macos;
mod owner;
mod runner;
#[cfg(not(target_os = "macos"))]
mod surface;

use std::time::Duration;

use denise::{DamageTracker, Frame, InputEvent, Rect, Size};
use winit::event_loop::EventLoop;

use runner::Runner;

#[cfg(target_os = "macos")]
pub use macos::MacSurface;
#[cfg(not(target_os = "macos"))]
pub use surface::WinitSurface;

/// The surface this backend presents through, which is not the same everywhere.
///
/// softbuffer on every platform but one; on macOS an `IOSurface` handed straight
/// to the window's layer, because softbuffer's CoreGraphics backend copies the
/// whole surface three times per present and ignores the damage. See
/// [`macos`](self) for the measurements.
#[cfg(target_os = "macos")]
type PlatformSurface = MacSurface;
#[cfg(not(target_os = "macos"))]
type PlatformSurface = WinitSurface;

/// Nominal pixels per wheel notch, for platforms that report scroll in lines.
const LINE_HEIGHT_PX: f32 = 16.0;

/// Failures from this backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The event loop could not be created or run.
    #[error("event loop: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),

    /// The window could not be created.
    #[error("window creation: {0}")]
    Window(#[from] winit::error::OsError),

    /// softbuffer could not bind to the window or present.
    ///
    /// Absent on macOS, which does not present through softbuffer.
    #[cfg(not(target_os = "macos"))]
    #[error("softbuffer: {0}")]
    Softbuffer(#[from] softbuffer::SoftBufferError),

    /// A surface operation failed.
    #[error(transparent)]
    Surface(#[from] denise::SurfaceError),

    /// The platform's presentation path could not be set up.
    #[error("present: {0}")]
    Present(String),
}

/// How the preview window is created.
#[derive(Clone, Debug)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Initial inner size in **logical** pixels.
    ///
    /// Logical, not physical, so one number describes the same amount of desk on
    /// every machine: a panel designed at 800×480 covers 800×480 of a Pi's
    /// framebuffer and the same apparent area on a 2× Retina display, where the
    /// surface it gets is 1600×960 physical pixels. Asking in physical pixels
    /// instead is how a window ends up a quarter of its intended size on a Mac.
    ///
    /// The surface — and therefore every coordinate the application works in —
    /// stays physical. Scaling the content to match is the application's job, and
    /// it is handed the factor at construction by [`run_with`].
    pub size: Size,
    /// Whether the user may resize the window.
    pub resizable: bool,
    /// Target frame interval. Defaults to 60 Hz.
    ///
    /// The loop sleeps until the next deadline rather than spinning, so an idle UI
    /// costs close to nothing — which is the behaviour we actually care about on
    /// the target hardware.
    pub frame_interval: Duration,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Denise".into(),
            size: Size::new(800, 480),
            resizable: true,
            frame_interval: Duration::from_nanos(1_000_000_000 / 60),
        }
    }
}

/// An application driven by this backend.
///
/// This is M0 scaffolding, not the eventual public API. From M3 the scene stack and
/// component tree sit between the application and these two methods.
pub trait DeniseApp {
    /// Handles input and records what that changed.
    ///
    /// Marking damage here rather than during `render` is deliberate: the renderer
    /// needs to know what to repaint *before* it starts, and on a real swapchain
    /// the region it must cover is wider than what changed this frame.
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker);

    /// Draws the frame.
    ///
    /// `damage` is the region that must be repainted for this particular buffer,
    /// already widened for its age and clipped to the surface. Drawing outside it
    /// is wasted work; drawing less than it leaves stale pixels.
    fn render(&mut self, frame: &mut Frame<'_>, damage: &[Rect]);

    /// Return `true` to quit after the current frame.
    fn exit_requested(&self) -> bool {
        false
    }

    /// How long the loop may sleep before asking for another frame.
    ///
    /// The default — `Some(Duration::ZERO)` — means "as often as
    /// [`WindowConfig::frame_interval`] allows", which is what this backend has
    /// always done. Answering with a longer wait, or with `None` for "nothing is
    /// animating, wake me on input", is how an application stops the loop doing
    /// work nobody asked for.
    ///
    /// A tree already knows the answer: `Ui::next_wake_ms` is the deadline of the
    /// most impatient animation in it. Ignoring it is not free. A `Spinner` asks
    /// to be woken every 50 ms and moves its arc exactly that often; ticked at
    /// 60 Hz instead it reports a repaint three times as often as it has anything
    /// new to show, and every one of those is a present. The kiosk backends have
    /// always slept on `next_wake_ms` — this is what lets a window agree with
    /// them.
    ///
    /// Input does not wait for this: an event wakes the loop immediately,
    /// whatever was asked for here.
    fn next_frame_in(&self) -> Option<Duration> {
        Some(Duration::ZERO)
    }

    /// Whether the window manager's close request should end the run.
    ///
    /// Defaults to `true`, because a close button that does not close is a bug in
    /// every application that has not deliberately decided otherwise. The request
    /// also arrives in [`update`](DeniseApp::update) as
    /// [`InputEvent::CloseRequested`], which is where saving on the way out
    /// belongs.
    ///
    /// Override it to `false` to *veto* the close — an unsaved-changes prompt is
    /// the reason to, and the application then quits by way of
    /// [`exit_requested`](DeniseApp::exit_requested) once the answer comes back.
    /// A veto is only as good as that follow-up: an application that never sets
    /// `exit_requested` has made its window unclosable by anything short of the
    /// platform's own kill.
    fn close_requested(&mut self) -> bool {
        true
    }

    /// Windows this application wants opened, taken once per frame.
    ///
    /// This is the whole of the secondary-window API, and what it hands back is
    /// another [`DeniseApp`] — so a settings form, an "edit details" window and
    /// the main window are the same kind of thing, built the same way, running in
    /// the same loop. The backend supplies a window, a surface and a place in the
    /// event loop; **what is inside one is entirely the application's**, exactly
    /// as `Ui::push_scene` knows nothing about the scene it pushed.
    ///
    /// Called immediately after [`update`](DeniseApp::update) on every frame,
    /// including frames that draw nothing. Returning the same request twice opens
    /// two windows: an application that must not open its settings form twice
    /// remembers that it has one open, which it needs to do anyway to know what to
    /// tell the second click.
    ///
    /// The new window is owned by the window whose application asked for it — so
    /// a modal opened from a settings form is modal to *that* form, not to the
    /// main window, and closing the form takes the modal with it.
    ///
    /// # Talking to a window you opened
    ///
    /// Nothing here carries state back, on purpose. A form is built by the
    /// application, so the application can give it whatever it likes to hold —
    /// and `Rc<RefCell<_>>` is the whole mechanism:
    ///
    /// ```no_run
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// #[derive(Default)]
    /// struct Settings {
    ///     brightness: u8,
    ///     /// Set by the form when it wants to go; read by its `exit_requested`.
    ///     closing: bool,
    /// }
    ///
    /// // The main window keeps one handle, the form gets another. Whichever one
    /// // writes, both see it.
    /// let shared = Rc::new(RefCell::new(Settings::default()));
    /// let for_the_form = shared.clone();
    /// ```
    ///
    /// The form's `exit_requested` returns `shared.borrow().closing`, which is
    /// also how the main window closes it from the outside. Nothing in the
    /// backend needs to know any of this happened.
    fn take_windows(&mut self) -> Vec<WindowRequest> {
        Vec::new()
    }
}

/// Opens a window and runs `app` until it exits.
///
/// The application is built before the window exists, so it cannot know the
/// display's scale factor. On a 1× display that is exactly right; on a HiDPI one
/// it means a tree laid out in physical pixels comes out half size. Use
/// [`run_with`] there.
pub fn run<A: DeniseApp + 'static>(config: WindowConfig, app: A) -> Result<(), Error> {
    run_with(config, move |_, _| app)
}

/// Opens a window and builds the application once the surface behind it is known.
///
/// The builder is handed the surface size in **physical** pixels and the display's
/// scale factor — the two facts a scale-aware tree needs and cannot obtain any
/// earlier. This is the whole of Denise's DPI story on the desktop: the application
/// scales, once, at construction, through `Theme::scaled`, `Rect::scaled` and its
/// own text sizes. Coordinates stay physical everywhere afterwards.
///
/// A later scale change — dragging the window to a display with a different DPI —
/// arrives as [`InputEvent::SurfaceResized`], carrying the new factor. An
/// application that wants to follow it rebuilds its tree there; one that does not
/// keeps the scale it was built with and is merely sized wrong on the second
/// display.
pub fn run_with<A, B>(config: WindowConfig, build: B) -> Result<(), Error>
where
    A: DeniseApp + 'static,
    B: FnOnce(Size, f32) -> A + 'static,
{
    let event_loop = EventLoop::new()?;
    let mut runner = Runner::new(config, boxed(build));
    event_loop.run_app(&mut runner)?;
    match runner.error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// How a secondary window relates to the one that opened it.
///
/// How much of this the platform will actually enforce differs sharply: Windows
/// does all of it, macOS keeps the z-order and blocks nothing, and X11 and Wayland
/// offer neither through winit. So the one guarantee that holds everywhere is
/// [`Modal`](Modality::Modal) input blocking, because the runner does that itself
/// rather than asking. The rest is appearance, and the crate's README has the
/// table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Modality {
    /// Above its owner, closed with it, and input still reaches both.
    ///
    /// The default, and what a settings form or an "edit details" window wants: a
    /// window belonging to this application rather than a second application that
    /// happens to share a process. The user can keep working in the main window
    /// while it is open.
    #[default]
    Owned,

    /// Owned, and the owner takes no input until this window closes.
    ///
    /// The window-sized version of `Ui::push_scene` with a dim: a confirmation, a
    /// wizard, anything the user must answer before going back. The owner keeps
    /// drawing — animations there do not freeze — it simply stops listening, and
    /// a press on it raises this window instead.
    Modal,

    /// A window of its own, with no relationship to its opener at all.
    ///
    /// A second document window, or a tool palette the user may want behind the
    /// main window. It does not close when its opener does, so an application that
    /// opens one is responsible for it — including for the fact that closing the
    /// main window ends the run and takes it down regardless.
    Independent,
}

/// A window the application wants opened, and the application that will run in it.
///
/// Handed back from [`DeniseApp::take_windows`]. The builder is called once, at
/// the moment the window's surface exists, and is given its size in **physical**
/// pixels and its display's scale factor — the same contract [`run_with`] makes
/// for the main window, and for the same reason: a form built without them is
/// laid out for a display it may not be opening on.
pub struct WindowRequest {
    /// Title, size, resizability and frame cadence for the new window.
    pub config: WindowConfig,
    /// How it relates to the window that asked for it.
    pub modality: Modality,
    pub(crate) build: Build,
}

impl WindowRequest {
    /// A window of `config`, whose application is built once its surface exists.
    ///
    /// [`Modality::Owned`] unless [`with_modality`](WindowRequest::with_modality)
    /// says otherwise.
    pub fn new<A, B>(config: WindowConfig, build: B) -> Self
    where
        A: DeniseApp + 'static,
        B: FnOnce(Size, f32) -> A + 'static,
    {
        Self {
            config,
            modality: Modality::default(),
            build: boxed(build),
        }
    }

    /// A window whose application is already built.
    ///
    /// Convenient, and wrong on a HiDPI display unless the tree inside it does not
    /// care about scale: the application cannot have been told the scale factor,
    /// because the window it will open on does not exist yet.
    /// [`new`](WindowRequest::new) is the one to reach for.
    pub fn ready<A: DeniseApp + 'static>(config: WindowConfig, app: A) -> Self {
        Self::new(config, move |_, _| app)
    }

    /// Sets how this window relates to the one opening it.
    #[must_use]
    pub fn with_modality(self, modality: Modality) -> Self {
        Self { modality, ..self }
    }
}

/// Builds an application once the surface it will draw to is known.
///
/// Boxed because the windows in one run do not share an application type — a
/// settings form is not the main window with different data, it is a different
/// program — and the loop holds them in one collection.
type Build = Box<dyn FnOnce(Size, f32) -> Box<dyn DeniseApp>>;

/// Erases a builder's application type.
fn boxed<A, B>(build: B) -> Build
where
    A: DeniseApp + 'static,
    B: FnOnce(Size, f32) -> A + 'static,
{
    Box::new(move |size, scale| Box::new(build(size, scale)))
}

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
