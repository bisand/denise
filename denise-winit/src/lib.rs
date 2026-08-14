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
#[cfg(not(target_os = "macos"))]
mod surface;

use std::rc::Rc;
use std::time::{Duration, Instant};

use denise::{
    DamageTracker, ElementState, Frame, InputEvent, InputSource, MAX_DAMAGE_RECTS, Point,
    PointerButton, Rect, Size, Surface,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

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
}

/// Opens a window and runs `app` until it exits.
///
/// The application is built before the window exists, so it cannot know the
/// display's scale factor. On a 1× display that is exactly right; on a HiDPI one
/// it means a tree laid out in physical pixels comes out half size. Use
/// [`run_with`] there.
pub fn run<A: DeniseApp>(config: WindowConfig, app: A) -> Result<(), Error> {
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
    A: DeniseApp,
    B: FnOnce(Size, f32) -> A,
{
    let event_loop = EventLoop::new()?;
    let mut runner = Runner {
        config,
        build: Some(build),
        app: None,
        window: None,
        surface: None,
        damage: DamageTracker::new(Size::ZERO),
        events: Vec::new(),
        modifiers: ModifiersState::empty(),
        cursor: Point::ZERO,
        next_frame: Some(Instant::now()),
        error: None,
    };
    event_loop.run_app(&mut runner)?;
    match runner.error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

struct Runner<A, B> {
    config: WindowConfig,
    /// Consumed when the window appears; `None` from then on.
    build: Option<B>,
    app: Option<A>,
    window: Option<Rc<Window>>,
    surface: Option<PlatformSurface>,
    damage: DamageTracker,
    events: Vec<InputEvent>,
    modifiers: ModifiersState,
    cursor: Point,
    /// When the next frame is due, or `None` to wait for input.
    next_frame: Option<Instant>,
    error: Option<Error>,
}

impl<A: DeniseApp, B: FnOnce(Size, f32) -> A> Runner<A, B> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, err: Error) {
        self.error = Some(err);
        event_loop.exit();
    }

    fn on_resize(&mut self, size: PhysicalSize<u32>) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let size = Size::new(size.width, size.height);
        let scale = surface.window().scale_factor() as f32;
        surface.resize(size, scale);
        self.damage.resize(size);
        self.events.push(InputEvent::SurfaceResized {
            size,
            scale_factor: scale,
        });
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        match self.draw_frame() {
            // A zero-sized or minimised window has nothing to draw; not an error.
            Ok(_) => {}
            Err(err) => self.fail(event_loop, err),
        }

        // Advance the cadence only when a frame was actually attempted. Doing this
        // from `about_to_wait` instead pushes the deadline further out on every
        // spurious wake-up, and the loop never draws again.
        //
        // How far it advances is the application's to say. `frame_interval` is
        // the floor — a cap on how fast, never a demand — and the answer is
        // taken after the frame, when whatever just animated has had its say.
        let now = Instant::now();
        self.next_frame = self
            .app
            .as_ref()
            .map_or(Some(Duration::ZERO), A::next_frame_in)
            .map(|asked| now + asked.max(self.config.frame_interval));

        if self.app.as_ref().is_some_and(A::exit_requested) {
            event_loop.exit();
        }
    }

    /// Runs one update/render/present cycle. `Ok(false)` means the surface was not
    /// in a drawable state and the frame was skipped.
    fn draw_frame(&mut self) -> Result<bool, Error> {
        let Runner {
            app,
            surface,
            damage,
            events,
            ..
        } = self;
        let (Some(surface), Some(app)) = (surface.as_mut(), app.as_mut()) else {
            return Ok(false);
        };

        surface.poll(events);
        app.update(events, damage);
        events.clear();

        // Nothing changed: no buffer, no paint, no present, no frame at all.
        //
        // This is the whole promise of a damage tracker, and skipping it here
        // was costing more than everything else in the loop put together. A
        // present is not free anywhere, and on macOS it is not even
        // proportional to the damage: CoreAnimation re-uploads the entire
        // surface through `CGContextDrawImage` on every commit, whatever
        // rectangles softbuffer was handed. At 60 Hz on a 2560×1600 Retina
        // surface that is sixteen megabytes a frame — about a gigabyte a
        // second — to display a window in which nothing had happened.
        //
        // The shadow buffer is persistent, so a skipped frame leaves the
        // screen exactly as it was and the next real frame still only owes the
        // damage since this one.
        if damage.is_clean() {
            return Ok(false);
        }

        let mut frame = match surface.acquire() {
            Ok(frame) => frame,
            Err(denise::SurfaceError::NotReady) => return Ok(false),
            Err(err) => return Err(err.into()),
        };

        // Widen this frame's damage to cover everything the acquired buffer missed,
        // then copy it out so the tracker can be advanced afterwards.
        let mut resolved = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let src = damage.resolve(frame.age());
            resolved[..src.len()].copy_from_slice(src);
            src.len()
        };
        let region = &resolved[..count];

        app.render(&mut frame, region);
        drop(frame);

        surface.present(region)?;
        damage.end_frame();
        Ok(true)
    }
}

impl<A: DeniseApp, B: FnOnce(Size, f32) -> A> ApplicationHandler for Runner<A, B> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            // Logical, so the window covers the same apparent area whatever the
            // display's DPI. What comes back is physical and may be larger.
            .with_inner_size(LogicalSize::new(
                self.config.size.width,
                self.config.size.height,
            ))
            .with_resizable(self.config.resizable);

        let window = match event_loop.create_window(attrs) {
            Ok(window) => Rc::new(window),
            Err(err) => return self.fail(event_loop, err.into()),
        };

        let surface = match PlatformSurface::new(window.clone()) {
            Ok(surface) => surface,
            Err(err) => return self.fail(event_loop, err),
        };

        // The first moment the scale factor exists, and the last one before a tree
        // is built from it.
        if let Some(build) = self.build.take() {
            self.app = Some(build(surface.size(), surface.scale_factor()));
        }

        self.damage = DamageTracker::new(surface.size());
        self.window = Some(window);
        self.surface = Some(surface);
        self.next_frame = Some(Instant::now());
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Deliberately not keyed on `StartCause::ResumeTimeReached`. macOS cancels
        // the wait constantly for reasons of its own, and a loop that only draws on
        // a clean timeout never draws at all there. Compare against the deadline
        // instead: it is the same test, and it survives spurious wake-ups.
        if let Some(next) = self.next_frame
            && Instant::now() >= next
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.surface.is_none() {
            return;
        }

        match event {
            WindowEvent::RedrawRequested => return self.draw(event_loop),

            WindowEvent::Resized(size) => return self.on_resize(size),

            WindowEvent::ScaleFactorChanged { .. } => {
                let size = self.window.as_ref().map(|w| w.inner_size());
                if let Some(size) = size {
                    self.on_resize(size);
                }
                return;
            }

            // Handled before the surface is borrowed below, because answering it
            // needs the application: the event goes to the tree either way, and
            // the answer decides whether this is the last frame.
            WindowEvent::CloseRequested => {
                if let Some(surface) = self.surface.as_mut() {
                    surface.push_event(InputEvent::CloseRequested);
                }
                if self.app.as_mut().is_none_or(A::close_requested) {
                    event_loop.exit();
                }
                return;
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
                return;
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Point::new(position.x as i32, position.y as i32);
            }

            _ => {}
        }

        let modifiers = keymap::modifiers(self.modifiers);
        let cursor = self.cursor;
        let Some(surface) = self.surface.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CursorMoved { .. } => {
                surface.push_event(InputEvent::PointerMoved { position: cursor });
            }

            WindowEvent::CursorLeft { .. } => surface.push_event(InputEvent::PointerLeft),

            WindowEvent::MouseInput { state, button, .. } => {
                surface.push_event(InputEvent::PointerButton {
                    button: match button {
                        MouseButton::Left => PointerButton::Left,
                        MouseButton::Right => PointerButton::Right,
                        MouseButton::Middle => PointerButton::Middle,
                        MouseButton::Back => PointerButton::Other(3),
                        MouseButton::Forward => PointerButton::Other(4),
                        MouseButton::Other(n) => PointerButton::Other(n),
                    },
                    state: element_state(state),
                    position: cursor,
                    modifiers,
                });
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * LINE_HEIGHT_PX, y * LINE_HEIGHT_PX),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                surface.push_event(InputEvent::PointerScroll {
                    delta_x: dx,
                    // winit reports positive-up; Denise reports positive-down so the
                    // sign matches the content offset a scroll view applies.
                    delta_y: -dy,
                    position: cursor,
                });
            }

            WindowEvent::KeyboardInput { event, .. } => {
                surface.push_event(InputEvent::Key {
                    code: keymap::key_code(event.physical_key),
                    state: element_state(event.state),
                    repeat: event.repeat,
                    modifiers,
                });
                // Composed text only, and only on press. This is where `æøå` and
                // dead-key output arrive; the physical key above cannot carry them.
                if event.state.is_pressed()
                    && let Some(text) = event.text
                {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        surface.push_event(InputEvent::Text { ch });
                    }
                }
            }

            WindowEvent::Touch(touch) => {
                let position = Point::new(touch.location.x as i32, touch.location.y as i32);
                let id = touch.id;
                surface.push_event(match touch.phase {
                    TouchPhase::Started => InputEvent::TouchDown { id, position },
                    TouchPhase::Moved => InputEvent::TouchMoved { id, position },
                    TouchPhase::Ended => InputEvent::TouchUp {
                        id,
                        position,
                        cancelled: false,
                    },
                    TouchPhase::Cancelled => InputEvent::TouchUp {
                        id,
                        position,
                        cancelled: true,
                    },
                });
            }

            _ => {}
        }

        // Input outranks the cadence. An application that said "wake me in a
        // second, nothing is moving" still expects its button to light up when
        // pressed, so anything that arrived here is drawn at the next
        // opportunity rather than at the deadline.
        self.next_frame = Some(Instant::now());
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            return;
        }
        // Sleep until the next frame is due rather than spinning. An idle UI should
        // cost nothing, which is the property that has to hold on the real target.
        // `None` is the strongest form of that: nothing is animating, so there is
        // no deadline at all and the loop blocks until something arrives.
        event_loop.set_control_flow(match self.next_frame {
            Some(next) => ControlFlow::WaitUntil(next),
            None => ControlFlow::Wait,
        });
    }
}

fn element_state(state: winit::event::ElementState) -> ElementState {
    match state {
        winit::event::ElementState::Pressed => ElementState::Down,
        winit::event::ElementState::Released => ElementState::Up,
    }
}

/// Compiles the examples in this crate's README, so they cannot drift from the API
/// they claim to demonstrate. Never built except under `cargo test --doc`.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;
