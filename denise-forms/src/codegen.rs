//! A form file, as Rust the compiler checks.
//!
//! The engine hands back `built.node("who")` and resolves message names through
//! a `match` on a string. That is right for a kiosk, it is checked when the form
//! loads, and it is what [`Form::build`](crate::Form::build) does. What it is not
//! is what Delphi gave you, which was `Button1: TButton` as a field the compiler
//! knew about.
//!
//! This generates that. Point a `build.rs` at a form file and get a struct whose
//! fields are the form's named nodes and an enum whose variants are the form's
//! messages:
//!
//! ```no_run
//! // build.rs
//! fn main() {
//!     denise_forms::codegen::to_out_dir("forms/settings.dform").unwrap();
//! }
//! ```
//!
//! ```ignore
//! // src/main.rs
//! include!(concat!(env!("OUT_DIR"), "/settings.rs"));
//!
//! let form = Settings::build(&mut ui, root)?;
//! ui.widget_mut::<TextInput<SettingsMessage>>(form.who);   // a field, not a lookup
//! ```
//!
//! **Rename a node in the form and the application stops compiling**, naming the
//! field that no longer exists. **Add a message to the form and every `match` on
//! the enum stops compiling**, because it is no longer exhaustive. Both are the
//! point, and both are what a string lookup cannot do.
//!
//! # A build script rather than a proc macro
//!
//! Chosen deliberately, and [#101] said to. The output is a file you can open,
//! `cargo doc` sees it, a debugger steps through it, and it needs no second
//! crate. A macro would read a little better at the call site and cost all four.
//!
//! # It generates a caller, not a second engine
//!
//! The generated `build` calls [`Form::build`](crate::Form::build) with a
//! generated [`Wiring`](crate::Wiring). There is one implementation of building
//! a form, and this is a typed door onto it — so a form that loads at runtime and
//! the same form generated behave identically, because they are the same code.
//!
//! [#101]: https://github.com/bisand/denise/issues/101

// The examples here are build scripts, and a build script *is* its `fn main`.
// Compiling them is what checks these call signatures are real, so they stay
// doctests rather than becoming prose.
#![allow(clippy::needless_doctest_main)]

use std::collections::BTreeMap;

use denise_ui::widgets::{Payload, PropertyKind, all};

use crate::error::{At, Error, Reason};
use crate::form::Form;

/// Rust's keywords, which a form file is free to use as a name and Rust is not.
///
/// Escaped as `r#name` rather than refused, because `type` and `match` are
/// perfectly good names for a field and the raw form is exactly what it is for.
/// The three that cannot be raw are refused instead.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield", "gen",
];

/// The three keywords Rust will not accept even raw.
const NEVER_RAW: &[&str] = &["crate", "self", "Self"];

/// What one form generated: the text of a module, and what is in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    /// The Rust source. Write it somewhere and `include!` it.
    pub source: String,
    /// The struct's name, taken from the file's own `name=` or its file name.
    pub kind: String,
    /// The message enum's name: the struct's, with `Message` on the end.
    pub message: String,
}

/// Generates the struct and the enum for a form.
///
/// `name` is what the struct is called — the file's stem, usually. See
/// [`to_out_dir`], which does that part for you.
///
/// ```
/// # use denise_forms::codegen::generate;
/// let form = r#"form "F" version=1 width=200 height=100 {
///     text-input name=full-name x=0 y=0 w=100 h=30 on-submit=save
///     checkbox "Notify" name=notify x=0 y=40 w=100 h=20 on-change=set-notify
/// }"#;
///
/// let generated = generate(form, "settings")?;
/// assert_eq!(generated.kind, "Settings");
/// assert_eq!(generated.message, "SettingsMessage");
///
/// // A kebab name becomes a snake field and a Pascal variant, and the payload
/// // the widget needs becomes what the variant carries.
/// assert!(generated.source.contains("pub full_name: ::denise_ui::NodeId"));
/// assert!(generated.source.contains("Save,"));
/// assert!(generated.source.contains("SetNotify(bool)"));
/// # Ok::<(), denise_forms::Error>(())
/// ```
///
/// # Errors
///
/// Everything [`Form::parse`](crate::Form::parse) can say, plus the three things
/// only generating code can hit: a name that is not a Rust identifier, two names
/// that become one identifier, and one message name used with two payload shapes.
/// Each carries the position in the file.
pub fn generate(source: &str, name: &str) -> Result<Generated, Error> {
    let form = Form::parse(source)?;
    let kind = type_name(name).ok_or_else(|| {
        Error::new(
            At::START,
            Reason::NotAnIdentifier {
                found: name.to_string(),
                because: "a form's name has to start with a letter",
            },
        )
    })?;
    let message = format!("{kind}Message");

    // Named nodes become fields; message names become variants. Both are
    // gathered in file order, so the generated file reads down the form.
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    let mut messages: Vec<(String, String, Payload)> = Vec::new();
    let mut seen: BTreeMap<String, (Payload, String)> = BTreeMap::new();

    for node in form.written() {
        if node.path.is_empty() {
            continue;
        }
        if let Some(name) = &node.name {
            let field = field_name(name)?;
            if let Some(first) = taken.get(&field) {
                return Err(Error::new(
                    At::START,
                    Reason::Collides {
                        found: name.clone(),
                        with: first.clone(),
                        spelled: field,
                    },
                ));
            }
            taken.insert(field.clone(), name.clone());
            fields.push((field, name.clone()));
        }

        let Some(info) = all().iter().find(|widget| widget.kind == node.kind) else {
            continue;
        };
        for property in info.properties {
            let PropertyKind::Message(payload) = property.kind else {
                continue;
            };
            let Some(used) = form.property(&node.path, property.name) else {
                continue;
            };
            match seen.get(&used) {
                Some((first, _)) if *first != payload => {
                    return Err(Error::new(
                        At::START,
                        Reason::PayloadClash {
                            found: used,
                            first: shape(*first),
                            then: shape(payload),
                        },
                    ));
                }
                Some(_) => continue,
                None => {}
            }
            let variant = variant_name(&used)?;
            seen.insert(used.clone(), (payload, variant.clone()));
            messages.push((variant, used, payload));
        }
    }

    Ok(Generated {
        source: write(source, &kind, &message, &fields, &messages),
        kind,
        message,
    })
}

/// Generates a form into `OUT_DIR` and tells Cargo to watch it.
///
/// The whole of a `build.rs`:
///
/// ```no_run
/// fn main() {
///     denise_forms::codegen::to_out_dir("forms/settings.dform").unwrap();
/// }
/// ```
///
/// The module lands at `$OUT_DIR/<stem>.rs` and the struct is named after the
/// stem: `settings.dform` gives `Settings` and `SettingsMessage`.
///
/// # Errors
///
/// Anything [`generate`] can say, and anything reading or writing a file can.
pub fn to_out_dir(path: impl AsRef<std::path::Path>) -> Result<std::path::PathBuf, String> {
    let path = path.as_ref();
    // Before anything can fail, so a form that stops generating still rebuilds
    // when it is fixed rather than staying broken until a clean.
    println!("cargo:rerun-if-changed={}", path.display());

    let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
        format!(
            "{}: no file name to take a struct name from",
            path.display()
        )
    })?;
    let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let generated = generate(&source, stem).map_err(|e| format!("{}:{e}", path.display()))?;

    let out = std::path::PathBuf::from(
        std::env::var("OUT_DIR")
            .map_err(|_| String::from("OUT_DIR is not set; this is for a build script"))?,
    )
    .join(format!("{stem}.rs"));
    std::fs::write(&out, &generated.source).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out)
}

/// What a payload is called in a message.
const fn shape(payload: Payload) -> &'static str {
    match payload {
        Payload::None => "the message itself",
        Payload::Bool => "a `fn(bool)`",
        Payload::Index => "a `fn(usize)`",
        Payload::Number => "a `fn(f32)`",
    }
}

/// The Rust type of a payload, as it appears in a variant.
const fn carried(payload: Payload) -> &'static str {
    match payload {
        Payload::None => "",
        Payload::Bool => "(bool)",
        Payload::Index => "(usize)",
        Payload::Number => "(f32)",
    }
}

/// The whole module, as text.
fn write(
    source: &str,
    kind: &str,
    message: &str,
    fields: &[(String, String)],
    messages: &[(String, String, Payload)],
) -> String {
    let mut out = String::new();
    out.push_str(
        "// Generated from a `.dform` file by `denise_forms::codegen`. Do not edit:\n\
         // the form file is the source, and this is rewritten on every build.\n\n",
    );

    // The form's own text, so the generated module needs no path at run time.
    out.push_str(&format!(
        "/// The form this was generated from, as it stood at build time.\n\
         pub const {}_SOURCE: &str = r####\"{source}\"####;\n\n",
        upper(kind),
    ));

    // The struct.
    out.push_str(&format!(
        "/// Every node [`{kind}`] names, as a field.\n\
         ///\n\
         /// Rename one in the form and this stops compiling where it was used,\n\
         /// which is the whole reason this file is generated.\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub struct {kind} {{\n"
    ));
    for (field, name) in fields {
        out.push_str(&format!(
            "    /// The node the form calls `{name}`.\n    pub {field}: ::denise_ui::NodeId,\n"
        ));
    }
    if fields.is_empty() {
        out.push_str("    /// The form names no nodes.\n    _private: (),\n");
    }
    out.push_str("}\n\n");

    // The message enum.
    out.push_str(&format!(
        "/// Every message [`{kind}`] can emit.\n\
         ///\n\
         /// Add one to the form and every `match` on this stops compiling,\n\
         /// because it is no longer exhaustive.\n\
         #[derive(Clone, Copy, PartialEq, Debug)]\n\
         pub enum {message} {{\n"
    ));
    for (variant, name, payload) in messages {
        out.push_str(&format!(
            "    /// What the form calls `{name}`.\n    {variant}{},\n",
            carried(*payload)
        ));
    }
    if messages.is_empty() {
        out.push_str("    /// The form emits nothing, and this cannot be constructed.\n    #[doc(hidden)]\n    Never,\n");
    }
    out.push_str("}\n\n");

    // The wiring, and the constructor.
    out.push_str(&format!(
        "impl {kind} {{\n\
         \x20   /// Builds the form under `parent`, with no pictures.\n\
         \x20   ///\n\
         \x20   /// # Errors\n\
         \x20   ///\n\
         \x20   /// Whatever [`denise_forms::Form::build`] says, with a line and a column.\n\
         \x20   pub fn build(\n\
         \x20       ui: &mut ::denise_ui::Ui<{message}>,\n\
         \x20       parent: ::denise_ui::NodeId,\n\
         \x20   ) -> ::core::result::Result<Self, ::denise_forms::Error> {{\n\
         \x20       Self::build_with(ui, parent, &mut |_: &str| None)\n\
         \x20   }}\n\n\
         \x20   /// Builds the form, loading pictures through `assets`.\n\
         \x20   ///\n\
         \x20   /// # Errors\n\
         \x20   ///\n\
         \x20   /// Whatever [`denise_forms::Form::build`] says, with a line and a column.\n\
         \x20   pub fn build_with(\n\
         \x20       ui: &mut ::denise_ui::Ui<{message}>,\n\
         \x20       parent: ::denise_ui::NodeId,\n\
         \x20       assets: &mut dyn FnMut(&str) -> Option<::denise_forms::Picture>,\n\
         \x20   ) -> ::core::result::Result<Self, ::denise_forms::Error> {{\n\
         \x20       let form = ::denise_forms::Form::parse({0}_SOURCE)?;\n\
         \x20       let fit = ::denise_forms::Placement {{\n\
         \x20           x: 1.0,\n\
         \x20           y: 1.0,\n\
         \x20           rect: ::denise::Rect::from_size(form.size()),\n\
         \x20       }};\n\
         \x20       Self::place(ui, parent, fit, assets)\n\
         \x20   }}\n\n\
         \x20   /// Builds the form at a [`Placement`](denise_forms::Placement), which is\n\
         \x20   /// what the file's own `scaling=` works out to.\n\
         \x20   ///\n\
         \x20   /// The typed door onto [`Form::build_fitted`](denise_forms::Form::build_fitted),\n\
         \x20   /// so a generated form scales exactly as a loaded one does.\n\
         \x20   ///\n\
         \x20   /// # Errors\n\
         \x20   ///\n\
         \x20   /// Whatever [`denise_forms::Form::build_fitted`] says, with a line and a column.\n\
         \x20   pub fn place(\n\
         \x20       ui: &mut ::denise_ui::Ui<{message}>,\n\
         \x20       parent: ::denise_ui::NodeId,\n\
         \x20       fit: ::denise_forms::Placement,\n\
         \x20       assets: &mut dyn FnMut(&str) -> Option<::denise_forms::Picture>,\n\
         \x20   ) -> ::core::result::Result<Self, ::denise_forms::Error> {{\n\
         \x20       let form = ::denise_forms::Form::parse({0}_SOURCE)?;\n\
         \x20       let built = form.build_fitted(ui, parent, fit, &mut {kind}Wiring {{ assets }})?;\n\
         \x20       Ok(Self {{\n",
        upper(kind),
    ));
    for (field, name) in fields {
        out.push_str(&format!(
            "            {field}: built.node({name:?}).expect(\"the form names it, and this file was generated from that form\"),\n"
        ));
    }
    if fields.is_empty() {
        out.push_str("            _private: (),\n");
    }
    out.push_str("        })\n    }\n\n");

    // The form's own facts, so a caller needs no second copy of them.
    out.push_str(&format!(
        "    /// What the form was designed at, and what it is called.\n\
         \x20   ///\n\
         \x20   /// # Panics\n\
         \x20   ///\n\
         \x20   /// Never: the source was parsed at build time to generate this.\n\
         \x20   pub fn form() -> ::denise_forms::Form {{\n\
         \x20       ::denise_forms::Form::parse({}_SOURCE).expect(\"generated from this very text\")\n\
         \x20   }}\n}}\n\n",
        upper(kind),
    ));

    // The generated resolver: one arm per name, with the shape the widget needs.
    out.push_str(&format!(
        "/// Turns the form's message names into [`{message}`].\n\
         ///\n\
         /// One arm per name, generated — so a name the form uses and this does\n\
         /// not answer is impossible rather than an error at load.\n\
         struct {kind}Wiring<'a> {{\n\
         \x20   assets: &'a mut dyn FnMut(&str) -> Option<::denise_forms::Picture>,\n\
         }}\n\n\
         impl ::denise_forms::Wiring<{message}> for {kind}Wiring<'_> {{\n\
         \x20   fn message(\n\
         \x20       &mut self,\n\
         \x20       name: &str,\n\
         \x20       payload: ::denise_forms::Payload,\n\
         \x20   ) -> Option<::denise_forms::Handler<{message}>> {{\n\
         \x20       Some(match (name, payload) {{\n"
    ));
    for (variant, name, payload) in messages {
        let arm = match payload {
            Payload::None => format!("::denise_forms::Handler::Plain({message}::{variant})"),
            Payload::Bool => format!("::denise_forms::Handler::Bool({message}::{variant})"),
            Payload::Index => format!("::denise_forms::Handler::Index({message}::{variant})"),
            Payload::Number => format!("::denise_forms::Handler::Number({message}::{variant})"),
        };
        out.push_str(&format!(
            "            ({name:?}, ::denise_forms::Payload::{:?}) => {arm},\n",
            payload
        ));
    }
    out.push_str(
        "            _ => return None,\n        })\n    }\n\n\
         \x20   fn asset(&mut self, path: &str) -> Option<::denise_forms::Picture> {\n\
         \x20       (self.assets)(path)\n    }\n}\n",
    );
    out
}

/// `settings` becomes `SETTINGS`, for the source constant.
fn upper(kind: &str) -> String {
    let mut out = String::new();
    for (index, character) in kind.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            out.push('_');
        }
        out.extend(character.to_uppercase());
    }
    out
}

/// `settings-screen` becomes `SettingsScreen`.
fn type_name(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut upper = true;
    for character in name.chars() {
        if character == '-' || character == '_' || character == ' ' {
            upper = true;
            continue;
        }
        if !character.is_ascii_alphanumeric() {
            return None;
        }
        if upper {
            out.extend(character.to_uppercase());
            upper = false;
        } else {
            out.push(character);
        }
    }
    (!out.is_empty() && out.starts_with(|c: char| c.is_ascii_alphabetic())).then_some(out)
}

/// `set-notify` becomes `SetNotify`.
fn variant_name(name: &str) -> Result<String, Error> {
    type_name(name).ok_or_else(|| {
        Error::new(
            At::START,
            Reason::NotAnIdentifier {
                found: name.to_string(),
                because: "a message name becomes an enum variant, so it must be letters, \
                          digits and dashes, starting with a letter",
            },
        )
    })
}

/// `full-name` becomes `full_name`, and `type` becomes `r#type`.
fn field_name(name: &str) -> Result<String, Error> {
    let refuse = |because| {
        Err(Error::new(
            At::START,
            Reason::NotAnIdentifier {
                found: name.to_string(),
                because,
            },
        ))
    };
    if name.is_empty() {
        return refuse("a name has to be something");
    }
    if NEVER_RAW.contains(&name) {
        return refuse("Rust will not accept this word as an identifier, even raw");
    }
    let mut out = String::new();
    for character in name.chars() {
        match character {
            '-' | ' ' => out.push('_'),
            c if c.is_ascii_alphanumeric() || c == '_' => out.push(c),
            _ => return refuse("a field name is letters, digits, dashes and underscores"),
        }
    }
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return refuse("a field name has to start with a letter or an underscore");
    }
    if KEYWORDS.contains(&out.as_str()) {
        out.insert_str(0, "r#");
    }
    Ok(out)
}
