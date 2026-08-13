//! The tree, the scene stack, and the compositor that turns them into pixels.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::{Drain, Vec};

use denise::{
    Color, DamageTracker, ElementState, Frame, InputEvent, KeyCode, MAX_DAMAGE_RECTS, Modifiers,
    Point, Rect, Role, Size, Surface, SurfaceError, Theme,
};
use denise_render::Canvas;
use denise_text::{FontId, GlyphSource, TextEngine};
use slotmap::SlotMap;

use crate::cursor::{Cursor, CursorImage};
use crate::node::{Node, NodeId, Popup, Scene};

/// Space between a popup and its anchor, in pixels. Small on purpose: a
/// dropdown visually belongs to its button, and a gap wide enough to see the
/// page through reads as two unrelated panels.
const POPUP_GAP: i32 = 4;
use crate::widget::{Event, EventCtx, Handled, PaintCtx, VisualState, Void, Widget};

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
    /// a widget returning `next_ms: None` drops out. Kept deliberately small and
    /// deliberately visible — [`Ui::animating`] exists so a test can assert a
    /// tree at rest holds nobody awake.
    animating: Vec<NodeId>,
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
            pointer: Point::ZERO,
            hovered: None,
            pressed: None,
            touch_scroll: None,
            focused: None,
            cursor: Cursor::default(),
            messages: Vec::new(),
            now_ms: 0,
            next_wake: None,
            animating: Vec::new(),
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
        self.damage.add(self.cursor.bounds());
        self.cursor.image = image;
        self.damage.add(self.cursor.bounds());
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
        self.damage.add(self.cursor.bounds());
        self.cursor.visible = visible;
        self.damage.add(self.cursor.bounds());
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
        self.pressed = None;
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
                    self.damage.add(clip);
                }
            }
        }
        self.drop_subtree(scene.root);
        self.order_dirty = true;
        self.set_focus(None);
        self.pressed = None;
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
        self.reflow(id);
        self.damage_subtree(id);
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
        if let Some(parent) = self.nodes[id].parent
            && let Some(node) = self.nodes.get_mut(parent)
        {
            node.children.retain(|&c| c != id);
        }
        self.drop_subtree(id);
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
        for &child in &node.children {
            if let Some(child) = self.nodes.get(child) {
                right = right.max(child.layout.right());
                bottom = bottom.max(child.layout.bottom());
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
        node.scroll = clamped;
        let clip = node.clip;
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

    /// Moves or resizes a node, damaging the rectangles it left and the ones it
    /// now occupies.
    pub fn set_layout(&mut self, id: NodeId, layout: Rect) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        if node.layout == layout {
            return;
        }
        self.damage_subtree(id);
        self.nodes[id].layout = layout;
        self.reflow(id);
        self.damage_subtree(id);
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
        if !visible {
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
    pub fn widget_mut<W: Widget<M>>(&mut self, id: NodeId) -> Option<&mut W> {
        let clip = self.nodes.get(id)?.clip;
        self.damage.add(clip);
        self.nodes
            .get_mut(id)?
            .widget
            .as_any_mut()
            .downcast_mut::<W>()
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

    /// Whether a press at `p` should close the topmost popup instead of being
    /// delivered: the top scene is a popup and the press is outside its
    /// container.
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
            let view = node.bounds;
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
    /// [`Animation`] with `next_ms: None` is the widget saying it is done. The
    /// tree never keeps a widget animating; the widget keeps itself animating,
    /// and the tree keeps the evidence — see [`Ui::animating`].
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        let mut wake: Option<u64> = None;
        let mut i = 0;
        while i < self.animating.len() {
            let id = self.animating[i];
            let Some(node) = self.nodes.get_mut(id) else {
                // Removed while animating; nothing to settle.
                self.animating.swap_remove(i);
                continue;
            };
            let animation = node.widget.animate(now_ms);
            let clip = node.clip;
            if animation.repaint {
                self.damage.add(clip);
            }
            match animation.next_ms {
                Some(next) => {
                    // The scene wakes for the most impatient animation, and
                    // everybody is asked again at that point. A widget's
                    // `animate` must therefore tolerate being called before the
                    // time it asked for — all of them already did, because
                    // `tick`'s clock was always the caller's.
                    wake = Some(wake.map_or(next, |w: u64| w.min(next)));
                    i += 1;
                }
                None => {
                    self.animating.swap_remove(i);
                }
            }
        }
        self.next_wake = wake;
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

    /// How many nodes are currently animating.
    ///
    /// Zero is the number a panel at rest must report, and the README's idle
    /// measurements depend on it. A test that asserts this stays zero is the
    /// guard against a widget quietly holding the device awake.
    #[inline]
    pub fn animating(&self) -> usize {
        self.animating.len()
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

    /// Returns `true` if anything has been marked dirty since the last present.
    #[inline]
    pub fn needs_paint(&self) -> bool {
        !self.damage.is_clean()
    }

    /// Marks the whole surface for repaint.
    #[inline]
    pub fn invalidate_all(&mut self) {
        self.damage.add_full();
    }

    /// Marks one node's rectangle for repaint.
    pub fn invalidate(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get(id) {
            let clip = node.clip;
            self.damage.add(clip);
        }
    }

    /// The regions [`Ui::paint`] last drew. Pass this to
    /// [`Surface::present`](denise::Surface::present).
    #[inline]
    pub fn damage(&self) -> &[Rect] {
        self.damage.resolved()
    }

    /// Retires this frame's damage. Call after a successful present.
    #[inline]
    pub fn presented(&mut self) {
        self.damage.end_frame();
    }

    /// Draws every damaged region of the scene stack into `frame`.
    ///
    /// The pipeline, in order: clear, base scene, each further scene over its
    /// backdrop, cursor sprite. All of it inside the damage clip, so an untouched
    /// panel costs nothing and a moved cursor costs two sprite-sized rectangles.
    pub fn paint(&mut self, frame: &mut Frame<'_>) {
        self.ensure_order();

        let mut regions = [Rect::ZERO; MAX_DAMAGE_RECTS];
        let count = {
            let resolved = self.damage.resolve(frame.age());
            regions[..resolved.len()].copy_from_slice(resolved);
            resolved.len()
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
                if self.dismisses_popup(*position) {
                    // Swallowed entirely: the press closed the popup, and must
                    // not also reach whatever was underneath. The matching Up
                    // finds nothing pressed and activates nothing.
                    self.close_popup();
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
                if self.dismisses_popup(*position) {
                    self.close_popup();
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
            self.damage.add(self.cursor.bounds());
            self.pointer = position;
            self.cursor.position = position;
            self.cursor.visible = visible;
            self.damage.add(self.cursor.bounds());
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
                let focus = self.is_focusable(id).then_some(id);
                self.set_focus(focus);
                self.dispatch(id, &Event::Input(event));
            }
            // Clicking the background drops focus, which is what makes a text
            // field commit and stop blinking.
            None => self.set_focus(None),
        }
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
            self.damage.add(clip);
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
        self.damage.add(clip);
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
            self.pressed = None;
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

    fn reflow(&mut self, id: NodeId) {
        let (origin, clip, disabled) = match self.nodes.get(id).and_then(|n| n.parent) {
            Some(parent) => match self.nodes.get(parent) {
                Some(node) => (
                    Point::new(node.bounds.x, node.bounds.y),
                    node.clip,
                    node.state.contains(VisualState::DISABLED),
                ),
                None => return,
            },
            None => (Point::ZERO, Rect::from_size(self.size), false),
        };

        let mut stack = vec![(id, origin, clip, disabled)];
        while let Some((id, origin, clip, disabled)) = stack.pop() {
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
            node.bounds = node.layout.translate(origin.x, origin.y);
            node.clip = node.bounds.intersect(&clip).unwrap_or(Rect::ZERO);
            let disabled = disabled || !node.enabled;
            node.state = node.state.set(VisualState::DISABLED, disabled);
            // The scroll offset happens here and only here. Children lay out
            // against a shifted origin, and everything downstream — bounds,
            // clip, paint, hit testing — reads the fields this loop wrote, so
            // nothing can disagree about where a scrolled child is.
            let child_origin =
                Point::new(node.bounds.x - node.scroll.x, node.bounds.y - node.scroll.y);
            let child_clip = node.clip;
            stack.extend(
                node.children
                    .iter()
                    .map(|&c| (c, child_origin, child_clip, disabled)),
            );
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
            self.damage.add(clip);
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
