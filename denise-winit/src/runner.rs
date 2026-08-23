//! The event loop, and the windows it drives.
//!
//! One `Runner` owns every window the application has open: the one [`run_with`]
//! created and every [`WindowRequest`] the application has handed back since. They
//! are not otherwise special-cased — a secondary window has its own surface, its
//! own damage tracker, its own frame deadline and its own [`DeniseApp`], and the
//! loop treats it exactly like the first one. The main window is distinguished by
//! one rule and no others: **closing it ends the run**, and closing any other
//! window closes that window.
//!
//! [`run_with`]: crate::run_with

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use denise::{
    DamageTracker, ElementState, InputEvent, InputSource, MAX_DAMAGE_RECTS, Point, PointerButton,
    Rect, Size, Surface,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::{
    Build, DeniseApp, Error, LINE_HEIGHT_PX, Modality, PlatformSurface, WindowConfig,
    WindowRequest, keymap, owner,
};

/// One window, its surface, and the application drawing into it.
///
/// Everything here is per-window and none of it can be shared. `modifiers` and
/// `cursor` especially: modifier state belongs to whichever window has keyboard
/// focus, and a shared cursor position would hand a dialog the coordinates of a
/// pointer that is over the window behind it.
struct WindowState {
    window: Rc<Window>,
    surface: PlatformSurface,
    app: Box<dyn DeniseApp>,
    damage: DamageTracker,
    events: Vec<InputEvent>,
    modifiers: ModifiersState,
    cursor: Point,
    /// When the next frame is due, or `None` to wait for input.
    next_frame: Option<Instant>,
    /// The window this one was opened from, and `None` for the main window or a
    /// window the application asked to be [`Modality::Independent`].
    owner: Option<WindowId>,
    modality: Modality,
    /// How fast this window may draw. Per-window because a settings form has no
    /// reason to share the main window's cadence.
    frame_interval: Duration,
}

pub(crate) struct Runner {
    /// The main window's configuration, held until there is an event loop to
    /// create it in.
    pub(crate) config: WindowConfig,
    /// Consumed when the main window appears; `None` from then on.
    pub(crate) build: Option<Build>,
    windows: HashMap<WindowId, WindowState>,
    /// The window whose closing ends the run. `None` until it exists.
    main: Option<WindowId>,
    /// Requests made during a frame, created before the loop next waits.
    ///
    /// Windows cannot be created while a frame is in flight: the application asks
    /// during `update`, and `ActiveEventLoop` is not in reach there. So the ask is
    /// queued with the window that made it — which is the window that will own the
    /// result — and honoured in `about_to_wait`, a few microseconds later.
    pending: Vec<(WindowId, WindowRequest)>,
    pub(crate) error: Option<Error>,
}

impl Runner {
    pub(crate) fn new(config: WindowConfig, build: Build) -> Self {
        Self {
            config,
            build: Some(build),
            windows: HashMap::new(),
            main: None,
            pending: Vec::new(),
            error: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, err: Error) {
        self.error = Some(err);
        event_loop.exit();
    }

    /// Creates a window, its surface and the application inside it.
    ///
    /// The application is built here rather than by the caller for the reason
    /// [`run_with`](crate::run_with) exists: the surface's size and the display's
    /// scale factor are two facts that do not exist until this moment, and a tree
    /// laid out in physical pixels without them comes out half size on a HiDPI
    /// display. A secondary window gets the same guarantee, and it is not
    /// academic — a settings form opens on whichever display its owner is on.
    fn spawn(
        &mut self,
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
        build: Build,
        owned_by: Option<WindowId>,
        modality: Modality,
    ) -> Result<WindowId, Error> {
        let mut attrs = Window::default_attributes()
            .with_title(config.title.clone())
            // Logical, so the window covers the same apparent area whatever the
            // display's DPI. What comes back is physical and may be larger.
            .with_inner_size(LogicalSize::new(config.size.width, config.size.height))
            .with_resizable(config.resizable);

        // The owner relationship is a creation-time fact on Windows, so it has to
        // be said here even though the platform that needs it most is not the one
        // this is usually compiled for.
        if modality != Modality::Independent
            && let Some(owner_state) = owned_by.and_then(|id| self.windows.get(&id))
        {
            attrs = owner::own(attrs, &owner_state.window);
        }

        let window = Rc::new(event_loop.create_window(attrs)?);
        let surface = PlatformSurface::new(window.clone())?;
        let id = window.id();

        if modality != Modality::Independent
            && let Some(owner_state) = owned_by.and_then(|id| self.windows.get(&id))
        {
            owner::adopt(&owner_state.window, &window);
            if modality == Modality::Modal {
                owner::set_enabled(&owner_state.window, false);
            }
        }

        let app = build(surface.size(), surface.scale_factor());

        self.windows.insert(
            id,
            WindowState {
                damage: DamageTracker::new(surface.size()),
                window,
                surface,
                app,
                events: Vec::new(),
                modifiers: ModifiersState::empty(),
                cursor: Point::ZERO,
                next_frame: Some(Instant::now()),
                owner: owned_by.filter(|_| modality != Modality::Independent),
                modality,
                frame_interval: config.frame_interval,
            },
        );
        Ok(id)
    }

    /// Closes a window and everything opened from it.
    ///
    /// Depth first, so a modal over a settings form is gone before the form it
    /// belonged to — which is the order that leaves no window without an owner,
    /// however briefly.
    fn close(&mut self, id: WindowId) {
        let mut unblocked = Vec::new();

        let links: Vec<Link> = self.links().collect();
        for doomed in closing_order(&links, id) {
            let Some(closed) = self.windows.remove(&doomed) else {
                continue;
            };
            if let (Modality::Modal, Some(owner_id)) = (closed.modality, closed.owner) {
                unblocked.push(owner_id);
            }
        }

        for owner_id in unblocked {
            // Two modals over the same owner is not a thing any application should
            // do, but re-enabling the owner while the second one is still up would
            // be a window that ignores the dialog in front of it — the exact bug
            // modality exists to prevent. So the owner comes back only when the
            // last of them has gone, and only if it is still here itself: closing
            // an owner is also what closed this modal.
            if self.blocker(owner_id).is_some() {
                continue;
            }
            if let Some(owner_state) = self.windows.get(&owner_id) {
                owner::set_enabled(&owner_state.window, true);
                owner_state.window.focus_window();
            }
        }
    }

    /// Every window's place in the ownership graph.
    fn links(&self) -> impl Iterator<Item = Link> + '_ {
        self.windows
            .iter()
            .map(|(id, state)| (*id, state.owner, state.modality))
    }

    /// The modal window blocking `id`, if one is open over it.
    ///
    /// On the input path — every pointer move consults it — so it walks the
    /// windows rather than collecting them. There are never many.
    fn blocker(&self, id: WindowId) -> Option<&WindowState> {
        let blocker = blocker_of(self.links(), id)?;
        self.windows.get(&blocker)
    }

    fn on_resize(&mut self, id: WindowId, size: PhysicalSize<u32>) {
        let Some(state) = self.windows.get_mut(&id) else {
            return;
        };
        let size = Size::new(size.width, size.height);
        let scale = state.surface.window().scale_factor() as f32;
        state.surface.resize(size, scale);
        state.damage.resize(size);
        state.events.push(InputEvent::SurfaceResized {
            size,
            scale_factor: scale,
        });
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        match self.draw_frame(id) {
            // A zero-sized or minimised window has nothing to draw; not an error.
            Ok(_) => {}
            Err(err) => return self.fail(event_loop, err),
        }

        let Some(state) = self.windows.get_mut(&id) else {
            return;
        };

        // Advance the cadence only when a frame was actually attempted. Doing this
        // from `about_to_wait` instead pushes the deadline further out on every
        // spurious wake-up, and the loop never draws again.
        //
        // How far it advances is the application's to say. `frame_interval` is
        // the floor — a cap on how fast, never a demand — and the answer is
        // taken after the frame, when whatever just animated has had its say.
        let now = Instant::now();
        let interval = state.frame_interval;
        state.next_frame = state
            .app
            .next_frame_in()
            .map(|asked| now + asked.max(interval));

        if state.app.exit_requested() {
            // The main window ends the run; any other window ends only itself,
            // which is what makes a settings form closable without taking the
            // application down with it.
            if self.main == Some(id) {
                event_loop.exit();
            } else {
                self.close(id);
            }
        }
    }

    /// Runs one update/render/present cycle. `Ok(false)` means the surface was not
    /// in a drawable state and the frame was skipped.
    fn draw_frame(&mut self, id: WindowId) -> Result<bool, Error> {
        let requests = {
            let Some(state) = self.windows.get_mut(&id) else {
                return Ok(false);
            };
            state.surface.poll(&mut state.events);
            state.app.update(&state.events, &mut state.damage);
            state.events.clear();
            // Asked for on every frame, including the ones that draw nothing: a
            // window opened in response to a keypress that dirtied no pixels is
            // still a window the application asked for.
            state.app.take_windows()
        };
        self.pending
            .extend(requests.into_iter().map(|request| (id, request)));

        self.paint(id)
    }

    /// Paints and presents one window, if anything in it changed.
    fn paint(&mut self, id: WindowId) -> Result<bool, Error> {
        let Some(state) = self.windows.get_mut(&id) else {
            return Ok(false);
        };

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
        if state.damage.is_clean() {
            return Ok(false);
        }

        let mut frame = match state.surface.acquire() {
            Ok(frame) => frame,
            Err(denise::SurfaceError::NotReady) => return Ok(false),
            Err(err) => return Err(err.into()),
        };

        // Widen this frame's damage to cover everything the acquired buffer missed,
        // then copy it out so the tracker can be advanced afterwards.
        let mut resolved = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let src = state.damage.resolve(frame.age());
            resolved[..src.len()].copy_from_slice(src);
            src.len()
        };
        let region = &resolved[..count];

        state.app.render(&mut frame, region);
        drop(frame);

        state.surface.present(region)?;
        state.damage.end_frame();
        Ok(true)
    }

    /// Creates every window asked for since the last wait.
    ///
    /// A request whose owner has closed in the meantime is dropped: the form it
    /// belonged to is gone, and a modal with nothing to be modal to is a window
    /// nobody can explain.
    fn open_pending(&mut self, event_loop: &ActiveEventLoop) {
        for (opener, request) in std::mem::take(&mut self.pending) {
            if !self.windows.contains_key(&opener) {
                continue;
            }
            let WindowRequest {
                config,
                modality,
                build,
            } = request;
            if let Err(err) = self.spawn(event_loop, &config, build, Some(opener), modality) {
                return self.fail(event_loop, err);
            }
        }
    }
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.main.is_some() {
            return;
        }
        let Some(build) = self.build.take() else {
            return;
        };
        let config = self.config.clone();
        match self.spawn(event_loop, &config, build, None, Modality::Independent) {
            Ok(id) => self.main = Some(id),
            Err(err) => self.fail(event_loop, err),
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Deliberately not keyed on `StartCause::ResumeTimeReached`. macOS cancels
        // the wait constantly for reasons of its own, and a loop that only draws on
        // a clean timeout never draws at all there. Compare against the deadline
        // instead: it is the same test, and it survives spurious wake-ups.
        let now = Instant::now();
        for state in self.windows.values() {
            if state.next_frame.is_some_and(|next| now >= next) {
                state.window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if !self.windows.contains_key(&id) {
            return;
        }

        match event {
            WindowEvent::RedrawRequested => return self.draw(event_loop, id),

            WindowEvent::Resized(size) => return self.on_resize(id, size),

            WindowEvent::ScaleFactorChanged { .. } => {
                let size = self.windows.get(&id).map(|s| s.window.inner_size());
                if let Some(size) = size {
                    self.on_resize(id, size);
                }
                return;
            }

            _ => {}
        }

        // A window with a modal open over it is not a window the user is talking
        // to. Windows has already stopped delivering most of this — a disabled
        // window gets no input at all — but macOS and Linux have not, and this is
        // where modality is actually enforced on them: the events are dropped, and
        // a press says where the user should be looking instead.
        //
        // Repainting is deliberately above this line. A blocked window still
        // resizes, still redraws and still animates; what it does not do is
        // listen.
        if let Some(blocker) = self.blocker(id) {
            let raise = matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::MouseInput {
                        state: winit::event::ElementState::Pressed,
                        ..
                    }
            );
            if raise {
                blocker.window.focus_window();
            }
            return;
        }

        match event {
            // Handled before the surface is borrowed below, because answering it
            // needs the application: the event goes to the tree either way, and
            // the answer decides whether this is the last frame.
            WindowEvent::CloseRequested => {
                let Some(state) = self.windows.get_mut(&id) else {
                    return;
                };
                state.surface.push_event(InputEvent::CloseRequested);
                if !state.app.close_requested() {
                    return;
                }
                if self.main == Some(id) {
                    event_loop.exit();
                } else {
                    self.close(id);
                }
                return;
            }

            WindowEvent::ModifiersChanged(mods) => {
                if let Some(state) = self.windows.get_mut(&id) {
                    state.modifiers = mods.state();
                }
                return;
            }

            WindowEvent::CursorMoved { position, .. } => {
                if let Some(state) = self.windows.get_mut(&id) {
                    state.cursor = Point::new(position.x as i32, position.y as i32);
                }
            }

            _ => {}
        }

        let Some(state) = self.windows.get_mut(&id) else {
            return;
        };
        let modifiers = keymap::modifiers(state.modifiers);
        let cursor = state.cursor;
        let surface = &mut state.surface;

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
        state.next_frame = Some(Instant::now());
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            return;
        }
        self.open_pending(event_loop);

        // Every window that has gone leaves the run when the last one does. The
        // main window normally ends it first — this is for the application that
        // closed it from the inside and left a settings form up.
        if self.windows.is_empty() {
            event_loop.exit();
            return;
        }

        // Sleep until the *earliest* frame is due rather than spinning. An idle UI
        // should cost nothing, which is the property that has to hold on the real
        // target. `None` from every window is the strongest form of that: nothing
        // anywhere is animating, so there is no deadline at all and the loop blocks
        // until something arrives. One animating window is enough to set the pace,
        // and the windows that are not animating still draw nothing when it does.
        let next = self
            .windows
            .values()
            .filter_map(|state| state.next_frame)
            .min();
        event_loop.set_control_flow(match next {
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

/// One window's place in the ownership graph: the window, who opened it, and how.
///
/// The two rules below are the whole of what ownership *means* to the loop, and
/// they are the part that breaks: a cascade that closes an owner before the window
/// it owns leaves a window pointing at nothing, and an owner re-enabled while a
/// second modal is still up is a window that ignores the dialog in front of it.
/// Both are awkward to reproduce by hand and trivial to state, so they are stated
/// here — over plain data, with no window in sight — and tested everywhere rather
/// than only where there is a display. The same split `keymap` makes.
type Link = (WindowId, Option<WindowId>, Modality);

/// The modal blocking `id`, if one is open over it.
///
/// Takes an iterator because the caller that matters is on the input path and has
/// nothing to gain from a collection.
fn blocker_of(links: impl Iterator<Item = Link>, id: WindowId) -> Option<WindowId> {
    links
        .into_iter()
        .find(|(_, owner, modality)| *owner == Some(id) && *modality == Modality::Modal)
        .map(|(blocker, _, _)| blocker)
}

/// `id` and everything opened from it, innermost first.
///
/// The order is the point: a window is always listed after everything it owns, so
/// closing down the list never orphans anything.
fn closing_order(links: &[Link], id: WindowId) -> Vec<WindowId> {
    let mut order = Vec::new();
    for (child, owner, _) in links {
        if *owner == Some(id) {
            order.extend(closing_order(links, *child));
        }
    }
    order.push(id);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> WindowId {
        WindowId::from(raw)
    }

    /// The example's shape: a main window, a modeless settings form, a modal over
    /// that form, and a second modal over the main window.
    fn tree() -> Vec<Link> {
        vec![
            (id(1), None, Modality::Independent),
            (id(2), Some(id(1)), Modality::Owned),
            (id(3), Some(id(2)), Modality::Modal),
            (id(4), Some(id(1)), Modality::Modal),
        ]
    }

    #[test]
    fn a_modal_blocks_its_own_owner_and_nobody_else() {
        assert_eq!(blocker_of(tree().into_iter(), id(1)), Some(id(4)));
        assert_eq!(blocker_of(tree().into_iter(), id(2)), Some(id(3)));
        // Nothing is open over the modals themselves.
        assert_eq!(blocker_of(tree().into_iter(), id(3)), None);
        assert_eq!(blocker_of(tree().into_iter(), id(4)), None);
    }

    #[test]
    fn a_modeless_form_blocks_nothing() {
        let links = vec![
            (id(1), None, Modality::Independent),
            (id(2), Some(id(1)), Modality::Owned),
        ];
        assert_eq!(blocker_of(links.iter().copied(), id(1)), None);
    }

    #[test]
    fn closing_a_form_closes_what_it_opened_first() {
        assert_eq!(closing_order(&tree(), id(2)), vec![id(3), id(2)]);
    }

    #[test]
    fn closing_the_main_window_closes_the_tree_owners_last() {
        let order = closing_order(&tree(), id(1));
        assert_eq!(order.len(), 4);
        assert_eq!(*order.last().expect("the opener comes last"), id(1));

        let at = |window| {
            order
                .iter()
                .position(|listed| *listed == window)
                .expect("every window in the tree is listed")
        };
        assert!(
            at(id(3)) < at(id(2)),
            "a modal closes before the form under it"
        );
        assert!(at(id(2)) < at(id(1)));
        assert!(at(id(4)) < at(id(1)));
    }

    #[test]
    fn a_window_that_owns_nothing_closes_alone() {
        assert_eq!(closing_order(&tree(), id(4)), vec![id(4)]);
    }

    /// An independent window is nobody's child, so nothing takes it along.
    #[test]
    fn an_independent_window_is_not_cascaded() {
        let links = vec![
            (id(1), None, Modality::Independent),
            (id(2), None, Modality::Independent),
        ];
        assert_eq!(closing_order(&links, id(1)), vec![id(1)]);
    }

    /// The rule `close` consults before handing input back to an owner.
    #[test]
    fn the_owner_stays_blocked_until_the_last_modal_goes() {
        let mut links = vec![
            (id(1), None, Modality::Independent),
            (id(2), Some(id(1)), Modality::Modal),
            (id(3), Some(id(1)), Modality::Modal),
        ];
        links.retain(|(window, _, _)| *window != id(2));
        assert_eq!(blocker_of(links.iter().copied(), id(1)), Some(id(3)));
        links.retain(|(window, _, _)| *window != id(3));
        assert_eq!(blocker_of(links.iter().copied(), id(1)), None);
    }
}
