#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use denise::Rect;
use denise_ui::{NodeId, Offer, Ui};

/// Which way a container lays its children out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    /// Left to right. The main axis is the width.
    #[default]
    Row,
    /// Top to bottom. The main axis is the height.
    Column,
    /// All on top of each other: every child gets the whole content box.
    ///
    /// Deliberately not called a *stack*, because
    /// [`Ui::set_stack`](denise_ui::Ui::set_stack) already means top-to-bottom
    /// and two meanings of one word in one workspace is a trap.
    Layer,
}

impl Flow {
    /// The main-axis extent of a rectangle under this flow.
    const fn main(self, rect: Rect) -> i32 {
        match self {
            Self::Row => rect.width,
            Self::Column | Self::Layer => rect.height,
        }
    }
}

/// How much of the main axis a child takes.
///
/// The cross axis is not a choice: a child fills it. See the crate docs for why
/// that is the whole of the alignment story.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sizing {
    /// This many pixels, whatever else happens.
    Fixed(i32),
    /// A share of what is left once the fixed and hugging children have taken
    /// theirs, in proportion to the weights of the other flex children.
    ///
    /// A weight of `0` takes nothing, which is a way to park a child without
    /// removing it.
    Flex(u16),
    /// Whatever the child says it wants to be, through
    /// [`Widget::measure`](denise_ui::Widget::measure).
    ///
    /// A node with no opinion — a panel, an image — hugs to nothing, which is
    /// visible immediately rather than silently wrong.
    Hug,
}

/// A container in an [`Arrange`], to add children to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group(usize);

/// What one slot in the arena is.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// A node of the tree.
    Node(NodeId),
    /// A container, by its index into `groups`.
    Group(usize),
}

#[derive(Clone, Copy, Debug)]
struct Child {
    kind: Kind,
    sizing: Sizing,
}

#[derive(Clone, Debug)]
struct Container {
    flow: Flow,
    padding: i32,
    gap: i32,
    children: Vec<Child>,
    /// The node whose rectangle this container occupies, if it has one. `None`
    /// for the root, which is given a rectangle by the caller.
    node: Option<NodeId>,
}

/// An arrangement: containers, their children, and how each child is sized.
///
/// Built once, applied as often as you like. Applying computes rectangles and
/// writes them with [`Ui::set_layout`] — exactly what an application doing its
/// own arithmetic would call, which is what keeps this a layer *over* the tree.
///
/// ```
/// # use denise::{Rect, Size, theme};
/// # use denise_ui::{Ui, Void, widgets::{Button, Label, Panel}};
/// # use denise_arrange::{Arrange, Flow, Sizing};
/// let mut ui: Ui<Void> = Ui::new(Size::new(400, 200), theme::DARK);
/// let root = ui.root();
/// let bar = ui.add(root, Panel::default(), Rect::new(0, 0, 400, 44)).unwrap();
/// let title = ui.add(bar, Label::new("Settings"), Rect::ZERO).unwrap();
/// let spacer = ui.add(bar, Panel::default(), Rect::ZERO).unwrap();
/// let save = ui.add(bar, Button::<Void>::inert("Save"), Rect::ZERO).unwrap();
///
/// let mut arrange = Arrange::new(Flow::Row);
/// let row = arrange.root();
/// arrange.set_padding(row, 8);
/// arrange.set_gap(row, 8);
/// arrange.node(row, title, Sizing::Hug);      // as wide as its text
/// arrange.node(row, spacer, Sizing::Flex(1)); // everything left over
/// arrange.node(row, save, Sizing::Hug);       // as wide as its label
///
/// arrange.apply(&mut ui, Rect::new(0, 0, 400, 44));
///
/// // The three fill the row between the paddings, in order, with the gaps.
/// let title_rect = ui.layout(title).unwrap();
/// let save_rect = ui.layout(save).unwrap();
/// assert_eq!(title_rect.x, 8, "the padding");
/// assert_eq!(save_rect.right(), 400 - 8, "flush against the far padding");
/// assert_eq!(title_rect.height, 44 - 8 * 2, "children fill the cross axis");
/// ```
#[derive(Clone, Debug)]
pub struct Arrange {
    groups: Vec<Container>,
}

impl Arrange {
    /// A new arrangement whose root container flows this way.
    #[must_use]
    pub fn new(flow: Flow) -> Self {
        Self {
            groups: alloc::vec![Container {
                flow,
                padding: 0,
                gap: 0,
                children: Vec::new(),
                node: None,
            }],
        }
    }

    /// The root container. Its rectangle is the one given to [`Arrange::apply`].
    #[must_use]
    pub const fn root(&self) -> Group {
        Group(0)
    }

    /// Space inside a container, on every side.
    pub fn set_padding(&mut self, group: Group, padding: i32) {
        if let Some(container) = self.groups.get_mut(group.0) {
            container.padding = padding.max(0);
        }
    }

    /// Space between a container's children. Not before the first or after the
    /// last — that is what padding is for.
    pub fn set_gap(&mut self, group: Group, gap: i32) {
        if let Some(container) = self.groups.get_mut(group.0) {
            container.gap = gap.max(0);
        }
    }

    /// Adds a node of the tree as a child.
    pub fn node(&mut self, parent: Group, id: NodeId, sizing: Sizing) {
        self.push(parent, Kind::Node(id), sizing);
    }

    /// Adds a nested container as a child, sized like any other.
    ///
    /// Give it `node` when a real node of the tree is the container — a
    /// [`Panel`](denise_ui::widgets::Panel) holding a row of buttons — and that
    /// node gets the container's rectangle. Give it `None` for a grouping that
    /// exists only in this arrangement.
    pub fn group(
        &mut self,
        parent: Group,
        flow: Flow,
        sizing: Sizing,
        node: Option<NodeId>,
    ) -> Group {
        let index = self.groups.len();
        self.groups.push(Container {
            flow,
            padding: 0,
            gap: 0,
            children: Vec::new(),
            node,
        });
        self.push(parent, Kind::Group(index), sizing);
        Group(index)
    }

    fn push(&mut self, parent: Group, kind: Kind, sizing: Sizing) {
        if let Some(container) = self.groups.get_mut(parent.0) {
            container.children.push(Child { kind, sizing });
        }
    }

    /// Computes every rectangle and writes it with [`Ui::set_layout`].
    ///
    /// `within` is the root container's rectangle, in the coordinates the root's
    /// children are placed in — which for a child of node `p` is `p`'s content
    /// box with its origin at zero, the same space
    /// [`Ui::set_layout`] already takes.
    ///
    /// Measuring happens here, so calling this again after content changed picks
    /// the change up. Nothing caches, and nothing runs unless you call it.
    pub fn apply<M>(&self, ui: &mut Ui<M>, within: Rect) {
        self.lay_out(ui, 0, within);
    }

    /// Places one container's children inside `box_of`, and recurses.
    fn lay_out<M>(&self, ui: &mut Ui<M>, index: usize, box_of: Rect) {
        let Some(container) = self.groups.get(index) else {
            return;
        };
        let pad = container.padding;
        let content = Rect::from_edges(
            box_of.x + pad,
            box_of.y + pad,
            (box_of.right() - pad).max(box_of.x + pad),
            (box_of.bottom() - pad).max(box_of.y + pad),
        );

        if container.flow == Flow::Layer {
            for child in &container.children {
                self.place(ui, child, content);
            }
            return;
        }

        // Pass one: what each child takes of the main axis, before the leftover
        // is shared out. A flex child takes nothing yet.
        let cross = match container.flow {
            Flow::Row => Offer::tall(content.height),
            Flow::Column | Flow::Layer => Offer::wide(content.width),
        };
        let mut taken: Vec<i32> = Vec::with_capacity(container.children.len());
        let mut weights: u32 = 0;
        for child in &container.children {
            let extent = match child.sizing {
                Sizing::Fixed(n) => n.max(0),
                Sizing::Flex(weight) => {
                    weights += u32::from(weight);
                    0
                }
                Sizing::Hug => self.hug(ui, child, container.flow, cross),
            };
            taken.push(extent);
        }

        let gaps = container
            .gap
            .saturating_mul((container.children.len().max(1) - 1) as i32);
        let used: i32 = taken.iter().copied().sum::<i32>().saturating_add(gaps);
        let spare = (container.flow.main(content) - used).max(0);

        // Pass two: share the leftover among the flex children and place them.
        // The last flex child takes the rounding, so the row ends flush against
        // the padding rather than a pixel or two short of it.
        let mut handed = 0;
        let mut remaining = weights;
        for (child, extent) in container.children.iter().zip(&mut taken) {
            if let Sizing::Flex(weight) = child.sizing {
                let weight = u32::from(weight);
                *extent = if weight == 0 || weights == 0 {
                    0
                } else if weight == remaining {
                    spare - handed
                } else {
                    let share = (i64::from(spare) * i64::from(weight) / i64::from(weights)) as i32;
                    handed += share;
                    share
                };
                remaining -= weight;
            }
        }

        let mut at = match container.flow {
            Flow::Row => content.x,
            Flow::Column | Flow::Layer => content.y,
        };
        for (child, extent) in container.children.iter().zip(&taken) {
            let rect = match container.flow {
                Flow::Row => Rect::new(at, content.y, *extent, content.height),
                Flow::Column | Flow::Layer => Rect::new(content.x, at, content.width, *extent),
            };
            self.place(ui, child, rect);
            at = at.saturating_add(*extent).saturating_add(container.gap);
        }
    }

    /// Writes one child's rectangle, and lays a container's own children out.
    fn place<M>(&self, ui: &mut Ui<M>, child: &Child, rect: Rect) {
        match child.kind {
            Kind::Node(id) => ui.set_layout(id, rect),
            Kind::Group(index) => {
                if let Some(node) = self.groups.get(index).and_then(|c| c.node) {
                    ui.set_layout(node, rect);
                    // A container that *is* a node places its children inside
                    // that node, so their coordinates start at its origin.
                    self.lay_out(ui, index, Rect::new(0, 0, rect.width, rect.height));
                } else {
                    self.lay_out(ui, index, rect);
                }
            }
        }
    }

    /// What a hugging child wants along the main axis.
    fn hug<M>(&self, ui: &mut Ui<M>, child: &Child, flow: Flow, cross: Offer) -> i32 {
        match child.kind {
            Kind::Node(id) => {
                let wanted = ui.measure(id, cross);
                match flow {
                    Flow::Row => wanted.width,
                    Flow::Column | Flow::Layer => wanted.height,
                }
                .unwrap_or(0)
                .max(0)
            }
            // A container hugs to the sum of what its own children want. A flex
            // child inside one contributes nothing, because "a share of what is
            // left" has no answer when nothing has been left yet.
            Kind::Group(index) => self.natural(ui, index, flow, cross),
        }
    }

    /// A container's own preferred extent along `flow`.
    fn natural<M>(&self, ui: &mut Ui<M>, index: usize, flow: Flow, cross: Offer) -> i32 {
        let Some(container) = self.groups.get(index) else {
            return 0;
        };
        let pad = container.padding.saturating_mul(2);
        let inner_cross = match (container.flow, cross) {
            (Flow::Row, Offer { height, .. }) => Offer {
                width: None,
                height: height.map(|h| (h - pad).max(0)),
            },
            (_, Offer { width, .. }) => Offer {
                width: width.map(|w| (w - pad).max(0)),
                height: None,
            },
        };

        let mut total = 0i32;
        let mut widest = 0i32;
        for child in &container.children {
            let extent = match child.sizing {
                Sizing::Fixed(n) => n.max(0),
                Sizing::Flex(_) => 0,
                Sizing::Hug => self.hug(ui, child, container.flow, inner_cross),
            };
            total = total.saturating_add(extent);
            widest = widest.max(extent);
        }
        let gaps = container
            .gap
            .saturating_mul((container.children.len().max(1) - 1) as i32);

        // Along its own flow the extents add up; across it, or for a layer, the
        // largest child is the answer.
        let along = match container.flow {
            Flow::Layer => widest,
            _ => total.saturating_add(gaps),
        };
        if container.flow == flow || container.flow == Flow::Layer {
            along.saturating_add(pad)
        } else {
            widest.saturating_add(pad)
        }
    }
}
