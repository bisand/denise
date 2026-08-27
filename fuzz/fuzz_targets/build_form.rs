//! A valid-ish form, mutated, through `Form::build` on a headless `Ui`.
//!
//! `parse_form` throws bytes at the front door; almost none of them get past
//! it, so the *builder* — the arm per widget kind, the property conversions,
//! the wiring — barely runs. This target starts from things shaped like forms
//! and mutates from there, which is what actually exercises those arms.
//!
//! The structure is generated rather than the text: a tree of plausible nodes,
//! each with a kind from the real list, properties that are sometimes sensible
//! and sometimes nonsense, and children where the fuzzer felt like it. The
//! text those produce is then what `Form::parse` sees, so everything reachable
//! here is reachable from a file.
//!
//! What must hold, whatever was built:
//!
//! - no panic, in the parser, the builder, or the widgets' own `set`s;
//! - every `NodeId` in `Built` exists in the `Ui` — a built form that hands
//!   the application a dangling id is a crash deferred to first use;
//! - the items every collection reports are addressable — `items` and
//!   `item_path` agree, since the designer stands on both.

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use denise_forms::{Form, Handler, Payload, Picture, Wiring};
use denise_ui::{Ui, Void};
use libfuzzer_sys::fuzz_target;

/// Every kind the builder has an arm for, plus a few it does not.
const KINDS: &[&str] = &[
    "panel", "label", "badge", "divider", "alert", "button", "checkbox", "toggle", "slider",
    "rating", "radio-group", "tabs", "select", "text-input", "list", "tree", "table", "timeline",
    "progress", "spinner", "avatar", "image", "carousel", "video", "keypad", "collapse", "sprite",
    "nonsense", "form", "option",
];

/// Property names worth colliding: the tree's own, several widgets', and junk.
const NAMES: &[&str] = &[
    "x", "y", "w", "h", "name", "visible", "enabled", "z", "tooltip", "scroll", "stack", "focus",
    "anchor", "dock", "role", "size", "text", "checked", "min", "max", "value", "selected",
    "placeholder", "row-height", "indent", "depth", "open", "expanded-height", "orientation",
    "thickness", "src", "fit", "on-change", "on-select", "on-toggle", "on-submit", "on-activate",
    "leading", "trailing", "time", "pending", "width", "flex", "align", "junk",
];

/// Values in every spelling the format has, and a few it refuses.
const VALUES: &[&str] = &[
    "0", "1", "-1", "9999", "-9999", "2147483647", "-2147483648", "0.5", "#true", "#false",
    "\"text\"", "\"\"", "primary", "base-100", "nonsense", "changed", "\"a b c\"", "3.14",
    "#null", "0x10",
];

#[derive(Arbitrary, Debug)]
struct Node {
    kind: u8,
    argument: Option<u8>,
    properties: Vec<(u8, u8)>,
    children: Vec<Node>,
}

impl Node {
    fn write(&self, out: &mut String, depth: usize) {
        // The generator respects the depth limit; the *parser's* enforcement of
        // it is `parse_form`'s job, and a generated tree that always trips it
        // would spend the whole run testing one error path.
        if depth > 8 {
            return;
        }
        let indent = "    ".repeat(depth);
        out.push_str(&indent);
        out.push_str(KINDS[self.kind as usize % KINDS.len()]);
        if let Some(argument) = self.argument {
            out.push_str(" \"");
            out.push_str(NAMES[argument as usize % NAMES.len()]);
            out.push('"');
        }
        for (name, value) in &self.properties {
            out.push(' ');
            out.push_str(NAMES[*name as usize % NAMES.len()]);
            out.push('=');
            out.push_str(VALUES[*value as usize % VALUES.len()]);
        }
        if self.children.is_empty() {
            out.push('\n');
            return;
        }
        out.push_str(" {\n");
        for child in &self.children {
            child.write(out, depth + 1);
        }
        out.push_str(&indent);
        out.push_str("}\n");
    }
}

/// Wiring that answers everything, so refusal never hides a builder arm.
struct Anything;

impl Wiring<Void> for Anything {
    fn message(&mut self, _name: &str, payload: Payload) -> Option<Handler<Void>> {
        Some(match payload {
            Payload::None => Handler::Plain(Void),
            Payload::Bool => Handler::Bool(|_| Void),
            Payload::Index => Handler::Index(|_| Void),
            Payload::Number => Handler::Number(|_| Void),
        })
    }

    fn asset(&mut self, _path: &str) -> Option<Picture> {
        Some(Picture {
            pixels: vec![0xFF00_0000; 4],
            size: denise::Size::new(2, 2),
        })
    }
}

fuzz_target!(|data: &[u8]| {
    let mut unstructured = Unstructured::new(data);
    let Ok(nodes) = Vec::<Node>::arbitrary(&mut unstructured) else {
        return;
    };

    let mut source = String::from("form \"Fuzz\" version=1 width=320 height=240 {\n");
    for node in nodes.iter().take(64) {
        node.write(&mut source, 1);
    }
    source.push_str("}\n");

    let Ok(form) = Form::parse(&source) else {
        // Refused input is `parse_form`'s beat; this target is for what builds.
        return;
    };

    let mut ui: Ui<Void> = Ui::new(form.size(), form.theme());
    let root = ui.root();
    let Ok(built) = form.build(&mut ui, root, &mut Anything) else {
        return;
    };

    // A built form must never hand the application a dangling id.
    for (name, id) in built.names() {
        assert!(ui.contains(id), "`{name}` is not in the tree it was built into");
    }
    for placed in built.placed() {
        assert!(ui.contains(placed.id), "a placed node is not in the tree");
        // And the designer's addressing must agree with the file's.
        for kind in ["option", "tab", "item", "column", "row", "event", "picture"] {
            let items = form.items(&placed.path, kind);
            for nth in 0..items.len() {
                assert!(
                    form.item_path(&placed.path, kind, nth).is_some(),
                    "item {nth} of `{kind}` is reported but not addressable",
                );
            }
        }
    }
});
