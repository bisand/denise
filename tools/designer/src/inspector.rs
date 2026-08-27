//! The right pane: every property of the selection, with an editor fit for it.
//!
//! There is no table of widgets here and there must never be one. A row exists
//! because a widget's [`Describe`] implementation says the property exists, and
//! its editor is chosen from the [`PropertyKind`] alone — so a twenty-sixth
//! widget, or a twenty-seventh property on an existing one, appears in this pane
//! without a line of this file changing.
//!
//! The properties the *tree* owns — `x`, `visible`, `dock` — come from
//! [`denise_forms::NODE_PROPERTIES`], described the same way for the same
//! reason.
//!
//! # Nothing here holds a value
//!
//! A row remembers only the text it last *showed*. Every frame the designer asks
//! each editor what it now holds, and a difference is somebody having edited it.
//! That is why typing applies live without a message per keystroke: this toolkit's
//! widgets take `fn(T) -> M` function pointers, which cannot carry a row index,
//! and polling twenty small values is cheaper than the machinery that would.
//!
//! [`Describe`]: denise_ui::widgets::Describe

use denise::{Rect, Role};
use denise_ui::widgets::{Button, Checkbox, Label, Property, PropertyKind, Select, Slider, Value};
use denise_ui::widgets::{Panel, TextInput};
use denise_ui::{NodeId, TextStyle, Ui};

use crate::app::Message;
use crate::scale::Scale;
use crate::text::Text;

/// A row's height, and what separates it from the next.
///
/// Logical, like every constant in this crate; [`Scale`] multiplies them on the
/// way into the tree. `width` and the rectangles handed to `build_editor` are
/// already physical, so those two meet in the middle.
const ROW: i32 = 24;
const GAP: i32 = 4;
/// The property's name, down the left.
const NAME: i32 = 88;
/// The reset control, at the right edge.
const RESET: i32 = 18;
/// The control that opens a file dialog, on an asset row.
const BROWSE: i32 = 22;

/// How wide a range may be before a slider stops being worth its pixels.
///
/// `x` is an `Int` between -8192 and 8192; a hundred-pixel slider over that
/// moves eighty units a pixel, which is a control that cannot be aimed. A
/// `Progress` between 0 and 1 is the case this is for.
const SLIDABLE: f32 = 1000.0;

/// What edits one property.
#[derive(Debug)]
pub enum Editor {
    /// A field holding the value as text.
    Field(NodeId),
    /// A field, and a slider beside it over a range narrow enough to aim.
    Slid {
        /// The field.
        field: NodeId,
        /// The slider.
        slider: NodeId,
    },
    /// A box.
    Flag(NodeId),
    /// A dropdown over a fixed set of names.
    Choice {
        /// The closed control.
        select: NodeId,
        /// What it offers, in the order it offers them.
        options: &'static [&'static str],
    },
}

impl Editor {
    /// The node that takes the caret, for telling when somebody has moved on.
    pub const fn focusable(&self) -> NodeId {
        match self {
            Editor::Field(id) | Editor::Slid { field: id, .. } => *id,
            Editor::Flag(id) => *id,
            Editor::Choice { select, .. } => *select,
        }
    }
}

/// One property of the selection, and what edits it.
#[derive(Debug)]
pub struct Row {
    /// What the widget — or the tree — says this property is.
    pub property: &'static Property,
    /// Whether the tree owns it rather than the widget.
    pub node: bool,
    /// What edits it.
    pub editor: Editor,
    /// The text the editor was last *given*, so that what it now holds being
    /// different means a person changed it.
    pub shown: String,
}

/// What the pane needs in order to draw one row.
pub struct Field {
    /// The property.
    pub property: &'static Property,
    /// Whether the tree owns it.
    pub node: bool,
    /// The value to show, or `None` when several nodes are selected and they
    /// disagree — which is shown as an empty editor rather than as one of them.
    pub value: Option<String>,
    /// Whether the file writes it, or the node's own argument supplies it.
    /// A property at its default is dimmed.
    pub written: bool,
    /// Whether there is a property entry to take out of the file.
    ///
    /// Not the same as [`written`](Field::written): a `label "Heading"` writes
    /// its text as an argument, and taking that away would leave a node with no
    /// text rather than a node with the default one.
    pub resettable: bool,
    /// A line under the property's own documentation, when there is something
    /// worth adding — the message names this form already uses, say.
    pub hint: Option<String>,
}

/// The pane.
pub struct Inspector {
    /// The panel the rows are built in, replaced when the selection changes.
    pub content: NodeId,
    /// The rows, in the order the descriptor listed them.
    pub rows: Vec<Row>,
    /// The line that says why a value was refused.
    pub complaint: NodeId,
}

impl Inspector {
    /// Builds the pane inside `parent`.
    ///
    /// `header` is the two lines above the rows: what is selected, and what it
    /// is called.
    pub fn build(
        ui: &mut Ui<Message>,
        parent: NodeId,
        width: i32,
        header: &[(String, Role, Text)],
        fields: &[Field],
        scale: Scale,
    ) -> Self {
        let (row, gap) = (scale.n(ROW), scale.n(GAP));
        let (name_width, reset) = (scale.n(NAME), scale.n(RESET));
        // A header line is twenty logical pixels of pitch holding an
        // eighteen-pixel label.
        let (header_pitch, header_height) = (scale.n(20), scale.n(Text::Heading.line()));
        let label = scale.text(Text::Body);
        // Tall enough for the step it holds — see `Text::line`. A box sized for
        // the old eleven-pixel text clipped `Body`'s descenders.
        let label_height = scale.n(Text::Body.line());

        // Tall enough for everything, inside a viewport that scrolls: a form
        // node with twenty properties and fourteen the tree owns does not fit a
        // pane, and a row quietly cut off at the bottom is worse than a wheel.
        let height = header.len() as i32 * header_pitch
            + fields.len() as i32 * (row + gap)
            + row * 2
            + gap * 4;
        let content = ui
            .add(parent, Panel::default(), Rect::new(0, 0, width, height))
            .expect("the inspector's viewport is there");

        let mut y = gap;
        for (text, role, size) in header {
            ui.add(
                content,
                Label::new(text.clone())
                    .with_role(*role)
                    .with_size(scale.text(*size)),
                Rect::new(gap, y, width - gap * 2, header_height),
            );
            y += header_pitch;
        }
        y += gap;

        let inner = width - gap * 2;
        let editor_x = gap + name_width + gap;
        let editor_w = inner - name_width - gap - reset - gap;

        let mut rows = Vec::with_capacity(fields.len());
        for (index, field) in fields.iter().enumerate() {
            // Dimmed when the file does not write it: what is on screen is then
            // the widget's own default, and knowing which is which is the
            // difference between reading a form and guessing at one.
            //
            // `Neutral` rather than `Base300`: a dimmed name still has to be
            // readable, and `Base300` is a *background* role — text in it is
            // one step off the panel it sits on.
            let role = if field.written {
                Role::BaseContent
            } else {
                Role::Neutral
            };
            let name = ui.add(
                content,
                Label::new(field.property.name)
                    .with_role(role)
                    .with_size(label),
                Rect::new(gap, y + scale.n(4), name_width, label_height),
            );
            if let Some(id) = name {
                let mut doc = field.property.doc.to_string();
                if let Some(hint) = &field.hint {
                    doc.push('\n');
                    doc.push_str(hint);
                }
                ui.set_tooltip(id, doc);
            }

            let shown = field.value.clone().unwrap_or_default();
            let editor = build_editor(
                ui,
                content,
                index,
                field,
                &shown,
                Rect::new(editor_x, y, editor_w, row),
                scale,
            );

            // Something to reset only when there is something to reset: a
            // property the file does not write is already at its default.
            if field.resettable
                && let Some(id) = ui.add(
                    content,
                    Button::new("×", Message::Reset(index))
                        .with_role(Role::Neutral)
                        .with_size(label),
                    Rect::new(gap + inner - reset, y + scale.n(2), reset, row - scale.n(4)),
                )
            {
                ui.set_tooltip(id, "Back to the default, which takes it out of the file");
            }

            rows.push(Row {
                property: field.property,
                node: field.node,
                editor,
                shown,
            });
            y += row + gap;
        }

        let complaint = ui
            .add(
                content,
                Label::new("").with_role(Role::Error).with_size(label),
                Rect::new(gap, y + gap, inner, row * 2),
            )
            .expect("the content panel is there");

        Self {
            content,
            rows,
            complaint,
        }
    }

    /// What each editor now holds, for every row where that differs from what it
    /// was given.
    ///
    /// Updates what was given, so one change is reported once.
    pub fn changed(&mut self, ui: &Ui<Message>) -> Vec<(usize, String)> {
        let mut changed = Vec::new();
        for (index, row) in self.rows.iter_mut().enumerate() {
            let Some(now) = read(ui, &row.editor) else {
                continue;
            };
            if now != row.shown {
                row.shown.clone_from(&now);
                changed.push((index, now));
            }
        }
        changed
    }

    /// Puts a value into a row's editor without that counting as an edit.
    ///
    /// What keeps the four rectangle fields following a drag on the canvas.
    pub fn show(&mut self, ui: &mut Ui<Message>, index: usize, text: String) {
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        if row.shown == text {
            return;
        }
        match &row.editor {
            Editor::Field(id) | Editor::Slid { field: id, .. } => {
                if let Some(field) = ui.widget_mut::<TextInput<Message>>(*id) {
                    field.set_text(text.clone());
                }
                if let Editor::Slid { slider, .. } = &row.editor
                    && let Ok(number) = text.parse::<f32>()
                    && let Some(widget) = ui.widget_mut::<Slider<Message>>(*slider)
                {
                    widget.set_value(number);
                }
            }
            Editor::Flag(id) => {
                if let Some(box_) = ui.widget_mut::<Checkbox<Message>>(*id) {
                    box_.set_checked(text == "#true");
                }
            }
            Editor::Choice { select, options } => {
                let at = options.iter().position(|option| *option == text);
                if let Some(widget) = ui.widget_mut::<Select<Message>>(*select) {
                    widget.set_selected(at);
                }
            }
        }
        row.shown = text;
    }

    /// Says why a value was refused, or says nothing.
    pub fn complain(&mut self, ui: &mut Ui<Message>, text: &str) {
        if let Some(label) = ui.widget_mut::<Label>(self.complaint) {
            label.set_text(text);
        }
    }
}

/// The editor a property's kind calls for.
fn build_editor(
    ui: &mut Ui<Message>,
    parent: NodeId,
    index: usize,
    field: &Field,
    shown: &str,
    rect: Rect,
    scale: Scale,
) -> Editor {
    // `rect` arrives physical, so the constants below are the ones that move.
    let (x, y, width) = (rect.x, rect.y, rect.width);
    let (row, gap) = (scale.n(ROW), scale.n(GAP));
    match field.property.kind {
        PropertyKind::Bool => {
            let id = ui
                .add(
                    parent,
                    Checkbox::<Message>::inert("")
                        .with_checked(shown == "#true")
                        .with_size(scale.text(Text::Body)),
                    rect,
                )
                .expect("the content panel is there");
            Editor::Flag(id)
        }
        PropertyKind::Enum(options) => {
            let at = options.iter().position(|option| *option == shown);
            // Opening it is a message this row can name, because a `Select`
            // holds the message itself rather than a function of the choice.
            // *Choosing* is not: the popup's list reports an index through a
            // `fn(usize) -> M`, so the designer remembers which row it opened.
            let id = ui
                .add(
                    parent,
                    Select::new(options.iter().copied(), Message::OpenChoice(index))
                        .with_selected(at)
                        .with_placeholder("—")
                        .with_style(TextStyle::built_in(scale.text(Text::Body))),
                    rect,
                )
                .expect("the content panel is there");
            Editor::Choice {
                select: id,
                options,
            }
        }
        kind => {
            // No slider without a number for it to start at: a property at a
            // default the widget does not report, or one several selected nodes
            // disagree about, has nothing to point the knob at.
            let slidable = bounded(kind).filter(|_| shown.parse::<f32>().is_ok());
            // A path is found rather than typed, most of the time.
            let browse = matches!(kind, PropertyKind::Asset).then(|| {
                ui.add(
                    parent,
                    Button::new("…", Message::Browse(index))
                        .with_role(Role::Neutral)
                        .with_size(scale.text(Text::Body)),
                    Rect::new(x, y + scale.n(2), scale.n(BROWSE), row - scale.n(4)),
                )
            });
            let taken = if browse.is_some() {
                scale.n(BROWSE) + gap
            } else {
                0
            };
            let field_w = if slidable.is_some() {
                scale.n(56)
            } else {
                width - taken
            };
            let id = ui
                .add(
                    parent,
                    TextInput::<Message>::new()
                        .with_size(scale.text(Text::Body))
                        .with_max_chars(512),
                    Rect::new(x + width - field_w, y, field_w, row),
                )
                .expect("the content panel is there");
            if let Some(input) = ui.widget_mut::<TextInput<Message>>(id) {
                input.set_text(shown);
            }
            let Some((min, max)) = slidable else {
                return Editor::Field(id);
            };
            let value = shown.parse::<f32>().unwrap_or(min);
            let slider = Slider::<Message>::inert(min, max, value);
            // A whole number stays whole while it is dragged.
            let slider = if matches!(kind, PropertyKind::Int { .. }) {
                slider.with_step(1.0)
            } else {
                slider
            };
            let slider = ui
                .add(
                    parent,
                    slider,
                    Rect::new(x, y + scale.n(4), width - field_w - gap, row - scale.n(8)),
                )
                .expect("the content panel is there");
            Editor::Slid { field: id, slider }
        }
    }
}

/// What an editor now holds, in the same spelling [`Field::value`] uses.
fn read(ui: &Ui<Message>, editor: &Editor) -> Option<String> {
    Some(match editor {
        Editor::Field(id) => ui.widget::<TextInput<Message>>(*id)?.text().to_string(),
        Editor::Slid { field, slider } => {
            // The slider is the coarse control and the field is the exact one,
            // so whichever moved last wins — and a slider that has not moved
            // reads back what the field was set to anyway.
            //
            // An empty field is *no value*, which a slider cannot express: it
            // always sits somewhere. So an empty field wins outright, or
            // clearing one would write the low end of the range straight back
            // over the default it was clearing.
            let text = ui.widget::<TextInput<Message>>(*field)?.text().to_string();
            if text.trim().is_empty() {
                return Some(text);
            }
            let value = ui.widget::<Slider<Message>>(*slider)?.value();
            if text.parse::<f32>().is_ok_and(|held| held == value) {
                text
            } else {
                trim(value)
            }
        }
        Editor::Flag(id) => {
            let checked = ui.widget::<Checkbox<Message>>(*id)?.checked();
            String::from(if checked { "#true" } else { "#false" })
        }
        Editor::Choice { select, .. } => ui
            .widget::<Select<Message>>(*select)?
            .selected_option()
            .unwrap_or_default()
            .to_string(),
    })
}

/// A range narrow enough that a slider over it can be aimed.
pub fn bounded(kind: PropertyKind) -> Option<(f32, f32)> {
    let (min, max) = match kind {
        PropertyKind::Int { min, max } => (min as f32, max as f32),
        PropertyKind::Float { min, max } => (min, max),
        _ => return None,
    };
    (min.is_finite() && max.is_finite() && max > min && max - min <= SLIDABLE).then_some((min, max))
}

/// A number as a field should show it: `70` rather than `70.0000001`.
pub fn trim(value: f32) -> String {
    if value.fract() == 0.0 {
        // `-0` is a number nobody typed.
        return format!("{}", value as i64);
    }
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A property's value as a field shows it.
pub fn show_value(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Bool(flag) => String::from(if *flag { "#true" } else { "#false" }),
        Value::Int(number) => number.to_string(),
        Value::Float(number) => trim(*number),
        Value::Enum(name) => (*name).to_string(),
        // `Value` is `non_exhaustive`, and a kind this pane has not learned
        // should be a row that reports rather than a row that is missing.
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_range_gets_no_slider_and_a_narrow_one_does() {
        // `x` runs from -8192 to 8192: a hundred-pixel slider over that cannot
        // be aimed, so the field is the whole editor.
        assert_eq!(
            bounded(PropertyKind::Int {
                min: -8192,
                max: 8192
            }),
            None
        );
        assert_eq!(
            bounded(PropertyKind::Float { min: 0.0, max: 1.0 }),
            Some((0.0, 1.0))
        );
        assert_eq!(bounded(PropertyKind::Text), None);
        assert_eq!(
            bounded(PropertyKind::Float {
                min: f32::MIN,
                max: f32::MAX
            }),
            None,
            "an unbounded float is not a slider"
        );
    }

    #[test]
    fn a_number_is_shown_the_way_somebody_would_write_it() {
        assert_eq!(trim(70.0), "70");
        assert_eq!(trim(-0.0), "0");
        assert_eq!(trim(0.5), "0.5");
        assert_eq!(trim(0.3333333), "0.333");
    }
}
