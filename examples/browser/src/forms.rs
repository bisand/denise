//! Forms: the half of HTML that was always a widget toolkit's business.
//!
//! One walk of the tree produces two views of the same controls. The
//! *fields* are what a submission reads — names, kinds, and for radios the
//! whole group folded into one field, which is also how the Denise
//! `RadioGroup` thinks of it: one choice, one tab stop. The *render* map is
//! what layout draws — a widget per control, sized before the pixels exist.
//!
//! Submission is [`url::form_urlencoded`] doing what it was named for.
//! Values come from the live widgets at the moment of the click, the way
//! `Ui::widget` was always the read path; nothing here caches state the
//! tree already holds.

use std::collections::HashMap;

use crate::dom::{Dom, NodeData};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    Get,
    Post,
}

pub struct FormsModel {
    pub forms: Vec<Form>,
    /// DOM index to what layout should put there. A radio group renders at
    /// the group's first radio; its siblings render nothing at all.
    pub render: HashMap<usize, RenderControl>,
}

pub struct Form {
    /// Unresolved, exactly as written; empty or missing means "here".
    pub action: Option<String>,
    pub method: Method,
    pub fields: Vec<Field>,
}

pub struct Field {
    /// The element a widget is bound to — for a radio group, the first of
    /// the group.
    pub dom: usize,
    pub name: Option<String>,
    pub kind: FieldKind,
}

pub enum FieldKind {
    /// A `TextInput`: text, search, email, password, a single-line
    /// textarea, and every type this browser does not recognise — HTML's
    /// own rule for unknown input types.
    Text,
    Checkbox {
        value: String,
    },
    /// One field for the whole name-group; `values` parallel the rendered
    /// options.
    Radio {
        values: Vec<String>,
    },
    Select {
        values: Vec<String>,
    },
    Hidden {
        value: String,
    },
}

pub enum RenderControl {
    Text {
        value: String,
        placeholder: String,
        password: bool,
        /// The `size` attribute: a width in characters.
        chars: Option<i32>,
        form: usize,
    },
    Checkbox {
        checked: bool,
    },
    Radio {
        labels: Vec<String>,
        selected: usize,
    },
    Select {
        labels: Vec<String>,
        selected: Option<usize>,
    },
    Button {
        label: String,
        form: usize,
    },
}

pub fn extract(dom: &Dom) -> FormsModel {
    let mut model = FormsModel {
        forms: Vec::new(),
        render: HashMap::new(),
    };
    walk(dom, 0, None, &mut model, &mut HashMap::new());
    model
}

/// `radios` remembers, per (form, name), which field the group folded into.
fn walk(
    dom: &Dom,
    idx: usize,
    form: Option<usize>,
    model: &mut FormsModel,
    radios: &mut HashMap<(usize, String), usize>,
) {
    let mut child_form = form;
    if let NodeData::Element { name, .. } = &dom.nodes[idx].data {
        match name.local.as_ref() {
            "form" => {
                model.forms.push(Form {
                    action: dom.attr(idx, "action").map(str::to_string),
                    method: match dom.attr(idx, "method") {
                        Some(m) if m.eq_ignore_ascii_case("post") => Method::Post,
                        _ => Method::Get,
                    },
                    fields: Vec::new(),
                });
                child_form = Some(model.forms.len() - 1);
            }
            "input" if form.is_some() => input(dom, idx, form.expect("checked"), model, radios),
            "select" if form.is_some() => select(dom, idx, form.expect("checked"), model),
            "textarea" if form.is_some() => {
                let f = form.expect("checked");
                push_field(model, f, idx, name_attr(dom, idx), FieldKind::Text);
                model.render.insert(
                    idx,
                    RenderControl::Text {
                        value: collapse(&dom.text_content(idx)),
                        placeholder: String::new(),
                        password: false,
                        chars: None,
                        form: f,
                    },
                );
            }
            "button" if form.is_some() => {
                // Only a submitting button is worth pixels: without scripts
                // a `type=button` does nothing at all.
                let kind = dom.attr(idx, "type").unwrap_or("submit");
                if kind.eq_ignore_ascii_case("submit") {
                    let label = collapse(&dom.text_content(idx));
                    model.render.insert(
                        idx,
                        RenderControl::Button {
                            label: if label.is_empty() {
                                "Submit".into()
                            } else {
                                label
                            },
                            form: form.expect("checked"),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    let children = dom.nodes[idx].children.clone();
    for child in children {
        walk(dom, child, child_form, model, radios);
    }
}

fn input(
    dom: &Dom,
    idx: usize,
    form: usize,
    model: &mut FormsModel,
    radios: &mut HashMap<(usize, String), usize>,
) {
    let kind = dom.attr(idx, "type").unwrap_or("text").to_ascii_lowercase();
    let name = name_attr(dom, idx);
    let value = dom.attr(idx, "value").unwrap_or_default().to_string();
    match kind.as_str() {
        "hidden" => {
            push_field(model, form, idx, name, FieldKind::Hidden { value });
        }
        "checkbox" => {
            push_field(
                model,
                form,
                idx,
                name,
                FieldKind::Checkbox {
                    value: if value.is_empty() { "on".into() } else { value },
                },
            );
            model.render.insert(
                idx,
                RenderControl::Checkbox {
                    checked: dom.attr(idx, "checked").is_some(),
                },
            );
        }
        "radio" => {
            let Some(name) = name else {
                // A nameless radio can neither group nor submit.
                return;
            };
            let label = if value.is_empty() { "on".into() } else { value };
            let checked = dom.attr(idx, "checked").is_some();
            let key = (form, name.clone());
            let field_at = *radios.entry(key).or_insert_with(|| {
                push_field(
                    model,
                    form,
                    idx,
                    Some(name),
                    FieldKind::Radio { values: Vec::new() },
                );
                model.render.insert(
                    idx,
                    RenderControl::Radio {
                        labels: Vec::new(),
                        selected: 0,
                    },
                );
                model.forms[form].fields.len() - 1
            });
            let field = &mut model.forms[form].fields[field_at];
            let FieldKind::Radio { values } = &mut field.kind else {
                return;
            };
            values.push(label.clone());
            let position = values.len() - 1;
            if let Some(RenderControl::Radio { labels, selected }) =
                model.render.get_mut(&field.dom)
            {
                labels.push(label);
                if checked {
                    *selected = position;
                }
            }
        }
        "submit" | "image" => {
            model.render.insert(
                idx,
                RenderControl::Button {
                    label: if value.is_empty() {
                        "Submit".into()
                    } else {
                        value
                    },
                    form,
                },
            );
        }
        // `file`, `reset`, and friends: not this browser's business.
        "file" | "reset" | "button" => {}
        other => {
            push_field(model, form, idx, name, FieldKind::Text);
            model.render.insert(
                idx,
                RenderControl::Text {
                    value,
                    placeholder: dom.attr(idx, "placeholder").unwrap_or_default().into(),
                    password: other == "password",
                    chars: dom
                        .attr(idx, "size")
                        .and_then(|s| s.trim().parse().ok())
                        .filter(|c| *c > 0),
                    form,
                },
            );
        }
    }
}

fn select(dom: &Dom, idx: usize, form: usize, model: &mut FormsModel) {
    let mut values = Vec::new();
    let mut labels = Vec::new();
    let mut selected = None;
    collect_options(dom, idx, &mut values, &mut labels, &mut selected);
    push_field(
        model,
        form,
        idx,
        name_attr(dom, idx),
        FieldKind::Select { values },
    );
    model
        .render
        .insert(idx, RenderControl::Select { labels, selected });
}

fn collect_options(
    dom: &Dom,
    idx: usize,
    values: &mut Vec<String>,
    labels: &mut Vec<String>,
    selected: &mut Option<usize>,
) {
    for &child in &dom.nodes[idx].children {
        match dom.tag(child) {
            Some("option") => {
                let label = collapse(&dom.text_content(child));
                let value = dom
                    .attr(child, "value")
                    .map(str::to_string)
                    .unwrap_or_else(|| label.clone());
                if dom.attr(child, "selected").is_some() {
                    *selected = Some(values.len());
                }
                values.push(value);
                labels.push(label);
            }
            // `<optgroup>` flattens; its label is dropped.
            Some("optgroup") => collect_options(dom, child, values, labels, selected),
            _ => {}
        }
    }
}

fn push_field(
    model: &mut FormsModel,
    form: usize,
    dom: usize,
    name: Option<String>,
    kind: FieldKind,
) {
    model.forms[form].fields.push(Field { dom, name, kind });
}

fn name_attr(dom: &Dom, idx: usize) -> Option<String> {
    dom.attr(idx, "name")
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

/// Whitespace collapsed and trimmed: what an attribute-less label means.
fn collapse(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// The body a POST sends and the query a GET carries: one serializer, the
/// one the `url` crate ships for exactly this.
pub fn urlencoded(pairs: &[(String, String)]) -> String {
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        out.append_pair(name, value);
    }
    out.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(html: &str) -> (Dom, FormsModel) {
        let dom = Dom::parse(html);
        let model = extract(&dom);
        (dom, model)
    }

    #[test]
    fn every_control_lands_in_its_form() {
        let (_, m) = model(
            r#"<form action="/go" method="post">
                <input name="q" value="x" size="12">
                <input type="checkbox" name="c" checked>
                <input type="radio" name="r" value="a">
                <input type="radio" name="r" value="b" checked>
                <select name="s"><option value="1">One</option><option selected>Two</option></select>
                <textarea name="t">seed  text</textarea>
                <input type="hidden" name="h" value="v">
                <input type="submit" value="Go">
            </form>"#,
        );
        assert_eq!(m.forms.len(), 1);
        let form = &m.forms[0];
        assert_eq!(form.method, Method::Post);
        assert_eq!(form.action.as_deref(), Some("/go"));
        // q, c, r (one field for two radios), s, t, h.
        assert_eq!(form.fields.len(), 6);
        let radio = form
            .fields
            .iter()
            .find(|f| matches!(f.kind, FieldKind::Radio { .. }))
            .expect("a radio field");
        let FieldKind::Radio { values } = &radio.kind else {
            unreachable!()
        };
        assert_eq!(values, &["a", "b"]);
        let RenderControl::Radio { selected, .. } = &m.render[&radio.dom] else {
            panic!("radio renders as a group")
        };
        assert_eq!(*selected, 1, "the checked one");
    }

    #[test]
    fn options_take_text_when_value_is_absent() {
        let (_, m) = model("<form><select name=s><option>Plain</option></select></form>");
        let FieldKind::Select { values } = &m.forms[0].fields[0].kind else {
            panic!()
        };
        assert_eq!(values, &["Plain"]);
    }

    #[test]
    fn controls_outside_any_form_are_not_fields() {
        let (_, m) = model("<input name=lonely><form><input name=kept></form>");
        assert_eq!(m.forms.len(), 1);
        assert_eq!(m.forms[0].fields.len(), 1);
        assert_eq!(m.render.len(), 1);
    }

    #[test]
    fn serialization_speaks_form_urlencoded() {
        let pairs = vec![
            ("q".to_string(), "two words & more".to_string()),
            ("nævn".to_string(), "ø".to_string()),
        ];
        assert_eq!(urlencoded(&pairs), "q=two+words+%26+more&n%C3%A6vn=%C3%B8");
    }

    #[test]
    fn a_get_form_defaults_that_way() {
        let (_, m) = model("<form><input name=q></form>");
        assert_eq!(m.forms[0].method, Method::Get);
        assert!(m.forms[0].action.is_none());
    }
}
