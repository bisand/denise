//! The tree, the scene stack, and the compositor that turns them into pixels.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::{Drain, Vec};

use denise::{
    Color, DamageTracker, ElementState, Frame, InputEvent, KeyCode, MAX_DAMAGE_RECTS,
    MAX_TRACKED_FRAMES, Modifiers, Point, Rect, Role, Size, Surface, SurfaceError, Theme,
};
use denise_render::Canvas;
use denise_text::{FontId, GlyphSource, TextEngine};
use slotmap::SlotMap;

use crate::cursor::{Cursor, CursorImage};
use crate::node::{Node, NodeId, Popup, Scene};
use crate::toast::Toasts;
use crate::tooltip::Tooltip;

/// Space between a popup and its anchor, in pixels. Small on purpose: a
/// dropdown visually belongs to its button, and a gap wide enough to see the
/// page through reads as two unrelated panels.
const POPUP_GAP: i32 = 4;
use crate::anchor::{self, Anchors, Dock};
use crate::motion::{Motion, Wake};
use crate::widget::{
    Event, EventCtx, Handled, MeasureCtx, Measured, Offer, PaintCtx, VisualState, Void, Widget,
};
use crate::widgets::describe::{DynDescribe, Property, PropertyError, Value};

/// A drawer's life, tracked by the tree.
#[derive(Clone, Copy, Debug)]
struct DrawerState {
    container: NodeId,
    closing: bool,
}

/// A shelf's node and whether it is on its way out. The same two facts a drawer
/// keeps, for the same reason: the slide has to finish before the node goes.
#[derive(Clone, Copy)]
struct ShelfState {
    container: NodeId,
    closing: bool,
}

/// How long a drawer takes to slide in or out.
const DRAWER_MS: u64 = 200;

/// How long a shelf takes to slide in or out. A drawer's, because they are
/// the same motion and two numbers would only drift apart.
const SHELF_MS: u64 = DRAWER_MS;

/// A shelf sorts above ordinary content in its scene. Nothing else in the
/// tree sets `z`, so any positive number would do; this one is far enough
/// from zero to leave room underneath for an application that wants some.
const SHELF_Z: i32 = 1_000;

/// The dim behind a drawer: enough to say "modal", light enough to keep the
/// page readable behind it.
const DRAWER_DIM: u8 = 120;

/// One layout being carried from `from` to `to` by the tree.
#[derive(Clone, Copy, Debug)]
struct LayoutTween {
    id: NodeId,
    from: Rect,
    to: Rect,
    start_ms: u64,
    duration_ms: u64,
}

impl LayoutTween {
    /// Where the journey has got to at `now_ms`: integer, monotonic, and
    /// exactly `to` from the duration onward.
    fn at(&self, now_ms: u64) -> Rect {
        let elapsed = now_ms.saturating_sub(self.start_ms);
        if elapsed >= self.duration_ms || self.duration_ms == 0 {
            return self.to;
        }
        let lerp = |a: i32, b: i32| -> i32 {
            a + (i64::from(b - a) * elapsed as i64 / self.duration_ms as i64) as i32
        };
        Rect::new(
            lerp(self.from.x, self.to.x),
            lerp(self.from.y, self.to.y),
            lerp(self.from.width, self.to.width),
            lerp(self.from.height, self.to.height),
        )
    }
}

/// A frame whose damage was nothing but one viewport scrolling.
///
/// The damage tracker unions, so a hover highlight *inside* a scrolled viewport
/// vanishes into the viewport's own rectangle and cannot be told apart from it
/// afterwards. Scrolling by moving pixels rather than redrawing them needs
/// exactly that told apart, so it is recorded as it happens rather than
/// reconstructed later. `None` in the ring means "something else changed too",
/// which is most frames.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Scrolled {
    /// The viewport that moved.
    node: NodeId,
    /// How far its content moved, this frame.
    by: Point,
    /// Where the viewport was, so a relayout in the meantime disqualifies it.
    clip: Rect,
}

/// A retained tree of widgets, a stack of scenes, and the damage they generate.
///
/// `M` is the application's message type. Widgets emit `M`; the application drains
/// them with [`Ui::drain_messages`] and decides what they mean. Nothing calls back
/// into the application mid-traversal, so there is no borrow to fight and no
/// `Rc<RefCell<_>>` anywhere in the path.
///
/// # Damage is the tree's job, not the application's
///
/// This is the part worth reading. Every route into mutable widget state runs
/// through the tree, and every one of them marks the node dirty:
///
/// - [`Ui::widget_mut`] invalidates on access, before you have even changed
///   anything. Taking `&mut` to a widget *is* the declaration that it will look
///   different.
/// - Hover, press, focus and enabled are tracked by the tree, so a widget cannot
///   forget to invalidate on a state it does not own.
/// - [`Handled::Yes`] from `on_event` invalidates the node.
/// - Moving, resizing, showing, hiding, adding or removing a node damages the
///   rectangles it vacated and the ones it now occupies.
///
/// The alternative — the application deciding what changed and telling the damage
/// tracker — is where the classic bug lives: some piece of state that decides the
/// pixels is left out of the comparison, and the screen keeps a stale colour until
/// something unrelated repaints over it. That bug is not fixed here so much as made
/// unrepresentable.
pub struct Ui<M: 'static> {
    nodes: SlotMap<NodeId, Node<M>>,
    scenes: Vec<Scene>,
    /// Flattened paint order across every scene: parents before children, siblings
    /// by z. Rebuilt only on structural or z-order change, never per frame.
    order: Vec<NodeId>,
    /// Exclusive end index in `order` for each scene.
    scene_end: Vec<usize>,
    order_dirty: bool,
    size: Size,
    theme: Theme,
    text: TextEngine,
    damage: DamageTracker,
    /// What each of the last few frames was, when it was only a scroll — the
    /// same ring the damage keeps, advanced in step with it. See [`Scrolled`].
    scrolled: [Option<Scrolled>; MAX_TRACKED_FRAMES],
    /// This frame's slot in that ring.
    scroll_head: usize,
    pointer: Point,
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    /// A touch that landed on a scrollable's background rather than on any
    /// interactive widget: subsequent moves drag the scroll. A touch that lands
    /// on a widget belongs to the widget — stealing an in-progress press for
    /// scrolling is gesture disambiguation, deliberately not attempted yet.
    touch_scroll: Option<(NodeId, Point)>,
    focused: Option<NodeId>,
    cursor: Cursor,
    messages: Vec<M>,
    now_ms: u64,
    next_wake: Option<u64>,
    /// Nodes that have asked to animate. Emptied by their own `animate` answers:
    /// a widget returning [`Wake::Never`] drops out. Kept deliberately small and
    /// deliberately visible — [`Ui::animating`] exists so a test can assert a
    /// tree at rest holds nobody awake.
    animating: Vec<NodeId>,
    /// How fast everything in the tree animates, and whether it does at all.
    /// The one place the rate is decided — see [`Motion`].
    motion: Motion,
    /// Layout tweens in flight: nodes the *tree* is carrying from one
    /// rectangle to another. Bounded by construction — every tween has a
    /// duration and is removed at arrival — and counted by [`Ui::animating`]
    /// alongside the widgets' animations, so the idle-cost evidence covers
    /// both kinds of motion.
    tweens: Vec<LayoutTween>,
    /// The drawer, while one is up: its container, and whether it is on the
    /// way out. The scene pops when the closing slide lands — the tree
    /// watching its own tween, rather than a public completion hook nobody
    /// has asked for yet.
    drawer: Option<DrawerState>,
    shelf: Option<ShelfState>,
    focus_changed: Option<Option<NodeId>>,
    occluded: Option<Rect>,
    /// Transient notifications. Not nodes, for the reasons in [`crate::toast`].
    toasts: Toasts,
    /// The hover-dwell bubble. Not a node and not a widget — see
    /// [`crate::tooltip`] for why.
    tooltip: Tooltip,
    /// Whether the tree still gets to decide the cursor's visibility. Cleared by
    /// the first `show_cursor`, which is a host taking the decision over.
    cursor_auto: bool,
}

impl<M: 'static> Ui<M> {
    /// Creates a tree covering a surface of `size`, with one base scene.
    pub fn new(size: Size, theme: Theme) -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(Node::new(Box::new(Void), Rect::from_size(size), 0));
        Self {
            nodes,
            scenes: vec![Scene {
                root,
                dim: 0,
                popup: None,
            }],
            order: Vec::new(),
            scene_end: Vec::new(),
            order_dirty: true,
            size,
            theme,
            text: TextEngine::new(),
            damage: DamageTracker::new(size),
            scrolled: [None; MAX_TRACKED_FRAMES],
            scroll_head: 0,
            pointer: Point::ZERO,
            hovered: None,
            pressed: None,
            touch_scroll: None,
            focused: None,
            cursor: Cursor::default(),
            tooltip: Tooltip::new(),
            toasts: Toasts::new(),
            messages: Vec::new(),
            now_ms: 0,
            next_wake: None,
            animating: Vec::new(),
            motion: Motion::default(),
            tweens: Vec::new(),
            drawer: None,
            shelf: None,
            focus_changed: None,
            occluded: None,
            cursor_auto: true,
        }
    }

    /// Surface extent the tree lays out against.
    #[inline]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The active theme.
    #[inline]
    pub const fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Swaps the theme. Every colour on screen may have changed, so this damages
    /// the whole surface — the one case where a full repaint is the honest answer.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.damage.add_full();
    }

    /// Fonts and the glyph cache.
    #[inline]
    pub const fn text(&self) -> &TextEngine {
        &self.text
    }

    /// Fonts and the glyph cache, mutably. Measuring fills the cache, so this is
    /// how an application asks how wide a string will be before laying it out.
    #[inline]
    pub const fn text_mut(&mut self) -> &mut TextEngine {
        &mut self.text
    }

    /// Registers a font and returns its id.
    ///
    /// The built-in bitmap font is always registered as `FontId(0)`, so a widget
    /// that names no font has one regardless of what else was loaded. Everything
    /// on screen may change width, so this damages the whole surface.
    pub fn add_font(&mut self, source: alloc::boxed::Box<dyn GlyphSource>) -> FontId {
        let id = self.text.add_font(source);
        self.damage.add_full();
        id
    }

    /// The cursor sprite.
    #[inline]
    pub const fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Replaces the cursor sprite, damaging both shapes' footprints.
    pub fn set_cursor_image(&mut self, image: &'static CursorImage) {
        self.dirty(self.cursor.bounds());
        self.cursor.image = image;
        self.dirty(self.cursor.bounds());
    }

    /// Shows or hides the cursor sprite, and stops the tree deciding for itself.
    ///
    /// Left alone, the sprite starts hidden, reveals itself on the first pointer
    /// motion and hides again when a finger arrives — which is what a panel with
    /// no window system underneath it wants, because nothing else is going to
    /// draw a pointer.
    ///
    /// Calling this takes that policy over for good, in whichever direction. That
    /// matters for an embedded host: a Win32 child window or an `NSView` already
    /// has a system cursor, and Denise compositing a second one that lags it by a
    /// frame is worse than drawing none. Such a host calls `show_cursor(false)`
    /// once at startup and never thinks about it again.
    pub fn show_cursor(&mut self, visible: bool) {
        self.cursor_auto = false;
        if self.cursor.visible == visible {
            return;
        }
        self.dirty(self.cursor.bounds());
        self.cursor.visible = visible;
        self.dirty(self.cursor.bounds());
    }

    // ---------------------------------------------------------------- scenes

    /// Root node of the base scene.
    #[inline]
    pub fn root(&self) -> NodeId {
        self.scenes[0].root
    }

    /// Root node of the topmost scene: the one that receives input.
    #[inline]
    pub fn top_root(&self) -> NodeId {
        self.scenes[self.scenes.len() - 1].root
    }

    /// Number of scenes on the stack, always at least one.
    #[inline]
    pub fn scene_count(&self) -> usize {
        self.scenes.len()
    }

    /// Pushes a scene over the current one and returns its root.
    ///
    /// `dim` is the alpha of a black backdrop painted under the new scene, `0` for
    /// none and `128` for a conventional modal veil. The backdrop is painted per
    /// damage region rather than over the whole surface, which matters: a
    /// full-screen alpha fill measured 63% of a 60 Hz frame budget on a Pi 3, so a
    /// dialog that repaints its own caret must not drag a megapixel of blending
    /// along with it.
    ///
    /// The new scene takes all input. Nothing underneath is hittable, focusable or
    /// reachable by Tab until it is popped — that is what makes it modal, and it is
    /// a property of the stack rather than something each dialog has to enforce.
    pub fn push_scene(&mut self, dim: u8) -> NodeId {
        let index = self.scenes.len();
        let root = self
            .nodes
            .insert(Node::new(Box::new(Void), Rect::from_size(self.size), index));
        self.scenes.push(Scene {
            root,
            dim,
            popup: None,
        });
        self.order_dirty = true;
        self.set_focus(None);
        self.cancel_press();
        self.set_hovered(None);
        if dim > 0 {
            self.damage.add_full();
        }
        root
    }

    /// Pushes a popup: a scene anchored to a node, dismissed by clicking away.
    ///
    /// The returned container is placed beside `anchor` on the preferred `side`
    /// — flipping to the other side when the surface has no room, see
    /// [`anchored`](crate::overlay::anchored) — and the caller adds content to
    /// it, exactly as [`Tabs`](crate::widgets::Tabs) leaves pages to the
    /// caller. The container itself draws nothing; the first child is usually a
    /// [`Panel`](crate::widgets::Panel) filling it.
    ///
    /// What makes it a popup rather than a plain scene:
    ///
    /// - **A press outside the container closes it, and is swallowed.** The
    ///   press must not also reach whatever is underneath — a dropdown that
    ///   closes *and* activates the button behind it is the classic bug. The
    ///   swallowing is structural: input only ever reaches the topmost scene,
    ///   so the press has nowhere else to go; closing consumes it entirely.
    /// - **Escape closes it**, before the focused widget sees the key.
    /// - **Focus returns to the anchor** on close, however it closes.
    ///
    /// There is no dimming: a popup is not a modal. A dialog that takes over is
    /// [`Ui::push_scene`] with a dim, and a tooltip needs no scene at all —
    /// just a non-interactive node placed with `anchored` at a high z.
    ///
    /// A popup over a modal is ordinary nesting and works; the popup's own
    /// scene captures input while it is up, and popping it returns input to
    /// the modal. Popups do not re-anchor when the surface is resized — they
    /// are transient, and the honest response to a resize is closing them.
    ///
    /// Returns `None` when `anchor` does not exist.
    pub fn push_popup(
        &mut self,
        anchor: NodeId,
        size: Size,
        side: crate::overlay::Side,
    ) -> Option<NodeId> {
        let anchor_bounds = self.bounds(anchor)?;
        let rect = crate::overlay::anchored(self.size, anchor_bounds, size, side, POPUP_GAP);
        let root = self.push_scene(0);
        let container = self
            .add(root, Void, rect)
            .expect("a scene root can always take a child");
        self.scenes.last_mut().expect("just pushed").popup = Some(Popup { anchor, container });
        Some(container)
    }

    /// Closes the topmost scene if it is a popup, returning focus to its
    /// anchor. Returns `false` when the top scene is not a popup.
    ///
    /// This is what a press outside the popup and the Escape key call; an
    /// application closes a popup the same way after acting on a selection.
    pub fn close_popup(&mut self) -> bool {
        match self.scenes.last() {
            Some(scene) if scene.popup.is_some() => self.pop_scene(),
            _ => false,
        }
    }

    /// Slides a panel in from an edge of the screen, over a dimmed backdrop,
    /// and returns its container for the application to fill.
    ///
    /// A drawer is modality plus motion, and both halves already exist: this
    /// composes [`Ui::push_scene`] with [`Ui::animate_layout`]. `size` is the
    /// drawer's width for [`Side::Before`](crate::Side::Before)/[`Side::After`](crate::Side::After) and its height for
    /// [`Side::Above`](crate::Side::Above)/[`Side::Below`](crate::Side::Below); the other dimension spans the screen.
    ///
    /// Escape closes it, a press on the dim closes it, and
    /// [`Ui::close_drawer`] closes it from the application — all by sliding
    /// out first: the scene pops when the slide lands, and focus returns to
    /// where it was, [`Ui::push_popup`]'s conventions. One drawer at a time;
    /// pushing over an open one returns `None`.
    pub fn push_drawer(&mut self, side: crate::overlay::Side, size: i32) -> Option<NodeId> {
        if self.drawer.is_some() {
            return None;
        }
        let (resting, offstage) = self.edge_rects(side, size);
        let root = self.push_scene(DRAWER_DIM);
        let container = self
            .add(root, Void, offstage)
            .expect("a scene root can always take a child");
        self.animate_layout(container, resting, DRAWER_MS);
        self.drawer = Some(DrawerState {
            container,
            closing: false,
        });
        Some(container)
    }

    /// Slides the drawer out; the scene pops when the slide lands.
    ///
    /// Returns `false` when no drawer is up. Calling again while one is
    /// already closing does nothing — the slide finishes on its own.
    pub fn close_drawer(&mut self) -> bool {
        let Some(state) = self.drawer else {
            return false;
        };
        if state.closing {
            return true;
        }
        let Some(layout) = self.layout(state.container) else {
            self.drawer = None;
            return false;
        };
        let screen = Rect::from_size(self.size);
        // Back out the way it came in: whichever screen edge is nearest.
        let offstage = if layout.x <= 0 && layout.width < screen.width {
            Rect::new(-layout.width, layout.y, layout.width, layout.height)
        } else if layout.right() >= screen.width && layout.width < screen.width {
            Rect::new(screen.width, layout.y, layout.width, layout.height)
        } else if layout.y <= 0 {
            Rect::new(layout.x, -layout.height, layout.width, layout.height)
        } else {
            Rect::new(layout.x, screen.height, layout.width, layout.height)
        };
        self.animate_layout(state.container, offstage, DRAWER_MS);
        self.drawer = Some(DrawerState {
            closing: true,
            ..state
        });
        true
    }

    /// Whether a drawer is up, closing included.
    #[inline]
    pub fn drawer_open(&self) -> bool {
        self.drawer.is_some()
    }

    /// Where a panel of `size` rests against `side`, and where it waits offstage.
    ///
    /// The full width or height of the screen in the other dimension. Shared by
    /// [`Ui::push_drawer`] and [`Ui::push_shelf`], which differ in everything
    /// except the arithmetic.
    fn edge_rects(&self, side: crate::overlay::Side, size: i32) -> (Rect, Rect) {
        use crate::overlay::Side;
        let screen = Rect::from_size(self.size);
        let size = size.clamp(1, screen.width.max(screen.height));
        match side {
            Side::Before => (
                Rect::new(0, 0, size, screen.height),
                Rect::new(-size, 0, size, screen.height),
            ),
            Side::After => (
                Rect::new(screen.width - size, 0, size, screen.height),
                Rect::new(screen.width, 0, size, screen.height),
            ),
            Side::Above => (
                Rect::new(0, 0, screen.width, size),
                Rect::new(0, -size, screen.width, size),
            ),
            Side::Below => (
                Rect::new(0, screen.height - size, screen.width, size),
                Rect::new(0, screen.height, screen.width, size),
            ),
        }
    }

    /// Slides a panel in from an edge and leaves everything else alone.
    ///
    /// A drawer is modality plus motion; a shelf is the motion without the
    /// modality. It pushes no scene, so **focus does not move**, input still
    /// reaches what is underneath, and a press outside does not dismiss it.
    /// The application closes it, because the application is the only thing
    /// that knows when it is done.
    ///
    /// That is the whole difference, and it is what an on-screen keyboard
    /// needs: it exists to type into a field that must stay focused while it
    /// is up. A status strip, a notification shade and a media bar over a
    /// video want the same thing.
    ///
    /// The container sits above ordinary content in the base scene, painting over
    /// it — and *under* any scene pushed later, which is why a modal covers a
    /// shelf rather than fighting it. One at a time; pushing over an open one
    /// returns `None`.
    ///
    /// `size` is the width for [`Side::Before`](crate::Side::Before) and
    /// [`Side::After`](crate::Side::After), the height for
    /// [`Side::Above`](crate::Side::Above) and [`Side::Below`](crate::Side::Below).
    ///
    /// ```
    /// # use denise::{Size, theme};
    /// # use denise_ui::{Side, Ui};
    /// # enum Msg { Noop }
    /// # let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
    /// let shelf = ui.push_shelf(Side::Below, 200).expect("nothing else is up");
    /// assert!(ui.shelf_open());
    /// ```
    pub fn push_shelf(&mut self, side: crate::overlay::Side, size: i32) -> Option<NodeId> {
        if self.shelf.is_some() {
            return None;
        }
        let (resting, offstage) = self.edge_rects(side, size);
        let base = self.root();
        let container = self.add(base, Void, offstage)?;
        self.set_z(container, SHELF_Z);
        self.animate_layout(container, resting, SHELF_MS);
        // Where it will be, not where it is: something focused during the slide
        // should be revealed clear of the keyboard's resting place rather than
        // scrolled twice.
        self.occluded = Some(resting);
        self.shelf = Some(ShelfState {
            container,
            closing: false,
        });
        Some(container)
    }

    /// Slides the shelf out; the node is removed when the slide lands.
    ///
    /// Returns `false` when none is up. Calling again while one is already
    /// closing does nothing — the slide finishes on its own.
    pub fn close_shelf(&mut self) -> bool {
        let Some(state) = self.shelf else {
            return false;
        };
        if state.closing {
            return true;
        }
        let Some(layout) = self.layout(state.container) else {
            self.shelf = None;
            return false;
        };
        let screen = Rect::from_size(self.size);
        // Back out the way it came in: whichever screen edge is nearest.
        let offstage = if layout.x <= 0 && layout.width < screen.width {
            Rect::new(-layout.width, layout.y, layout.width, layout.height)
        } else if layout.right() >= screen.width && layout.width < screen.width {
            Rect::new(screen.width, layout.y, layout.width, layout.height)
        } else if layout.y <= 0 {
            Rect::new(layout.x, -layout.height, layout.width, layout.height)
        } else {
            Rect::new(layout.x, screen.height, layout.width, layout.height)
        };
        self.animate_layout(state.container, offstage, SHELF_MS);
        // Given back when it starts leaving rather than when it lands: it is on
        // its way out, and a field focused now belongs on the whole screen.
        self.occluded = None;
        self.shelf = Some(ShelfState {
            closing: true,
            ..state
        });
        true
    }

    /// Where focus went since this was last asked, or `None` if it has not moved.
    ///
    /// Drained on read, like [`Ui::drain_messages`], and read in the same place:
    /// the application's turn, once a frame. `Some(None)` is focus *lost* —
    /// somebody clicked the background — which is a different event from focus
    /// not having moved, and the two are worth telling apart.
    ///
    /// The tree reports the movement and takes no view on what it means. Whether
    /// a node deserves an on-screen keyboard is a question only the application
    /// can answer, and answering it here would mean `denise-ui` knowing what a
    /// keyboard is.
    ///
    /// ```
    /// # use denise::{Rect, Size, theme};
    /// # use denise_ui::{Ui, widgets::TextInput};
    /// # #[derive(Clone, Debug)] enum Msg { Noop }
    /// # let mut ui: Ui<Msg> = Ui::new(Size::new(800, 480), theme::DARK);
    /// # let root = ui.root();
    /// let field = ui.add(root, TextInput::new(), Rect::new(0, 0, 200, 40)).unwrap();
    /// ui.focus(Some(field));
    /// assert_eq!(ui.focus_changed(), Some(Some(field)));
    /// assert_eq!(ui.focus_changed(), None, "drained on read");
    /// ```
    pub fn focus_changed(&mut self) -> Option<Option<NodeId>> {
        self.focus_changed.take()
    }

    /// The part of the surface a shelf is covering, if one is up.
    ///
    /// A shelf is the one thing that hides content without capturing input, so
    /// it is the one thing the tree has to remember is in the way. Revealing a
    /// focused node already scrolls clear of it — see
    /// [`Ui::focus_changed`] for what an application does with the movement —
    /// and this is for the case scrolling cannot fix: a layout with fixed
    /// rectangles, where getting a field out from under the keyboard means the
    /// application moving something.
    ///
    /// The resting rectangle from the moment the shelf is pushed, so a reveal
    /// during the slide aims where the shelf is going rather than where it has
    /// got to. `None` from the moment it starts leaving.
    #[inline]
    pub const fn occluded(&self) -> Option<Rect> {
        self.occluded
    }

    /// Whether a shelf is up, closing included.
    #[inline]
    pub fn shelf_open(&self) -> bool {
        self.shelf.is_some()
    }

    /// Whether a popup — a dropdown's option list, a tooltip's anchor menu — is up.
    ///
    /// The companion to [`drawer_open`](Ui::drawer_open), and it exists for the
    /// same reason: these two and only these two make the tree claim Escape. An
    /// application that binds Escape itself asks both before acting, so a key that
    /// should dismiss a dropdown does not quit the program instead.
    #[inline]
    pub fn popup_open(&self) -> bool {
        self.scenes.last().is_some_and(|s| s.popup.is_some())
    }

    /// Pops the topmost scene and everything in it. The base scene cannot be
    /// popped; returns `false` if that is all there is.
    ///
    /// A popped popup returns focus to its anchor — through here, so it holds
    /// however the popup is closed.
    pub fn pop_scene(&mut self) -> bool {
        if self.scenes.len() <= 1 {
            return false;
        }
        self.ensure_order();
        let scene = self.scenes.pop().expect("checked non-empty");
        let start = if self.scenes.is_empty() {
            0
        } else {
            self.scene_end[self.scenes.len() - 1]
        };
        // Repaint exactly what the scene covered: every node's clip, plus the whole
        // surface if it was dimming what was underneath.
        if scene.dim > 0 {
            self.damage.add_full();
        } else {
            for i in start..self.order.len() {
                let id = self.order[i];
                if let Some(node) = self.nodes.get(id) {
                    let clip = node.clip;
                    self.dirty(clip);
                }
            }
        }
        self.drop_subtree(scene.root);
        self.order_dirty = true;
        self.set_focus(None);
        self.cancel_press();
        self.set_hovered(None);
        if let Some(popup) = scene.popup {
            // Focus goes back where the popup came from, not to nothing: a
            // keyboard user who opened a dropdown and pressed Escape is
            // standing exactly where they were before it opened.
            self.focus(Some(popup.anchor));
        }
        true
    }

    // ------------------------------------------------------------------ tree

    /// Adds `widget` under `parent`, positioned relative to the parent's origin.
    ///
    /// Returns `None` if `parent` no longer exists.
    pub fn add(&mut self, parent: NodeId, widget: impl Widget<M>, layout: Rect) -> Option<NodeId> {
        let scene = self.nodes.get(parent)?.scene;
        let id = self
            .nodes
            .insert(Node::new(Box::new(widget), layout, scene));
        self.nodes[id].parent = Some(parent);
        self.nodes[parent].children.push(id);
        self.sort_children(parent);
        // Into a stack, a new child pushes its siblings down; the reflow and
        // the damage have to cover them, not just the newcomer.
        let root = self.reflow_root(id);
        self.reflow(root);
        self.damage_subtree(root);
        self.order_dirty = true;
        Some(id)
    }

    /// Removes a node and its descendants. Scene roots cannot be removed this way;
    /// use [`Ui::pop_scene`].
    pub fn remove(&mut self, id: NodeId) -> bool {
        if !self.nodes.contains_key(id) || self.scenes.iter().any(|s| s.root == id) {
            return false;
        }
        self.damage_subtree(id);
        let parent = self.nodes[id].parent;
        if let Some(parent) = parent
            && let Some(node) = self.nodes.get_mut(parent)
        {
            node.children.retain(|&c| c != id);
        }
        self.drop_subtree(id);
        // Out of a stack, the siblings below close the gap.
        if let Some(parent) = parent
            && self.nodes.get(parent).is_some_and(|n| n.stack.is_some())
        {
            self.reflow(parent);
            self.damage_subtree(parent);
        }
        self.order_dirty = true;
        true
    }

    /// Returns `true` if the node still exists.
    #[inline]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Absolute bounds of a node, before ancestor clipping.
    #[inline]
    pub fn bounds(&self, id: NodeId) -> Option<Rect> {
        self.nodes.get(id).map(|n| n.bounds)
    }

    /// Position and extent relative to the parent.
    #[inline]
    pub fn layout(&self, id: NodeId) -> Option<Rect> {
        self.nodes.get(id).map(|n| n.layout)
    }

    /// Moves or resizes a node, damaging the rectangles it left and the ones it
    /// now occupies.
    /// Marks a node as a viewport the tree may scroll: wheel over it, page
    /// keys inside it, and reveal requests from its content all move its
    /// [`Ui::scroll`] offset. Content is clipped to the node either way — this
    /// flag is about who may *move* it.
    ///
    /// Explicit rather than inferred from overflowing content, so a panel with
    /// a decoratively clipped child does not start moving under the wheel.
    pub fn set_scrollable(&mut self, id: NodeId, scrollable: bool) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.scrollable = scrollable;
        }
    }

    /// Shows a transient notification, which fades in, holds and goes by itself.
    ///
    /// The overlay counterpart of [`Alert`](crate::widgets::Alert): an alert
    /// sits *in* the layout where the thing it is about would be, and a toast
    /// is the same message when there is nowhere in the layout to put it. It is
    /// not a node, so it never takes focus, never appears in the tab order and
    /// nothing has to remove it.
    ///
    /// A press inside a toast dismisses it **and is swallowed**, so somebody
    /// clearing a notification does not also press the button it was covering.
    ///
    /// It costs almost nothing while it holds: the tree asks to be woken once,
    /// at the instant the fade-out starts. Only the fades draw frames.
    pub fn toast(&mut self, text: impl Into<alloc::string::String>, role: Role) {
        self.toast_for(text, role, crate::toast::HOLD_MS);
    }

    /// A toast that holds for a stated time before fading.
    ///
    /// For the message somebody needs longer to read, or the one that should
    /// barely register. The fades are fixed either way.
    pub fn toast_for(&mut self, text: impl Into<alloc::string::String>, role: Role, hold_ms: u64) {
        self.toasts.push(text.into(), role, hold_ms, self.now_ms);
        self.damage_toasts();
        // A toast added between ticks must not wait for an unrelated event to
        // appear: the loop may be blocked on input right now.
        self.next_wake = Some(self.now_ms);
    }

    /// How many notifications are on screen.
    #[inline]
    pub fn toasts(&self) -> usize {
        self.toasts.len()
    }

    /// Removes every notification, read or not.
    pub fn clear_toasts(&mut self) {
        if self.toasts.len() == 0 {
            return;
        }
        self.damage_toasts();
        self.toasts.clear();
    }

    /// Shows `text` when the pointer rests on this node.
    ///
    /// A **pointer** affordance: it needs hover, and a touchscreen has none, so
    /// on a touch-only panel this does nothing at all. That is the honest
    /// outcome rather than a gap — the panels that want tooltips are the
    /// mouse-driven HMIs and the controls embedded in desktop applications
    /// where every other control has one.
    ///
    /// The tree owns everything else about it: the dwell delay, the placement
    /// (below the node, flipping above near an edge), the dismissal on any
    /// press or key, and the drawing — above every widget, below the cursor.
    /// It is not a node, so it is never hit-tested and never takes focus.
    pub fn set_tooltip(&mut self, id: NodeId, text: impl Into<alloc::string::String>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.tooltip = Some(text.into());
        }
    }

    /// Removes a node's tooltip.
    pub fn clear_tooltip(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.tooltip = None;
        }
        if self.hovered == Some(id) {
            if self.tooltip.is_shown() {
                self.damage_tooltip();
            }
            self.tooltip.dismiss();
        }
    }

    /// How far a node's content is scrolled. `Point::ZERO` until somebody
    /// scrolls.
    pub fn scroll(&self, id: NodeId) -> Point {
        self.nodes.get(id).map_or(Point::ZERO, |n| n.scroll)
    }

    /// The furthest a node can scroll: how far its content extends past its
    /// own rectangle, axis by axis. Zero when everything fits.
    pub fn max_scroll(&self, id: NodeId) -> Point {
        let Some(node) = self.nodes.get(id) else {
            return Point::ZERO;
        };
        let mut right = 0;
        let mut bottom = 0;
        match node.stack {
            None => {
                for &child in &node.children {
                    if let Some(child) = self.nodes.get(child) {
                        right = right.max(child.layout.right());
                        bottom = bottom.max(child.layout.bottom());
                    }
                }
            }
            // A stack places children at the running y, not at their layout's,
            // so the content's extent is the same arithmetic `reflow` runs: the
            // visible heights plus the gaps between them. Reading the layouts
            // here would make a scrollable stacked column — a settings page of
            // cards — report almost no range at all.
            Some(spacing) => {
                let mut running = 0i32;
                let mut any = false;
                for &child in &node.children {
                    if let Some(child) = self.nodes.get(child)
                        && child.visible
                    {
                        right = right.max(child.layout.right());
                        running = running
                            .saturating_add(child.layout.height.max(0))
                            .saturating_add(spacing);
                        any = true;
                    }
                }
                if any {
                    bottom = running - spacing;
                }
            }
        }
        Point::new(
            (right - node.layout.width).max(0),
            (bottom - node.layout.height).max(0),
        )
    }

    /// Scrolls a node's content to `offset`, clamped to what its content
    /// actually extends to — a viewport cannot be scrolled past its last child
    /// or into negative space.
    ///
    /// Damages the whole viewport, deliberately: scrolling moves every visible
    /// pixel in it, and the honest damage for that is the viewport itself.
    pub fn set_scroll(&mut self, id: NodeId, offset: Point) {
        let limit = self.max_scroll(id);
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let clamped = Point::new(offset.x.clamp(0, limit.x), offset.y.clamp(0, limit.y));
        if node.scroll == clamped {
            return;
        }
        let was = node.scroll;
        node.scroll = clamped;
        let clip = node.clip;
        let by = Point::new(clamped.x - was.x, clamped.y - was.y);
        // Recorded *before* the damage, and not through `dirty`: this is the
        // scroll, and the scroll is what the record is about.
        self.note_scroll(id, by, clip);
        self.damage.add(clip);
        self.reflow(id);
        // The pointer has not moved, but what is under it has.
        self.update_hover();
    }

    /// Scrolls a node's content by a delta, clamped like [`Ui::set_scroll`].
    pub fn scroll_by(&mut self, id: NodeId, dx: i32, dy: i32) {
        let current = self.scroll(id);
        self.set_scroll(
            id,
            Point::new(current.x.saturating_add(dx), current.y.saturating_add(dy)),
        );
    }

    /// How big a node would like to be, given what the caller can promise.
    ///
    /// [`Measured::NOTHING`] when the node has no opinion, which is most of
    /// them, and when there is no node of that id.
    ///
    /// **This exists because of a borrow.** Measuring needs the widget and the
    /// text engine at once, and both live in this struct — so
    /// `widget.preferred_width(ui.text_mut())` cannot be written by anybody
    /// outside it, however much they are holding. An application that *is*
    /// holding the widget, before it goes in the tree, should keep calling the
    /// widget's own `preferred_width`/`preferred_height`: they are the nicer
    /// call and this is a wrapper over the same arithmetic.
    ///
    /// **The tree never calls this itself.** See [`Widget::measure`] for why
    /// that sentence is the whole point.
    ///
    /// ```
    /// # use denise::{Rect, Size, theme};
    /// # use denise_ui::{Measured, Offer, Ui, Void, widgets::{Label, Panel}};
    /// let mut ui: Ui<Void> = Ui::new(Size::new(320, 240), theme::DARK);
    /// let root = ui.root();
    /// let hello = ui.add(root, Label::new("Hello"), Rect::new(0, 0, 10, 10)).unwrap();
    /// let panel = ui.add(root, Panel::default(), Rect::new(0, 0, 10, 10)).unwrap();
    ///
    /// // A label is as wide as its text, whatever rectangle it was given.
    /// let wanted = ui.measure(hello, Offer::NOTHING);
    /// assert!(wanted.width.is_some_and(|w| w > 0));
    ///
    /// // A panel is the background other things sit on, and has no view.
    /// assert_eq!(ui.measure(panel, Offer::NOTHING), Measured::NOTHING);
    /// ```
    pub fn measure(&mut self, id: NodeId, offered: Offer) -> Measured {
        // The disjoint-field borrow the paint path already relies on: `nodes` is
        // read while `text` is written, which is allowed inside this type and
        // expressible nowhere else.
        let Some(node) = self.nodes.get(id) else {
            return Measured::NOTHING;
        };
        let mut ctx = MeasureCtx {
            theme: &self.theme,
            text: &mut self.text,
        };
        node.widget.measure(&mut ctx, offered)
    }

    /// Moves or resizes a node, damaging the rectangles it left and the ones it
    /// now occupies. Siblings in a [stack](Ui::set_stack) move with it.
    ///
    /// Cancels any [`Ui::animate_layout`] in flight on this node: the
    /// application wrote state, and state written is state shown — the
    /// silent-setter rule applied to the tree itself.
    pub fn set_layout(&mut self, id: NodeId, layout: Rect) {
        self.tweens.retain(|t| t.id != id);
        // A new layout is a new design, stated against whatever the parent is
        // now, so anchoring re-baselines against the box this node is next
        // placed in. `apply_layout` deliberately does not: it is also the path a
        // tween drives, and re-baselining every frame would leave an anchored
        // node standing still while its parent moved around it.
        if let Some(node) = self.nodes.get_mut(id) {
            node.anchor_base = None;
        }
        self.apply_layout(id, layout);
    }

    /// [`Ui::set_layout`] without the tween cancellation — the path the tween
    /// itself drives, so advancing a tween does not cancel it.
    fn apply_layout(&mut self, id: NodeId, layout: Rect) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.layout == layout {
            return;
        }
        // The stack parent, when there is one: a resized child moves every
        // sibling below it, so the damage and the reflow both start there.
        let root = self.reflow_root(id);
        self.damage_subtree(root);
        self.nodes[id].layout = layout;
        self.reflow(root);
        self.damage_subtree(root);
        // Content that shrank may have left a viewport scrolled past its own
        // last child, which paints as a band of nothing at the bottom and no
        // way to reach what is above it — collapse a section, hide a widget,
        // take rows out of a list, or give back the room an on-screen keyboard
        // was borrowing. `max_scroll` is computed on demand and was already
        // right; the stored offset was the stale half.
        self.clamp_scroll_above(id);
    }

    /// Re-clamps this node's scroll and every scrollable ancestor's.
    ///
    /// Upwards because the node that changed size is the *content*: the
    /// viewport whose offset is now out of range is one of its parents. Itself
    /// too, since a node can be both.
    fn clamp_scroll_above(&mut self, id: NodeId) {
        let mut next = Some(id);
        while let Some(current) = next {
            let Some(node) = self.nodes.get(current) else {
                return;
            };
            next = node.parent;
            if node.scroll == Point::ZERO {
                continue;
            }
            let limit = self.max_scroll(current);
            let scroll = self.nodes[current].scroll;
            let clamped = Point::new(scroll.x.clamp(0, limit.x), scroll.y.clamp(0, limit.y));
            if clamped != scroll {
                self.nodes[current].scroll = clamped;
                let clip = self.nodes[current].clip;
                self.dirty(clip);
                self.reflow(current);
            }
        }
    }

    /// Carries a node's layout to `to` over `duration_ms`, through the same
    /// path [`Ui::set_layout`] uses — so damage and reflow, stacks included,
    /// come along on every frame.
    ///
    /// Runs on [`Ui::tick`], at about 20 fps while flying, and lands *exactly*
    /// on `to`. A second call mid-flight retargets from the current mid-flight
    /// rectangle, so a section told to close while opening turns around
    /// smoothly. A plain [`Ui::set_layout`] cancels the journey; hiding the
    /// node completes it instantly, because a hidden node must not keep the
    /// device awake and half-moved is the one dishonest place to stop.
    ///
    /// Counted by [`Ui::animating`], so the idle-cost evidence covers it.
    pub fn animate_layout(&mut self, id: NodeId, to: Rect, duration_ms: u64) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let from = node.layout;
        self.tweens.retain(|t| t.id != id);
        if duration_ms == 0 || from == to {
            self.apply_layout(id, to);
            return;
        }
        self.tweens.push(LayoutTween {
            id,
            from,
            to,
            start_ms: self.now_ms,
            duration_ms,
        });
        // Wake immediately, as a widget's request_animation does: the first
        // frame belongs to the next tick, however far away the event loop
        // thought its next deadline was.
        self.next_wake = Some(self.next_wake.map_or(self.now_ms, |w| w.min(self.now_ms)));
    }

    /// Makes a node a vertical stack: its visible children are placed
    /// top-to-bottom in order, `spacing` pixels apart, each keeping its own
    /// x, width and height.
    ///
    /// Not a layout engine, and not the intrinsic-size protocol — the tree
    /// still asks widgets nothing, and every height is the same explicit
    /// rectangle as ever. It is scrolling's argument again: paint, damage,
    /// clipping and hit testing must agree about where a moved sibling is,
    /// and one reflow rule is how they agree. Combined with
    /// [`Ui::animate_layout`] on one child's height, the stack re-places the
    /// rest on every frame — which is the whole accordion mechanism.
    ///
    /// A hidden child takes no space; children are placed in paint order, so
    /// [`Ui::set_z`] reorders the stack too.
    pub fn set_stack(&mut self, id: NodeId, spacing: i32) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.stack = Some(spacing);
            let root = self.reflow_root(id);
            self.reflow(root);
            self.damage_subtree(root);
        }
    }

    /// Sets which of its parent's edges a node keeps its distance from.
    ///
    /// [`Anchors::TOP_LEFT`] by default — the node keeps its rectangle whatever
    /// its parent does, which is what the tree did before anchoring existed.
    /// See the [`anchor`](crate::anchor) module for what each combination means.
    ///
    /// Not a layout engine: this is one derived rectangle per child, in the
    /// reflow the tree already runs. The node's own `layout` is never rewritten.
    ///
    /// ```
    /// # use denise::{Rect, Size, theme};
    /// # use denise_ui::{Anchors, Ui, Void};
    /// # use denise_ui::widgets::Panel;
    /// let mut ui: Ui<Void> = Ui::new(Size::new(200, 100), theme::DARK);
    /// let root = ui.root();
    /// let bar = ui.add(root, Panel::default(), Rect::new(10, 10, 180, 20)).unwrap();
    ///
    /// // Held at both ends, so it spans whatever width there is.
    /// ui.set_anchors(bar, Anchors::new(true, true, true, false));
    /// assert_eq!(ui.bounds(bar).unwrap().width, 180);
    /// ```
    pub fn set_anchors(&mut self, id: NodeId, anchors: Anchors) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.anchors == anchors {
            return;
        }
        node.anchors = anchors;
        let root = self.reflow_root(id);
        self.damage_subtree(root);
        self.reflow(root);
        self.damage_subtree(root);
        self.clamp_scroll_above(id);
    }

    /// Whether this node is drawn and reachable.
    ///
    /// The counterpart to [`Ui::set_visible`]. A designer needs it: a hidden node
    /// still has bounds — it may be shown again, and its children lay out
    /// against them — so "what is under the pointer" has to ask, or clicking an
    /// empty canvas would select the invisible sheet covering it.
    pub fn visible(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| node.visible)
    }

    /// This node's sort key among its siblings. See [`Ui::set_z`].
    pub fn z(&self, id: NodeId) -> i32 {
        self.nodes.get(id).map_or(0, |node| node.z)
    }

    /// Whether this node takes input and paints as live. See [`Ui::set_enabled`].
    ///
    /// A node whose *parent* is disabled still answers `true` here: this is what
    /// was asked of the node itself, which is what a caller wanting to put it
    /// back needs to know.
    pub fn enabled(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| node.enabled)
    }

    /// The text this node shows on a dwell, if it was given one. See
    /// [`Ui::set_tooltip`].
    pub fn tooltip(&self, id: NodeId) -> Option<&str> {
        self.nodes.get(id).and_then(|node| node.tooltip.as_deref())
    }

    /// Which of its parent's edges this node keeps its distance from.
    pub fn anchors(&self, id: NodeId) -> Anchors {
        self.nodes.get(id).map_or(Anchors::TOP_LEFT, |n| n.anchors)
    }

    /// Gives a node an entire edge of what is left of its parent, or takes it
    /// back with `None`.
    ///
    /// Docked children are placed in paint order, each taking its edge from what
    /// the ones before it left, and everything undocked is placed in what
    /// remains — so docking a bar to the top moves the rest down rather than
    /// covering it. Only the node's extent along the docking axis is used: a
    /// [`Dock::Top`] keeps its `height` and is given the full width.
    ///
    /// ```
    /// # use denise::{Rect, Size, theme};
    /// # use denise_ui::{Dock, Ui, Void};
    /// # use denise_ui::widgets::Panel;
    /// let mut ui: Ui<Void> = Ui::new(Size::new(200, 100), theme::DARK);
    /// let root = ui.root();
    /// let bar = ui.add(root, Panel::default(), Rect::new(0, 0, 0, 24)).unwrap();
    /// let body = ui.add(root, Panel::default(), Rect::new(0, 0, 0, 0)).unwrap();
    ///
    /// ui.set_dock(bar, Some(Dock::Top));
    /// ui.set_dock(body, Some(Dock::Fill));
    /// assert_eq!(ui.bounds(bar).unwrap(), Rect::new(0, 0, 200, 24));
    /// assert_eq!(ui.bounds(body).unwrap(), Rect::new(0, 24, 200, 76));
    /// ```
    pub fn set_dock(&mut self, id: NodeId, dock: Option<Dock>) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.dock == dock {
            return;
        }
        node.dock = dock;
        // Docking changes the box every *sibling* is placed in, so the reflow and
        // the damage start at the parent whether or not it stacks — and climb
        // from there, since the parent may be docked itself.
        let parent = node.parent;
        let root = parent.map_or(id, |p| self.reflow_root(p));
        self.damage_subtree(root);
        self.reflow(root);
        self.damage_subtree(root);
        self.clamp_scroll_above(id);
    }

    /// Which edge of its parent this node takes, if any.
    pub fn dock(&self, id: NodeId) -> Option<Dock> {
        self.nodes.get(id).and_then(|n| n.dock)
    }

    /// Stops stacking: children return to their own layout positions.
    pub fn clear_stack(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id)
            && node.stack.take().is_some()
        {
            let root = self.reflow_root(id);
            self.reflow(root);
            self.damage_subtree(root);
        }
    }

    /// Whether `id` is `ancestor` or sits anywhere under it.
    fn is_descendant_or_self(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.nodes.get(node).and_then(|n| n.parent);
        }
        false
    }

    /// Where a reflow triggered by `id` has to start: the parent when it
    /// stacks, because the change moves siblings, and the node itself
    /// otherwise.
    /// Where a reflow touching `id` has to start.
    ///
    /// Usually `id` itself. But a node whose rectangle depends on its *siblings*
    /// cannot be computed without them, and there are two ways for that to be
    /// true: the node **docks**, so it takes an edge of whatever the siblings
    /// before it left; or its parent **arranges** its children — a stack, or any
    /// docked sibling shrinking the box the rest are placed in.
    ///
    /// Either way the reflow, and the damage, start at the parent. And it climbs:
    /// a docked node inside a docked column depends on its siblings, which depend
    /// on theirs, all the way to whoever is placed on their own terms. Stopping
    /// after one step leaves the node it stopped at holding the rectangle its
    /// `layout` happens to say, which for a docked node is not a position at all.
    fn reflow_root(&self, id: NodeId) -> NodeId {
        let mut at = id;
        loop {
            let Some(node) = self.nodes.get(at) else {
                return at;
            };
            let Some(parent) = node.parent else {
                return at;
            };
            let Some(above) = self.nodes.get(parent) else {
                return at;
            };
            let arranged = node.dock.is_some()
                || above.stack.is_some()
                || above
                    .children
                    .iter()
                    .any(|&c| self.nodes.get(c).is_some_and(|n| n.dock.is_some()));
            if !arranged {
                return at;
            }
            at = parent;
        }
    }

    /// Sets the sibling sort key. Higher paints later, so higher is on top.
    pub fn set_z(&mut self, id: NodeId, z: i32) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.z == z {
            return;
        }
        node.z = z;
        let parent = node.parent;
        if let Some(parent) = parent {
            self.sort_children(parent);
        }
        self.damage_subtree(id);
        // A stack places children in paint order, so reordering moves them.
        let root = self.reflow_root(id);
        if root != id {
            self.reflow(root);
            self.damage_subtree(root);
        }
        self.order_dirty = true;
    }

    /// Shows or hides a node and its descendants.
    pub fn set_visible(&mut self, id: NodeId, visible: bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.visible == visible {
            return;
        }
        node.visible = visible;
        self.damage_subtree(id);
        // In a stack, appearing and disappearing move the siblings below.
        let root = self.reflow_root(id);
        if root != id {
            self.reflow(root);
            self.damage_subtree(root);
        }
        if !visible {
            // A hidden node's layout tween completes instantly: it must not
            // keep the device awake, and half-moved is the one dishonest
            // place to stop. Descendants' tweens too — hiding a panel hides
            // everything it contains.
            let snapping: alloc::vec::Vec<LayoutTween> = self
                .tweens
                .iter()
                .copied()
                .filter(|t| self.is_descendant_or_self(t.id, id))
                .collect();
            self.tweens
                .retain(|t| !snapping.iter().any(|snap| snap.id == t.id));
            for tween in snapping {
                self.apply_layout(tween.id, tween.to);
            }
            self.forget(id);
            // A hidden widget must not keep the device awake: an invisible
            // spinner spinning forever is the exact failure the animation set
            // exists to make visible. Disabling, by contrast, does *not* stop
            // animation — a disabled toggle mid-slide still gets to finish
            // rather than freeze part-way.
            self.stop_animating_subtree(id);
        }
    }

    /// Enables or disables a node and its descendants. Disabled widgets are not
    /// hittable, not focusable, and paint with [`VisualState::DISABLED`].
    pub fn set_enabled(&mut self, id: NodeId, enabled: bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.enabled == enabled {
            return;
        }
        node.enabled = enabled;
        self.reflow(id);
        self.damage_subtree(id);
        if !enabled {
            self.forget(id);
        }
    }

    /// Borrows a widget as its concrete type.
    pub fn widget<W: Widget<M>>(&self, id: NodeId) -> Option<&W> {
        self.nodes.get(id)?.widget.as_any().downcast_ref::<W>()
    }

    /// Borrows a widget mutably as its concrete type, **marking it dirty**.
    ///
    /// Invalidation happens on access rather than on change, because the tree
    /// cannot see what you did through the `&mut`. The cost of being conservative
    /// is repainting one widget; the cost of being clever would be the class of bug
    /// this whole design exists to remove.
    ///
    /// # Do not poll with it
    ///
    /// "Repainting one widget" is the cost of *one* call. Calling it over many
    /// nodes every frame to see whether any of them has something for you costs
    /// a repaint of all of them, every frame — and past
    /// [`MAX_DAMAGE_RECTS`](denise::MAX_DAMAGE_RECTS) rectangles the tracker
    /// collapses to their bounding box, so the answer is not even "sixty small
    /// repaints" but one large one.
    ///
    /// This is not hypothetical: it is how the on-screen keyboard came to
    /// repaint itself on every frame anything else woke the tree for, which on a
    /// panel showed up as a keyboard that flickers. Read through
    /// [`widget`](Ui::widget) to find the node worth writing to, and use this on
    /// that one.
    pub fn widget_mut<W: Widget<M>>(&mut self, id: NodeId) -> Option<&mut W> {
        let clip = self.nodes.get(id)?.clip;
        self.dirty(clip);
        self.nodes
            .get_mut(id)?
            .widget
            .as_any_mut()
            .downcast_mut::<W>()
    }

    // ----------------------------------------------------------- properties

    /// What kind of widget a node holds, as a form file spells it.
    ///
    /// `None` for a widget that does not describe itself — see
    /// [`Widget::describe`].
    pub fn kind(&self, id: NodeId) -> Option<&'static str> {
        self.nodes.get(id)?.widget.describe().map(DynDescribe::kind)
    }

    /// Every property a node's widget accepts.
    ///
    /// Empty for a widget that does not describe itself. A property inspector
    /// walks this to decide which editors to show, so it never names a widget.
    pub fn properties(&self, id: NodeId) -> &'static [Property] {
        self.nodes
            .get(id)
            .and_then(|node| node.widget.describe())
            .map_or(&[], DynDescribe::properties)
    }

    /// The current value of one property.
    ///
    /// `None` for a node that does not exist, a widget that does not describe
    /// itself, a property it does not have, and a property that is simply not
    /// set — [`Describe::get`](crate::widgets::Describe::get) explains why those
    /// last two share an answer.
    pub fn get_property(&self, id: NodeId, name: &str) -> Option<Value> {
        self.nodes.get(id)?.widget.describe()?.get_property(name)
    }

    /// Sets one property by name, and marks the node for repaint.
    ///
    /// The one place a string becomes a typed call on a widget: a form file's
    /// `role=primary` and a property inspector's dropdown arrive here and go no
    /// further apart. An error names the widget, the property and what would
    /// have been accepted.
    ///
    /// Returns `None` if the node does not exist or its widget does not describe
    /// itself; that is a different thing from a property that was refused, which
    /// is the `Err` inside.
    pub fn set_property(
        &mut self,
        id: NodeId,
        name: &str,
        value: Value,
    ) -> Option<Result<(), PropertyError>> {
        let node = self.nodes.get_mut(id)?;
        let result = node.widget.describe_mut()?.set_property(name, value);
        // The widget does not own its damage, so setting a property that changed
        // what it draws would otherwise leave a stale rectangle on the panel.
        // Invalidating unconditionally costs one widget-sized repaint on a
        // no-op, which is the cheap way to be wrong.
        let clip = node.clip;
        self.dirty(clip);
        Some(result)
    }

    // ---------------------------------------------------------------- focus

    /// The node holding keyboard focus.
    #[inline]
    pub const fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// The node under the pointer.
    #[inline]
    pub const fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    /// Moves keyboard focus, refusing nodes that are gone, hidden, disabled,
    /// unfocusable, or in a scene under a modal.
    pub fn focus(&mut self, id: Option<NodeId>) {
        let id = id.filter(|&id| self.is_focusable(id));
        self.set_focus(id);
    }

    /// Topmost node under `p` that accepts pointer input, within the input scene.
    pub fn hit_test(&mut self, p: Point) -> Option<NodeId> {
        self.ensure_order();
        let (start, end) = self.input_span();
        self.order[start..end].iter().rev().copied().find(|&id| {
            self.nodes
                .get(id)
                .is_some_and(|n| n.paintable() && self.is_interactive(n) && n.clip.contains(p))
        })
    }

    /// Dismisses a toast under `p`, reporting whether the press was consumed.
    ///
    /// A toast is not a node, so nothing else would stop the press reaching
    /// what is underneath — and somebody clearing a notification would press
    /// the button it was covering.
    fn dismiss_toast(&mut self, p: Point) -> bool {
        if self.toasts.len() == 0 {
            return false;
        }
        self.damage_toasts();
        let now = self.now_ms;
        self.toasts.dismiss_at(p, self.size, &mut self.text, now)
    }

    /// Whether a press at `p` should close the topmost popup instead of being
    /// delivered: the top scene is a popup and the press is outside its
    /// container.
    /// Whether the topmost scene is an open drawer's — the state in which
    /// Escape and a press on the dim belong to the drawer.
    fn drawer_on_top(&self) -> bool {
        self.drawer.is_some_and(|state| {
            !state.closing
                && self
                    .nodes
                    .get(state.container)
                    .is_some_and(|n| n.scene + 1 == self.scenes.len())
        })
    }

    /// Whether a press at `p` is on the drawer's dim rather than the drawer,
    /// and should close it — swallowed entirely, like a popup's.
    fn dismisses_drawer(&self, p: Point) -> bool {
        self.drawer_on_top()
            && self
                .drawer
                .and_then(|state| self.nodes.get(state.container))
                .is_some_and(|n| !n.bounds.contains(p))
    }

    fn dismisses_popup(&mut self, p: Point) -> bool {
        let Some(popup) = self.scenes.last().and_then(|s| s.popup) else {
            return false;
        };
        self.ensure_order();
        !self
            .nodes
            .get(popup.container)
            .is_some_and(|node| node.clip.contains(p))
    }

    /// The innermost scrollable whose viewport contains `p`, in the scene input
    /// currently reaches. Innermost, so a scrollable inside a scrollable
    /// scrolls the one the pointer is actually over.
    fn scroll_target(&mut self, p: Point) -> Option<NodeId> {
        self.ensure_order();
        let (start, end) = self.input_span();
        self.order[start..end].iter().rev().copied().find(|&id| {
            self.nodes
                .get(id)
                .is_some_and(|n| n.scrollable && n.visible && n.clip.contains(p))
        })
    }

    /// The nearest scrollable ancestor of `id`, itself included.
    fn scrollable_ancestor(&self, id: Option<NodeId>) -> Option<NodeId> {
        let mut current = id;
        while let Some(id) = current {
            let node = self.nodes.get(id)?;
            if node.scrollable {
                return Some(id);
            }
            current = node.parent;
        }
        None
    }

    /// Scrolls ancestors of `id` so that `rect` (absolute) becomes visible in
    /// each of their viewports — the mechanism behind focus following and a
    /// widget's [`EventCtx::reveal`].
    ///
    /// Walks inside-out, so a scrollable inside a scrollable brings the target
    /// into its own viewport first and the outer one then brings *that* into
    /// view.
    /// Scrolls whatever has the focus back into view.
    ///
    /// Focus reveals itself the moment it moves, which is the only moment the
    /// tree can act on unprompted. When what is *around* the focus changes
    /// instead — a keyboard slides up over it, an application gives a page more
    /// room to scroll into — nothing about the focus has changed, so nothing
    /// re-runs the reveal and the caret stays where it was left. This is how an
    /// application says the geometry moved underneath it.
    ///
    /// Nothing focused, or nowhere left to scroll, and it does nothing.
    pub fn reveal_focused(&mut self) {
        let Some(id) = self.focused else {
            return;
        };
        let Some(bounds) = self.nodes.get(id).map(|node| node.bounds) else {
            return;
        };
        self.reveal_rect(id, bounds);
    }

    /// `view` with any occluded band taken off it.
    ///
    /// A shelf lies against one screen edge and spans it, so removing it from a
    /// viewport leaves a rectangle rather than an L — which is what makes this
    /// arithmetic rather than a region.
    ///
    /// Without this, revealing a focused node scrolls it into its viewport and
    /// stops, and a viewport that extends under the keyboard happily reveals a
    /// field underneath it: solved-looking, and not solved.
    fn unoccluded(&self, view: Rect) -> Rect {
        let Some(occ) = self.occluded else {
            return view;
        };
        let screen = Rect::from_size(self.size);
        let (mut top, mut bottom) = (view.y, view.bottom());
        let (mut left, mut right) = (view.x, view.right());
        if occ.width >= screen.width {
            if occ.y <= screen.y {
                top = top.max(occ.bottom());
            } else {
                bottom = bottom.min(occ.y);
            }
        } else if occ.height >= screen.height {
            if occ.x <= screen.x {
                left = left.max(occ.right());
            } else {
                right = right.min(occ.x);
            }
        }
        Rect::new(left, top, (right - left).max(0), (bottom - top).max(0))
    }

    fn reveal_rect(&mut self, id: NodeId, rect: Rect) {
        let mut rect = rect;
        let mut current = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(ancestor) = current {
            let Some(node) = self.nodes.get(ancestor) else {
                return;
            };
            current = node.parent;
            if !node.scrollable {
                continue;
            }
            let view = self.unoccluded(node.bounds);
            let scroll = node.scroll;
            // How far the viewport must move so the rect's near edge is inside.
            // A rect taller than the viewport reveals its top, which is where
            // reading starts.
            let dy = if rect.bottom() > view.bottom() {
                (rect.bottom() - view.bottom()).min(rect.y - view.y)
            } else if rect.y < view.y {
                rect.y - view.y
            } else {
                0
            };
            let dx = if rect.right() > view.right() {
                (rect.right() - view.right()).min(rect.x - view.x)
            } else if rect.x < view.x {
                rect.x - view.x
            } else {
                0
            };
            if dx != 0 || dy != 0 {
                let before = self.scroll(ancestor);
                self.set_scroll(ancestor, Point::new(before.x + dx, before.y + dy));
                let after = self.scroll(ancestor);
                // The rect moved with the content; the outer loop must judge it
                // where it now is.
                rect = rect.translate(before.x - after.x, before.y - after.y);
            }
            let _ = scroll;
        }
    }

    // ---------------------------------------------------------------- input

    /// Routes a batch of input events into the tree.
    pub fn handle(&mut self, events: &[InputEvent]) {
        self.ensure_order();
        for event in events {
            self.handle_one(event);
        }
    }

    /// Advances time-based state for every node that asked to animate.
    ///
    /// A node gets into that set through [`EventCtx::request_animation`] or
    /// [`Ui::request_animation`], and out of it by its own answer: an
    /// [`Animation`](crate::Animation) with [`Wake::Never`] is the widget
    /// saying it is done. The
    /// tree never keeps a widget animating; the widget keeps itself animating,
    /// and the tree keeps the evidence — see [`Ui::animating`].
    ///
    /// **How often** a moving widget is asked is the tree's decision, not the
    /// widget's: a widget answers [`Wake::Animating`] and [`Ui::motion`] turns
    /// that into a time. A widget answering [`Wake::At`] has named a deadline
    /// instead, and the rate does not touch it.
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        let interval = self.motion.interval_ms();
        let mut wake: Option<u64> = None;
        let mut i = 0;
        while i < self.animating.len() {
            let id = self.animating[i];
            let Some(node) = self.nodes.get_mut(id) else {
                // Removed while animating; nothing to settle.
                self.animating.swap_remove(i);
                continue;
            };
            // Under `Motion::None` a widget is asked to land rather than to
            // move, once, and is then expected to have nothing left to do.
            let animation = match interval {
                Some(_) => node.widget.animate(now_ms),
                None => node.widget.snap(now_ms),
            };
            let clip = node.clip;
            if animation.repaint {
                self.dirty(clip);
            }
            // The scene wakes for the most impatient animation, and everybody is
            // asked again at that point. A widget's `animate` must therefore
            // tolerate being called before the time it asked for — all of them
            // already did, because `tick`'s clock was always the caller's.
            let next = match animation.next {
                Wake::Never => None,
                // The saturating add that every widget used to do for itself.
                // `Wake::Animating` under `Motion::None` is a widget that could
                // not land: there is no rate to come back at, so it stops.
                Wake::Animating => interval.map(|ms| now_ms.saturating_add(ms)),
                Wake::At(due) => Some(due),
            };
            match next {
                Some(next) => {
                    wake = Some(wake.map_or(next, |w: u64| w.min(next)));
                    i += 1;
                }
                None => {
                    self.animating.swap_remove(i);
                }
            }
        }

        // Layout tweens: the tree's own animation. Advanced through the same
        // apply path the application's set_layout uses, so damage — the
        // rectangles left behind and the ones now occupied, stacked siblings
        // included — comes along on every frame. A tween that has arrived
        // lands exactly on its target and is gone.
        let mut i = 0;
        while i < self.tweens.len() {
            let tween = self.tweens[i];
            if !self.nodes.contains_key(tween.id) {
                self.tweens.swap_remove(i);
                continue;
            }
            // The tree's own animation, sampled at the tree's own rate — and
            // with no rate at all, a tween is a `set_layout` that happens to
            // have been asked for politely.
            let rect = match interval {
                Some(_) => tween.at(now_ms),
                None => tween.to,
            };
            self.apply_layout(tween.id, rect);
            if rect == tween.to {
                self.tweens.swap_remove(i);
                continue;
            }
            // Only reachable with an interval: a tween with no rate landed on
            // `to` above and is already gone.
            if let Some(ms) = interval {
                let next = now_ms.saturating_add(ms);
                wake = Some(wake.map_or(next, |w: u64| w.min(next)));
            }
            i += 1;
        }

        // A closing drawer pops its scene the moment its slide has landed —
        // the first thing in the tree to happen *because* a tween arrived.
        // Also the cleanup for a drawer whose scene somebody popped directly.
        if let Some(state) = self.drawer {
            if !self.nodes.contains_key(state.container) {
                self.drawer = None;
            } else if state.closing && !self.tweens.iter().any(|t| t.id == state.container) {
                self.drawer = None;
                self.pop_scene();
            }
        }

        // The same landing, for a shelf. It has no scene to pop, so what goes
        // is the node itself — and with it every key that was on it.
        if let Some(state) = self.shelf {
            if !self.nodes.contains_key(state.container) {
                self.shelf = None;
            } else if state.closing && !self.tweens.iter().any(|t| t.id == state.container) {
                self.shelf = None;
                self.remove(state.container);
            }
            if self.shelf.is_none() {
                self.occluded = None;
            }
        }

        // The tooltip's dwell deadline is a wake reason too, and the one most
        // easily forgotten: a kiosk blocks on input until the tree says it
        // wants waking, so a deadline left out here is a bubble that appears
        // the next time something unrelated happens.
        let hovered = self.hovered;
        let anchor = hovered.and_then(|id| self.nodes.get(id)).map(|n| n.bounds);
        let text = hovered
            .and_then(|id| self.nodes.get(id))
            .and_then(|n| n.tooltip.clone());
        if self
            .tooltip
            .tick(now_ms, text.as_deref(), anchor.unwrap_or(Rect::ZERO))
        {
            self.damage_tooltip();
        }
        // Notifications repaint only when they are actually changing — mid-fade,
        // or expiring. A holding toast is a still picture, and damaging it every
        // tick would repaint the bottom of the screen for four seconds to show
        // something that never moved.
        if self.toasts.is_changing(now_ms) {
            self.damage_toasts();
            self.toasts.retire(now_ms);
        }

        // The other two reasons the tree wants waking: a tooltip's dwell
        // deadline and a toast's next frame. Folded in here rather than
        // anywhere else, because this is the one answer the event loop asks
        // for and a deadline left out of it is a feature that never fires.
        for deadline in [self.tooltip.next_wake(), self.toasts.next_wake(now_ms)]
            .into_iter()
            .flatten()
        {
            wake = Some(wake.map_or(deadline, |w: u64| w.min(deadline)));
        }
        self.next_wake = wake;
    }

    /// Damages whatever the notifications cover.
    ///
    /// Measured before any change that would move them, for the reason the
    /// tooltip's damage had to learn: a stack that has already forgotten where
    /// it was cannot say what to repaint.
    fn damage_toasts(&mut self) {
        let now = self.now_ms;
        if let Some(bounds) = self.toasts.bounds(self.size, &mut self.text, now) {
            self.dirty(bounds);
        }
    }

    /// Damages whatever the tooltip covers.
    ///
    /// It is not a node, so nothing else will do it: the bubble sits over
    /// arbitrary widgets and its footprint has to be repainted when it appears
    /// and again when it goes.
    fn damage_tooltip(&mut self) {
        if let Some(bounds) = self.tooltip.bounds(self.size, &mut self.text) {
            self.dirty(bounds);
        }
    }

    /// Asks the tree to start animating `id`.
    ///
    /// The widget's [`Widget::animate`] is called from the next [`Ui::tick`],
    /// and keeps being called until it answers with `next_ms: None`. Wanting
    /// frames is almost always decided inside an event handler, where
    /// [`EventCtx::request_animation`] does this without an id — this entry
    /// point is for the widget that starts moving without being touched, a
    /// spinner being the canonical case.
    ///
    /// # The cost of asking
    ///
    /// A bounded transition — a knob crossing, a toast fading — costs its
    /// duration and then stops asking. An *unbounded* animation is expressible,
    /// because a spinner genuinely is one, and it is exactly what would keep a
    /// kiosk's CPU awake at frame rate for a year if one is left running on a
    /// screen nobody looks at. Hide the node or remove it and the animation
    /// stops with it; [`Ui::animating`] is how a test proves there is nothing
    /// left running.
    pub fn request_animation(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) || self.animating.contains(&id) {
            return;
        }
        self.animating.push(id);
        // Wake immediately: the event loop may already be deciding how long to
        // sleep, and the newly animating widget has not been asked yet.
        self.next_wake = Some(self.next_wake.map_or(self.now_ms, |w| w.min(self.now_ms)));
    }

    /// How fast the tree animates, and whether it does at all.
    #[inline]
    pub const fn motion(&self) -> Motion {
        self.motion
    }

    /// Sets the rate every moving thing in the tree runs at.
    ///
    /// One decision covering spinners, knobs crossing, carousel slides, layout
    /// tweens and toast fades — see [`Motion`] for what it is and is not.
    ///
    /// ```
    /// # use denise::{Size, theme};
    /// # use denise_ui::{Motion, Ui};
    /// # enum Msg { Noop }
    /// # let mut ui: Ui<Msg> = Ui::new(Size::new(1920, 1080), theme::DARK);
    /// ui.set_motion(Motion::Every(33));  // 30 fps: half the wakes
    /// ui.set_motion(Motion::None);       // reduced motion
    /// ```
    ///
    /// # Where the setting belongs
    ///
    /// Here rather than on [`Theme`], although the theme already carries
    /// `metrics` and `depth` and motion tokens would not be absurd beside them.
    /// A theme is an **identity** — swapping dark for light must not change the
    /// power budget — while the frame rate is a **deployment** decision, and the
    /// same panel wants a different answer on a bench and on a battery. Putting
    /// it on the tree also puts it next to the thing it acts on: the animating
    /// set is here, and so is the wake this feeds.
    ///
    /// Takes effect at the next [`Ui::tick`], which is asked for immediately —
    /// an event loop may be blocked on input right now with a sleep it worked
    /// out under the old setting.
    pub fn set_motion(&mut self, motion: Motion) {
        self.motion = motion;
        // The notification stack is not a node, so `tick` cannot reach it the
        // way it reaches widgets.
        self.toasts.set_motion(motion);
        if self.animating() > 0 || self.toasts.len() > 0 {
            self.next_wake = Some(self.now_ms);
        }
    }

    /// How many nodes are currently animating.
    ///
    /// Zero is the number a panel at rest must report, and the README's idle
    /// measurements depend on it. A test that asserts this stays zero is the
    /// guard against a widget quietly holding the device awake.
    #[inline]
    pub fn animating(&self) -> usize {
        self.animating.len() + self.tweens.len()
    }

    /// When something wants to be woken, in the same clock as [`Ui::tick`].
    ///
    /// `None` means nothing is animating and the event loop may block on input
    /// indefinitely, which is the state a kiosk should be in almost all the time.
    #[inline]
    pub const fn next_wake_ms(&self) -> Option<u64> {
        self.next_wake
    }

    /// Messages emitted since the last drain.
    ///
    /// **Drain every frame.** The queue has no ceiling, and it is the one thing
    /// in the tree that does not: toasts cap at three and drop the oldest, damage
    /// coalesces into [`MAX_DAMAGE_RECTS`](denise::MAX_DAMAGE_RECTS) and then
    /// collapses to its bounds, but messages accumulate for as long as an
    /// application keeps handling events without reading them. That is deliberate
    /// — dropping one silently would lose a button press, and no widget can know
    /// which press mattered — so it is the application's contract to keep, and
    /// the failure mode is a slow leak on a panel expected to run for a year.
    ///
    /// An application that deliberately ignores messages for a while should call
    /// this and discard the result rather than let them pile up.
    #[inline]
    pub fn drain_messages(&mut self) -> Drain<'_, M> {
        self.messages.drain(..)
    }

    /// Messages emitted since the last drain, without consuming them.
    #[inline]
    pub fn messages(&self) -> &[M] {
        &self.messages
    }

    // --------------------------------------------------------------- painting

    /// Marks a rectangle for repaint.
    ///
    /// The one door every damaging path in this file goes through, so that
    /// "this frame was nothing but a scroll" stays knowable: anything landing
    /// *inside* a scrolled viewport is a change the union would hide, and hides
    /// it by taking the record away. See [`Scrolled`].
    fn dirty(&mut self, rect: Rect) {
        if self.scrolled[self.scroll_head].is_some_and(|it| it.clip.intersects(&rect)) {
            self.scrolled[self.scroll_head] = None;
        }
        self.damage.add(rect);
    }

    /// Records that a viewport scrolled, for a frame that has done nothing else.
    ///
    /// A second viewport scrolling in the same frame gives up rather than
    /// growing a list: two moving at once is a case worth having and not a case
    /// worth being clever about the first time.
    fn note_scroll(&mut self, node: NodeId, by: Point, clip: Rect) {
        let slot = &mut self.scrolled[self.scroll_head];
        *slot = match *slot {
            None if self.damage.is_clean() => Some(Scrolled { node, by, clip }),
            Some(it) if it.node == node && it.clip == clip => Some(Scrolled {
                node,
                by: Point::new(it.by.x + by.x, it.by.y + by.y),
                clip,
            }),
            _ => None,
        };
    }

    /// Returns `true` if anything has been marked dirty since the last present.
    #[inline]
    pub fn needs_paint(&self) -> bool {
        !self.damage.is_clean()
    }

    /// Marks the whole surface for repaint.
    #[inline]
    pub fn invalidate_all(&mut self) {
        self.scrolled[self.scroll_head] = None;
        self.damage.add_full();
    }

    /// Marks one node's rectangle for repaint.
    pub fn invalidate(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get(id) {
            let clip = node.clip;
            self.dirty(clip);
        }
    }

    /// The regions [`Ui::paint`] last drew. Pass this to
    /// [`Surface::present`](denise::Surface::present).
    #[inline]
    pub fn damage(&self) -> &[Rect] {
        self.damage.resolved()
    }

    /// What has changed since the last present, before [`Ui::paint`] has run.
    ///
    /// [`Ui::damage`] cannot answer this: it reports what `paint` last resolved,
    /// so before this frame is painted it still describes the previous one. A
    /// backend that must know the dirty region *before* drawing — anything
    /// marking damage from `DeniseApp::update`, which happens ahead of `render`
    /// — asks here instead, and gets this frame's rectangles.
    ///
    /// Empty when the whole surface is dirty, which [`Ui::needs_paint`]
    /// distinguishes from nothing being dirty at all.
    #[inline]
    pub fn pending_damage(&self) -> &[Rect] {
        self.damage.pending()
    }

    /// Retires this frame's damage. Call after a successful present.
    #[inline]
    pub fn presented(&mut self) {
        self.damage.end_frame();
        self.scroll_head = (self.scroll_head + 1) % MAX_TRACKED_FRAMES;
        self.scrolled[self.scroll_head] = None;
    }

    /// Moves the rows a scroll left still valid, and hands back the strip that
    /// came into view.
    ///
    /// A viewport scrolled by `dy` has the same content, moved. Copying what is
    /// still good and drawing only what is new turns a 1584x1016 repaint into a
    /// 1584x20 one, which on a Pi at 1080p is the difference between 25 ms and
    /// about 8 — see [#46](https://github.com/bisand/denise/issues/46).
    ///
    /// The copy is *within the buffer being drawn into*, which is what makes
    /// this need nothing new from a backend: that buffer is `age` frames old, so
    /// it holds the content from `age` frames ago, and the scroll since then is
    /// what the ring in [`Scrolled`] has been recording.
    ///
    /// `None` for anything at all uncertain, and every one of these is a case
    /// where the caller repaints the viewport exactly as it always has:
    ///
    /// - the buffer's age is unknown or older than the ring;
    /// - any of those frames did something other than scroll that one viewport;
    /// - the viewport moved or resized in the meantime;
    /// - the scroll was sideways, or further than the viewport is tall;
    /// - anything is drawn *over* it — a scene, a tooltip, a toast, the cursor —
    ///   because an overlay would be copied along with the rows and leave a
    ///   ghost where it used to be;
    /// - the damage is anything but that viewport, so the strip would not be
    ///   the whole of what needs drawing.
    fn scroll_blit(&mut self, frame: &mut Frame<'_>) -> Option<Rect> {
        let frames = match frame.age() {
            denise::BufferAge::Frames(n) if (n as usize) <= MAX_TRACKED_FRAMES => n as usize,
            _ => return None,
        };
        if frames == 0 {
            return None;
        }

        // Every frame this buffer is behind by has to have been the same
        // viewport scrolling, or the content it holds is not what this thinks.
        let first = self.scrolled[self.scroll_head]?;
        let mut moved = Point::new(0, 0);
        for step in 0..frames {
            let slot = (self.scroll_head + MAX_TRACKED_FRAMES - step) % MAX_TRACKED_FRAMES;
            let was = self.scrolled[slot]?;
            if was.node != first.node || was.clip != first.clip {
                return None;
            }
            moved = Point::new(moved.x + was.by.x, moved.y + was.by.y);
        }

        // Sideways is a different copy and a different strip. The case that
        // matters is vertical; the other waits until it does.
        if moved.x != 0 || moved.y == 0 {
            return None;
        }
        if self.scenes.len() > 1 || self.tooltip.is_shown() || self.toasts.len() > 0 {
            return None;
        }
        if self.cursor.bounds().intersects(&first.clip) {
            return None;
        }

        let clip = first.clip.intersect(&Rect::from_size(self.size))?;
        let shift = moved.y.unsigned_abs() as usize;
        let (width, height) = (clip.width.max(0) as usize, clip.height.max(0) as usize);
        if shift == 0 || shift >= height || width == 0 {
            return None;
        }

        // And nothing else may be dirty, or the strip would not cover it.
        let only_the_viewport = {
            let resolved = self.damage.resolve(frame.age());
            resolved.len() == 1 && resolved[0] == first.clip
        };
        if !only_the_viewport {
            return None;
        }

        let stride = frame.stride() as usize;
        let (left, top) = (clip.x.max(0) as usize, clip.y.max(0) as usize);
        let words = frame.pixels_mut();
        // Belt and braces: the clip is inside the surface and the stride covers
        // it, so this holds — and a panic here would be a panic in the paint.
        if (top + height - 1) * stride + left + width > words.len() {
            return None;
        }

        if moved.y > 0 {
            // The content moved up, so row `y` takes what row `y + shift` had.
            // Top to bottom, because the destination trails the source.
            for row in 0..height - shift {
                let from = (top + row + shift) * stride + left;
                words.copy_within(from..from + width, (top + row) * stride + left);
            }
            Some(Rect::new(
                clip.x,
                clip.bottom() - moved.y,
                clip.width,
                moved.y,
            ))
        } else {
            // And the other way, bottom to top for the same reason.
            for row in (shift..height).rev() {
                let from = (top + row - shift) * stride + left;
                words.copy_within(from..from + width, (top + row) * stride + left);
            }
            Some(Rect::new(clip.x, clip.y, clip.width, -moved.y))
        }
    }

    /// Draws every damaged region of the scene stack into `frame`.
    ///
    /// The pipeline, in order: clear, base scene, each further scene over its
    /// backdrop, cursor sprite. All of it inside the damage clip, so an untouched
    /// panel costs nothing and a moved cursor costs two sprite-sized rectangles.
    pub fn paint(&mut self, frame: &mut Frame<'_>) {
        self.ensure_order();

        // The rows a scroll left still valid are moved rather than redrawn, and
        // what comes back is the strip that came into view. What is *reported*
        // as damage is untouched: the screen still needs the whole viewport,
        // because the rows moved in this buffer and not in the one on the panel.
        let blitted = self.scroll_blit(frame);

        let mut regions = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let resolved = self.damage.resolve(frame.age());
            regions[..resolved.len()].copy_from_slice(resolved);
            resolved.len()
        };
        let count = match blitted {
            Some(strip) => {
                regions[0] = strip;
                1
            }
            None => count,
        };

        let base = self.theme.color(Role::Base100);
        let mut canvas = Canvas::new(frame);

        for region in &regions[..count] {
            let mut region_canvas = canvas.with_clip(*region);
            if region_canvas.is_clipped_out() {
                continue;
            }
            region_canvas.clear(base);

            // Only the topmost veil paints. Two modals stacked would otherwise
            // double-dim everything under both, and a popup inside a modal must
            // not darken the modal it serves.
            let top_veil = self.scenes.iter().rposition(|s| s.dim > 0);
            let mut start = 0;
            for (index, scene) in self.scenes.iter().enumerate() {
                let end = self.scene_end[index];
                if scene.dim > 0 && top_veil == Some(index) {
                    let veil = region_canvas.clip();
                    region_canvas.fill_rect(veil, Color::rgba(0, 0, 0, scene.dim));
                }
                for &id in &self.order[start..end] {
                    let Some(node) = self.nodes.get(id) else {
                        continue;
                    };
                    if !node.paintable() || !node.clip.intersects(region) {
                        continue;
                    }
                    let mut ctx = PaintCtx {
                        theme: &self.theme,
                        text: &mut self.text,
                        bounds: node.bounds,
                        state: node.state,
                        now_ms: self.now_ms,
                    };
                    let mut widget_canvas = region_canvas.with_clip(node.clip);
                    node.widget.paint(&mut ctx, &mut widget_canvas);
                }
                start = end;
            }

            self.toasts.paint(
                &self.theme,
                self.size,
                &mut self.text,
                self.now_ms,
                &mut region_canvas,
            );
            // Above every widget, below the pointer: a bubble the cursor
            // covers is a bubble nobody can read.
            self.tooltip
                .paint(&self.theme, self.size, &mut self.text, &mut region_canvas);
            self.cursor.paint(&self.theme, &mut region_canvas);
        }
    }

    /// Acquires, paints and presents in one call. Returns `false` when nothing was
    /// dirty and no frame was drawn.
    pub fn render(&mut self, surface: &mut impl Surface) -> Result<bool, SurfaceError> {
        if !self.needs_paint() {
            return Ok(false);
        }
        let mut frame = surface.acquire()?;
        self.paint(&mut frame);
        drop(frame);
        surface.present(self.damage.resolved())?;
        self.damage.end_frame();
        Ok(true)
    }

    // -------------------------------------------------------------- internals

    fn handle_one(&mut self, event: &InputEvent) {
        // Anything but a bare pointer move means the person moved on.
        if !matches!(event, InputEvent::PointerMoved { .. }) && self.tooltip.dismiss_wanted(event) {
            if self.tooltip.is_shown() {
                self.damage_tooltip();
            }
            self.tooltip.dismiss();
        }
        match event {
            InputEvent::PointerMoved { position } => {
                self.move_pointer(*position, true);
                if let Some(id) = self.pressed.or(self.hovered) {
                    self.dispatch(id, &Event::Input(event));
                }
            }
            InputEvent::PointerButton {
                state: ElementState::Down,
                position,
                ..
            } => {
                if self.dismiss_toast(*position) {
                    return;
                }
                if self.dismisses_popup(*position) {
                    // Swallowed entirely: the press closed the popup, and must
                    // not also reach whatever was underneath. The matching Up
                    // finds nothing pressed and activates nothing.
                    self.close_popup();
                    return;
                }
                if self.dismisses_drawer(*position) {
                    self.close_drawer();
                    return;
                }
                self.move_pointer(*position, true);
                self.press(event);
            }
            InputEvent::PointerButton {
                state: ElementState::Up,
                position,
                ..
            } => {
                self.move_pointer(*position, true);
                self.release(event);
            }
            InputEvent::PointerScroll {
                position,
                delta_x,
                delta_y,
            } => {
                self.move_pointer(*position, true);
                // The hovered widget sees the wheel first — a widget may make
                // it mean something else. Unconsumed, it scrolls the innermost
                // scrollable under the pointer.
                let handled = match self.hovered {
                    Some(id) => self.dispatch(id, &Event::Input(event)).is_handled(),
                    None => false,
                };
                if !handled && let Some(target) = self.scroll_target(*position) {
                    self.scroll_by(target, *delta_x as i32, *delta_y as i32);
                }
            }
            InputEvent::PointerLeft => {
                self.show_cursor(false);
                self.set_hovered(None);
            }
            InputEvent::TouchDown { position, .. } => {
                if self.dismiss_toast(*position) {
                    return;
                }
                if self.dismisses_popup(*position) {
                    self.close_popup();
                    return;
                }
                if self.dismisses_drawer(*position) {
                    self.close_drawer();
                    return;
                }
                self.move_pointer(*position, false);
                self.press(event);
                if self.pressed.is_none() {
                    // Nothing interactive claimed the finger; if it landed in a
                    // viewport, moving it drags the scroll.
                    self.touch_scroll = self.scroll_target(*position).map(|id| (id, *position));
                }
            }
            InputEvent::TouchMoved { position, .. } => {
                self.move_pointer(*position, false);
                if let Some((target, last)) = self.touch_scroll {
                    // Content follows the finger: dragging up moves the scroll
                    // down.
                    self.scroll_by(target, last.x - position.x, last.y - position.y);
                    self.touch_scroll = Some((target, *position));
                } else if let Some(id) = self.pressed {
                    self.dispatch(id, &Event::Input(event));
                }
            }
            InputEvent::TouchUp { position, .. } => {
                self.touch_scroll = None;
                self.move_pointer(*position, false);
                self.release(event);
                self.set_hovered(None);
            }
            InputEvent::Key {
                code: KeyCode::Tab,
                state: ElementState::Down,
                modifiers,
                ..
            } => {
                // Tab belongs to the toolkit, not to the focused widget. A panel
                // with no pointer is driven entirely by this.
                self.focus_step(modifiers.contains(Modifiers::SHIFT));
            }
            InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } if self.scenes.last().is_some_and(|s| s.popup.is_some()) => {
                // Escape belongs to the popup before it belongs to the focused
                // widget: a dropdown open over a text field closes on Escape
                // rather than handing the key to the field behind it.
                self.close_popup();
            }
            InputEvent::Key {
                code: KeyCode::Escape,
                state: ElementState::Down,
                ..
            } if self.drawer_on_top() => {
                // And to the drawer, for the same reason.
                self.close_drawer();
            }
            InputEvent::Key {
                code: code @ (KeyCode::PageUp | KeyCode::PageDown),
                state: ElementState::Down,
                ..
            } => {
                // The focused widget sees the page keys first; unconsumed, they
                // page the scrollable that contains the focus, by its own
                // height.
                let handled = match self.focused {
                    Some(id) => self.dispatch(id, &Event::Input(event)).is_handled(),
                    None => false,
                };
                if !handled && let Some(target) = self.scrollable_ancestor(self.focused) {
                    let page = self.nodes[target].layout.height;
                    let dy = if matches!(code, KeyCode::PageDown) {
                        page
                    } else {
                        -page
                    };
                    self.scroll_by(target, 0, dy);
                }
            }
            InputEvent::Key { .. } | InputEvent::Text { .. } => {
                if let Some(id) = self.focused {
                    self.dispatch(id, &Event::Input(event));
                }
            }
            InputEvent::SurfaceResized { size, .. } => self.resize(*size),
            _ => {}
        }
    }

    fn move_pointer(&mut self, position: Point, show_cursor: bool) {
        // `show_cursor` here is the *input kind* — a pointer wants a sprite, a
        // finger does not. It only decides anything while nobody has said
        // otherwise; see `Ui::show_cursor`.
        let visible = if self.cursor_auto {
            show_cursor
        } else {
            self.cursor.visible
        };
        if self.pointer != position || self.cursor.visible != visible {
            self.dirty(self.cursor.bounds());
            self.pointer = position;
            self.cursor.position = position;
            self.cursor.visible = visible;
            self.dirty(self.cursor.bounds());
        }
        self.update_hover();
    }

    fn update_hover(&mut self) {
        let hit = self.hit_test(self.pointer);
        match self.pressed {
            // While a button is held the hover does not wander off to other
            // widgets, and the pressed one shows its state only while the pointer
            // is still over it — which is how a drag-off-then-release cancels.
            Some(pressed) => {
                let inside = hit == Some(pressed);
                self.set_hovered(inside.then_some(pressed));
                self.set_state(pressed, VisualState::PRESSED, inside);
            }
            None => self.set_hovered(hit),
        }
    }

    fn press(&mut self, event: &InputEvent) {
        let hit = self.hit_test(self.pointer);
        self.pressed = hit;
        match hit {
            Some(id) => {
                self.set_state(id, VisualState::PRESSED, true);
                self.set_hovered(Some(id));
                // A widget that preserves focus is asking to be pressed without
                // being noticed by the focus ring at all — neither taking it nor
                // clearing it, which is what a keyboard key needs while the field
                // it types into stays live.
                if !self
                    .nodes
                    .get(id)
                    .is_some_and(|node| node.widget.preserves_focus())
                {
                    let focus = self.is_focusable(id).then_some(id);
                    self.set_focus(focus);
                }
                self.dispatch(id, &Event::Input(event));
            }
            // Clicking the background drops focus, which is what makes a text
            // field commit and stop blinking.
            None => self.set_focus(None),
        }
    }

    /// Drops a held press that no release will ever arrive for.
    ///
    /// A scene pushed over the pressed node, the scene it lives in popped, its
    /// node removed or disabled: the press is over and nothing in the pointer
    /// stream says so. Clearing the flag alone left the widget looking pressed
    /// and — for one that drives a timer from being held — believing a finger
    /// was still there.
    fn cancel_press(&mut self) {
        let Some(id) = self.pressed.take() else {
            return;
        };
        if !self.nodes.contains_key(id) {
            return;
        }
        self.set_state(id, VisualState::PRESSED, false);
        self.dispatch(id, &Event::PressCancelled);
    }

    fn release(&mut self, event: &InputEvent) {
        if let Some(id) = self.pressed {
            self.set_state(id, VisualState::PRESSED, false);
            self.dispatch(id, &Event::Input(event));
        }
        self.pressed = None;
        self.update_hover();
    }

    /// Delivers an event and applies whatever the widget asked for.
    fn dispatch(&mut self, id: NodeId, event: &Event<'_>) -> Handled {
        let (handled, wants_focus) = self.deliver(id, event);
        if wants_focus && self.is_focusable(id) {
            self.set_focus(Some(id));
        }
        handled
    }

    fn deliver(&mut self, id: NodeId, event: &Event<'_>) -> (Handled, bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return (Handled::No, false);
        };
        if node.state.contains(VisualState::DISABLED) {
            return (Handled::No, false);
        }
        let mut ctx = EventCtx::new(
            node.bounds,
            &self.theme,
            &mut self.text,
            node.state,
            self.now_ms,
            &mut self.messages,
        );
        let handled = node.widget.on_event(event, &mut ctx);
        let (dirty, wants_focus, wants_animation, reveal) = ctx.finish();
        let clip = node.clip;
        if dirty || handled.is_handled() {
            self.dirty(clip);
        }
        if wants_animation {
            self.request_animation(id);
        }
        if let Some(rect) = reveal {
            self.reveal_rect(id, rect);
        }
        (handled, wants_focus)
    }

    fn set_hovered(&mut self, id: Option<NodeId>) {
        if self.hovered == id {
            return;
        }
        // Moving on restarts the dwell, or ends it. The footprint is damaged
        // *first*: every state change here forgets where the bubble was, so
        // measuring afterwards measures nothing and the pixels stay on a
        // display that repaints only what it was told to.
        let has_tooltip = id
            .and_then(|id| self.nodes.get(id))
            .is_some_and(|node| node.tooltip.is_some());
        if self.tooltip.is_shown() {
            self.damage_tooltip();
        }
        self.tooltip.hover_changed(has_tooltip, self.now_ms);
        if let Some(old) = self.hovered {
            self.set_state(old, VisualState::HOVERED, false);
        }
        self.hovered = id;
        if let Some(new) = id {
            self.set_state(new, VisualState::HOVERED, true);
        }
    }

    fn set_focus(&mut self, id: Option<NodeId>) {
        if self.focused == id {
            return;
        }
        if let Some(old) = self.focused {
            self.set_state(old, VisualState::FOCUSED, false);
            self.focused = None;
            self.deliver(old, &Event::FocusLost);
        }
        self.focused = id;
        // One place records it because one place changes it, and the early
        // return above means a focus that did not move is never reported.
        self.focus_changed = Some(id);
        if let Some(new) = id {
            self.set_state(new, VisualState::FOCUSED, true);
            self.deliver(new, &Event::FocusGained);
            // Focus must be visible: tabbing to a widget below the fold scrolls
            // it into view, or a keyboard-only panel focuses something nobody
            // can see.
            if let Some(node) = self.nodes.get(new) {
                let bounds = node.bounds;
                self.reveal_rect(new, bounds);
            }
        }
    }

    fn set_state(&mut self, id: NodeId, flag: VisualState, on: bool) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        if node.state.contains(flag) == on {
            return;
        }
        node.state = node.state.set(flag, on);
        let clip = node.clip;
        self.dirty(clip);
    }

    /// Drops a subtree out of the animating set — used on hide and removal.
    fn stop_animating_subtree(&mut self, id: NodeId) {
        let mut i = 0;
        while i < self.animating.len() {
            if self.subtree_contains(id, Some(self.animating[i])) {
                self.animating.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Drops a node out of hover, press and focus — used when it is removed,
    /// hidden or disabled, so no stale id keeps receiving events.
    fn forget(&mut self, id: NodeId) {
        if self.subtree_contains(id, self.hovered) {
            self.set_hovered(None);
        }
        if self.subtree_contains(id, self.pressed) {
            self.cancel_press();
        }
        if self.subtree_contains(id, self.focused) {
            self.set_focus(None);
        }
    }

    fn subtree_contains(&self, root: NodeId, id: Option<NodeId>) -> bool {
        let Some(mut cursor) = id else {
            return false;
        };
        loop {
            if cursor == root {
                return true;
            }
            match self.nodes.get(cursor).and_then(|n| n.parent) {
                Some(parent) => cursor = parent,
                None => return false,
            }
        }
    }

    fn focus_step(&mut self, backwards: bool) {
        self.ensure_order();
        let (start, end) = self.input_span();
        let mut candidates = Vec::new();
        for &id in &self.order[start..end] {
            if self.is_focusable(id) {
                candidates.push(id);
            }
        }
        if candidates.is_empty() {
            self.set_focus(None);
            return;
        }
        let current = self
            .focused
            .and_then(|f| candidates.iter().position(|&c| c == f));
        let next = match (current, backwards) {
            (Some(i), false) => (i + 1) % candidates.len(),
            (Some(i), true) => (i + candidates.len() - 1) % candidates.len(),
            (None, false) => 0,
            (None, true) => candidates.len() - 1,
        };
        self.set_focus(Some(candidates[next]));
    }

    fn is_focusable(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| {
            // A clipped-out node is still reachable when a scrollable ancestor
            // can bring it back: taking focus is what scrolls it into view, so
            // demanding visibility first would make everything below the fold
            // permanently unreachable by keyboard — the catch-22 the scrolling
            // tests caught on their first run.
            let reachable = node.visible
                && (!node.clip.is_empty() || self.scrollable_ancestor(node.parent).is_some());
            reachable
                && !node.state.contains(VisualState::DISABLED)
                && node.widget.focusable()
                // Only the topmost scene is reachable. Scanning the paint order
                // for this would be O(n) per candidate and O(n²) per Tab; the
                // node already knows which scene it belongs to.
                && node.scene + 1 == self.scenes.len()
        })
    }

    fn is_interactive(&self, node: &Node<M>) -> bool {
        !node.state.contains(VisualState::DISABLED) && node.widget.accepts_pointer()
    }

    /// Range of `order` belonging to the topmost scene.
    fn input_span(&self) -> (usize, usize) {
        match self.scene_end.len() {
            0 => (0, 0),
            1 => (0, self.scene_end[0]),
            n => (self.scene_end[n - 2], self.scene_end[n - 1]),
        }
    }

    fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.damage.resize(size);
        let roots: Vec<NodeId> = self.scenes.iter().map(|s| s.root).collect();
        for root in roots {
            if let Some(node) = self.nodes.get_mut(root) {
                node.layout = Rect::from_size(size);
            }
            self.reflow(root);
        }
        self.damage.add_full();
    }

    fn sort_children(&mut self, parent: NodeId) {
        // Siblings are kept in paint order, so the flatten below is a plain
        // depth-first walk rather than a sort of the whole tree every frame.
        let mut children = match self.nodes.get_mut(parent) {
            Some(node) => core::mem::take(&mut node.children),
            None => return,
        };
        let mut keys: Vec<(i32, NodeId)> = children
            .iter()
            .map(|&id| (self.nodes.get(id).map_or(0, |n| n.z), id))
            .collect();
        keys.sort_by_key(|&(z, _)| z);
        children.clear();
        children.extend(keys.into_iter().map(|(_, id)| id));
        if let Some(node) = self.nodes.get_mut(parent) {
            node.children = children;
        }
    }

    fn ensure_order(&mut self) {
        if !self.order_dirty {
            return;
        }
        self.order.clear();
        self.scene_end.clear();
        let roots: Vec<NodeId> = self.scenes.iter().map(|s| s.root).collect();
        let mut stack = Vec::new();
        for root in roots {
            stack.clear();
            stack.push(root);
            while let Some(id) = stack.pop() {
                let Some(node) = self.nodes.get(id) else {
                    continue;
                };
                self.order.push(id);
                // Reversed, because the stack pops last-in first.
                stack.extend(node.children.iter().rev().copied());
            }
            self.scene_end.push(self.order.len());
        }
        self.order_dirty = false;
    }

    /// A rectangle's extent as a [`Size`], never negative.
    fn extent(rect: Rect) -> Size {
        Size::new(rect.width.max(0) as u32, rect.height.max(0) as u32)
    }

    /// Turns layouts into absolute bounds for a subtree.
    ///
    /// The one place that happens. Four rules meet here and their order is the
    /// contract: **docking** takes edges from the parent's content box, then a
    /// **stack** or **anchoring** places what is left, then the **scroll offset**
    /// shifts all of it. Everything downstream — bounds, clip, paint, damage,
    /// hit testing — reads only what this loop wrote, so none of them can
    /// disagree about where a node ended up.
    ///
    /// No rule rewrites a node's `layout`. Each derives a rectangle from it, so
    /// the application's rectangles stay the application's, and a form file keeps
    /// one rectangle per node however it is being placed.
    fn reflow(&mut self, id: NodeId) {
        // A docked node's rectangle is a statement about its siblings, so it
        // cannot be computed from the node alone. Climbing here rather than at
        // every call site is what makes that true of *every* path into the
        // reflow — a layout set, a stack turned on, a scroll clamped — instead of
        // only the ones that remembered.
        let id = self.reflow_root(id);
        let (rect, clip, disabled) = match self.nodes.get(id).and_then(|n| n.parent) {
            Some(parent) => {
                let Some(node) = self.nodes.get(parent) else {
                    return;
                };
                let (clip, disabled) = (node.clip, node.state.contains(VisualState::DISABLED));
                let origin =
                    Point::new(node.bounds.x - node.scroll.x, node.bounds.y - node.scroll.y);
                let available = Self::extent(node.bounds);
                // Reaching here means the parent does not arrange its children:
                // `reflow_root` sends a node whose parent stacks or docks to the
                // parent instead, because such a node cannot be placed without
                // its siblings. So the box is the parent's whole content box.
                let Some(child) = self.nodes.get(id) else {
                    return;
                };
                let base = child.anchor_base.unwrap_or(available);
                let local = anchor::anchored(child.layout, base, available, child.anchors);
                if let Some(child) = self.nodes.get_mut(id) {
                    child.anchor_base.get_or_insert(available);
                }
                (local.translate(origin.x, origin.y), clip, disabled)
            }
            None => match self.nodes.get(id) {
                Some(node) => (node.layout, Rect::from_size(self.size), false),
                None => return,
            },
        };

        let mut work = vec![(id, rect, clip, disabled)];
        while let Some((id, rect, clip, disabled)) = work.pop() {
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
            node.bounds = rect;
            node.clip = rect.intersect(&clip).unwrap_or(Rect::ZERO);
            let disabled = disabled || !node.enabled;
            node.state = node.state.set(VisualState::DISABLED, disabled);
            // The scroll offset happens here and only here.
            let origin = Point::new(rect.x - node.scroll.x, rect.y - node.scroll.y);
            let child_clip = node.clip;
            let stack = node.stack;
            let children = node.children.clone();

            // Docked children take their edges first, in paint order, each from
            // what the ones before it left — so two bars docked to the top are
            // two stacked bars, and what remains is the box everything else is
            // placed in. A hidden child takes no room, as in a stack.
            let mut remaining = Rect::new(0, 0, rect.width, rect.height);
            for &c in &children {
                let Some(child) = self.nodes.get(c) else {
                    continue;
                };
                let (Some(dock), true) = (child.dock, child.visible) else {
                    continue;
                };
                let (taken, rest) = anchor::docked(child.layout, remaining, dock);
                remaining = rest;
                work.push((c, taken.translate(origin.x, origin.y), child_clip, disabled));
            }

            let available = Self::extent(remaining);
            let mut running = 0i32;
            for c in children {
                let Some(child) = self.nodes.get(c) else {
                    continue;
                };
                if child.dock.is_some() && child.visible {
                    continue;
                }
                let local = match stack {
                    // A stack places its visible children top-to-bottom at the
                    // running y, keeping their own x, width and height — which
                    // is what lets a layout tween on one child move every
                    // sibling below it without anybody keeping books.
                    Some(spacing) if child.visible => {
                        let placed = Rect::new(
                            child.layout.x,
                            running,
                            child.layout.width,
                            child.layout.height,
                        );
                        running = running
                            .saturating_add(child.layout.height.max(0))
                            .saturating_add(spacing);
                        placed
                    }
                    // A hidden child takes no space and moves nobody, but still
                    // needs bounds: it may be shown again, and its own children
                    // are reflowed from them.
                    Some(_) => child.layout,
                    None => {
                        let base = child.anchor_base.unwrap_or(available);
                        let local = anchor::anchored(child.layout, base, available, child.anchors);
                        if let Some(child) = self.nodes.get_mut(c) {
                            child.anchor_base.get_or_insert(available);
                        }
                        local
                    }
                };
                work.push((
                    c,
                    local.translate(origin.x + remaining.x, origin.y + remaining.y),
                    child_clip,
                    disabled,
                ));
            }
        }
    }

    fn damage_subtree(&mut self, id: NodeId) {
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let clip = node.clip;
            stack.extend(node.children.iter().copied());
            self.dirty(clip);
        }
    }

    fn drop_subtree(&mut self, id: NodeId) {
        self.forget(id);
        self.stop_animating_subtree(id);
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.remove(id) else {
                continue;
            };
            stack.extend(node.children.iter().copied());
        }
    }
}

impl<M: 'static> core::fmt::Debug for Ui<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ui")
            .field("nodes", &self.nodes.len())
            .field("scenes", &self.scenes.len())
            .field("size", &self.size)
            .field("theme", &self.theme.name)
            .field("glyphs", &self.text.atlas().len())
            .field("focused", &self.focused)
            .field("hovered", &self.hovered)
            .finish_non_exhaustive()
    }
}
