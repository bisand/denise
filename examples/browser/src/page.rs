//! Where layout output becomes a Denise tree.
//!
//! One `Void` wrapper per page under the scrollable viewport, every leaf a
//! direct child of it with the rectangle layout gave it — flat, not nested,
//! because the toolkit's scroll range is derived from children's rects and a
//! flat page keeps the node count at "one per visible thing". Tearing a page
//! down is [`Ui::remove`] on the wrapper; the subtree goes with it.

use std::collections::HashMap;

use denise::{Rect, Size};
use denise_text::TextStyle;
use denise_ui::widgets::{Checkbox, Divider, Fit, Image, RadioGroup, Select, TextInput};
use denise_ui::{NodeId, Ui, Void};
use url::Url;

use crate::app::Message;
use crate::forms::{FormsModel, RenderControl};
use crate::layout::{Leaf, PageLayout};
use crate::textflow::{BulletMark, Filler, TextFlow};

pub struct Page {
    /// The wrapper: remove this and the page is gone.
    pub root: NodeId,
    /// Absolute targets, aligned with the link indices baked into the flows.
    /// `None` is an href that would not resolve; clicking it does nothing.
    pub links: Vec<Option<Url>>,
    /// The `<img>` nodes waiting for pixels, in document order.
    pub images: Vec<ImageJob>,
    /// The form model, read at submit time.
    pub forms: FormsModel,
    /// Control element to the widget node holding its live value.
    pub controls: HashMap<usize, NodeId>,
    /// Select nodes by the index their open-message carries.
    pub selects: Vec<NodeId>,
    /// What relative form actions resolve against.
    pub base: Url,
    /// Element ids to their content-relative y, for `#fragment` scrolling.
    pub anchors: Vec<(String, i32)>,
}

fn checkbox_toggled(_: bool) -> Message {
    Message::Noop
}
fn radio_chosen(_: usize) -> Message {
    Message::Noop
}

pub struct ImageJob {
    /// The element, for pairing an arriving response with its box.
    pub dom: usize,
    /// Unresolved, exactly as the page wrote it.
    pub src: String,
    /// The box already fits — filling it will not move the text.
    pub sized: bool,
    pub node: NodeId,
}

// Eight arguments for the same reason `layout_page` has them: a page is
// that many things, and a struct would only rename the count.
#[allow(clippy::too_many_arguments)]
pub fn build(
    ui: &mut Ui<Message>,
    viewport: NodeId,
    width: i32,
    layout: PageLayout,
    links: Vec<Option<Url>>,
    forms: FormsModel,
    base: Url,
    style: TextStyle,
) -> Page {
    let root = ui
        .add(viewport, Void, Rect::new(0, 0, width, layout.height))
        .expect("the viewport exists");
    let anchors = layout.anchors;
    let mut images = Vec::new();
    let mut controls = HashMap::new();
    let mut selects = Vec::new();
    for placed in layout.leaves {
        let rect = placed.rect;
        match placed.leaf {
            Leaf::Background(color) => {
                ui.add(root, Filler(color), rect);
            }
            Leaf::Flow(flow) => {
                ui.add(root, TextFlow::new(flow), rect);
            }
            Leaf::Bullet { text, style, color } => {
                ui.add(root, BulletMark { text, style, color }, rect);
            }
            Leaf::Rule => {
                ui.add(root, Divider::new(), rect);
            }
            Leaf::Control { dom } => {
                let Some(render) = forms.render.get(&dom) else {
                    continue;
                };
                // Each HTML control becomes the Denise widget it always
                // resembled — which is the demonstration this example is
                // for. Change messages are Noop: values are read from the
                // widgets at submit time, not mirrored anywhere.
                let node = match render {
                    RenderControl::Text {
                        value,
                        placeholder,
                        password,
                        form,
                        ..
                    } => {
                        let input = TextInput::<Message>::new()
                            .with_placeholder(placeholder.clone())
                            .with_max_chars(4096)
                            .with_style(style)
                            .with_password(*password)
                            .with_submit(Message::SubmitForm(*form));
                        let node = ui.add(root, input, rect).expect("the wrapper exists");
                        if !value.is_empty()
                            && let Some(widget) = ui.widget_mut::<TextInput<Message>>(node)
                        {
                            widget.set_text(value.clone());
                        }
                        node
                    }
                    RenderControl::Checkbox { checked } => ui
                        .add(
                            root,
                            Checkbox::new("", checkbox_toggled)
                                .with_checked(*checked)
                                .with_style(style),
                            rect,
                        )
                        .expect("the wrapper exists"),
                    RenderControl::Radio { labels, selected } => ui
                        .add(
                            root,
                            RadioGroup::new(labels.clone(), radio_chosen)
                                .with_selected(*selected)
                                .with_style(style),
                            rect,
                        )
                        .expect("the wrapper exists"),
                    RenderControl::Select { labels, selected } => {
                        let open = Message::OpenSelect(selects.len());
                        let node = ui
                            .add(
                                root,
                                Select::new(labels.clone(), open)
                                    .with_selected(*selected)
                                    .with_style(style),
                                rect,
                            )
                            .expect("the wrapper exists");
                        selects.push(node);
                        node
                    }
                    RenderControl::Button { label, form } => ui
                        .add(
                            root,
                            denise_ui::widgets::Button::new(
                                label.clone(),
                                Message::SubmitForm(*form),
                            )
                            .with_style(style),
                            rect,
                        )
                        .expect("the wrapper exists"),
                };
                controls.insert(dom, node);
            }
            Leaf::Image { dom, src, sized } => {
                // One transparent pixel until the bytes come: the widget is
                // the reservation, `set_pixels` is the arrival.
                let node = ui
                    .add(
                        root,
                        Image::new(vec![0], Size::new(1, 1)).with_fit(Fit::Contain),
                        rect,
                    )
                    .expect("the wrapper exists");
                images.push(ImageJob {
                    dom,
                    src,
                    sized,
                    node,
                });
            }
        }
    }
    Page {
        root,
        links,
        images,
        forms,
        controls,
        selects,
        base,
        anchors,
    }
}
