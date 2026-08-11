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
mod surface;

use std::rc::Rc;
use std::time::{Duration, Instant};

use denise::{
    DamageTracker, ElementState, Frame, InputEvent, InputSource, MAX_DAMAGE_RECTS, Point,
    PointerButton, Rect, Size, Surface,
};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

pub use surface::WinitSurface;

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
    #[error("softbuffer: {0}")]
    Softbuffer(#[from] softbuffer::SoftBufferError),

    /// A surface operation failed.
    #[error(transparent)]
    Surface(#[from] denise::SurfaceError),
}

/// How the preview window is created.
#[derive(Clone, Debug)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Initial inner size in physical pixels.
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
}

/// Opens a window and runs `app` until it exits.
pub fn run<A: DeniseApp>(config: WindowConfig, app: A) -> Result<(), Error> {
    let event_loop = EventLoop::new()?;
    let mut runner = Runner {
        config,
        app,
        window: None,
        surface: None,
        damage: DamageTracker::new(Size::ZERO),
        events: Vec::new(),
        modifiers: ModifiersState::empty(),
        cursor: Point::ZERO,
        next_frame: Instant::now(),
        error: None,
    };
    event_loop.run_app(&mut runner)?;
    match runner.error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

struct Runner<A> {
    config: WindowConfig,
    app: A,
    window: Option<Rc<Window>>,
    surface: Option<WinitSurface>,
    damage: DamageTracker,
    events: Vec<InputEvent>,
    modifiers: ModifiersState,
    cursor: Point,
    next_frame: Instant,
    error: Option<Error>,
}

impl<A: DeniseApp> Runner<A> {
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
        let now = Instant::now();
        self.next_frame += self.config.frame_interval;
        if self.next_frame <= now {
            // We fell behind. Resynchronise rather than replaying missed frames.
            self.next_frame = now + self.config.frame_interval;
        }

        if self.app.exit_requested() {
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
        let Some(surface) = surface.as_mut() else {
            return Ok(false);
        };

        surface.poll(events);
        app.update(events, damage);
        events.clear();

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

impl<A: DeniseApp> ApplicationHandler for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(PhysicalSize::new(
                self.config.size.width,
                self.config.size.height,
            ))
            .with_resizable(self.config.resizable);

        let window = match event_loop.create_window(attrs) {
            Ok(window) => Rc::new(window),
            Err(err) => return self.fail(event_loop, err.into()),
        };

        let surface = match WinitSurface::new(window.clone()) {
            Ok(surface) => surface,
            Err(err) => return self.fail(event_loop, err),
        };

        self.damage = DamageTracker::new(surface.size());
        self.window = Some(window);
        self.surface = Some(surface);
        self.next_frame = Instant::now();
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Deliberately not keyed on `StartCause::ResumeTimeReached`. macOS cancels
        // the wait constantly for reasons of its own, and a loop that only draws on
        // a clean timeout never draws at all there. Compare against the deadline
        // instead: it is the same test, and it survives spurious wake-ups.
        if Instant::now() >= self.next_frame
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
            WindowEvent::CloseRequested => {
                surface.push_event(InputEvent::CloseRequested);
            }

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
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            return;
        }
        // Sleep until the next frame is due rather than spinning. An idle UI should
        // cost nothing, which is the property that has to hold on the real target.
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

fn element_state(state: winit::event::ElementState) -> ElementState {
    match state {
        winit::event::ElementState::Pressed => ElementState::Down,
        winit::event::ElementState::Released => ElementState::Up,
    }
}
