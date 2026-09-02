//! The code behind a form: where it lives, and the way to an event's handler.
//!
//! Delphi paired `Unit1.pas` with `Unit1.dfm` by name, and a form had exactly
//! one unit. A `.dform` is not like that: it holds no code and is not owed to
//! any one application — `hello.dform` is built by three of the examples in
//! this repository — so the designer cannot guess where an event's handler is
//! and the form file is the wrong place to say. So it is said **beside** the
//! form, in a sidecar the designer writes the first time it is asked:
//!
//! ```text
//! forms/hello.dform
//! forms/hello.designer      code = ../examples/typed/src/main.rs
//!                           handlers = Typed
//! ```
//!
//! Versioned with the form, so the next person's designer knows too, and never
//! a byte of the form itself. The second line is optional and hand-written: it
//! names the type whose `impl` block the handlers belong in, which is what
//! turns a placeholder from a free function at the end of the file into a
//! method with `&mut self` where the others are.
//!
//! # Finding the handler
//!
//! There is no single convention for how a message name reaches Rust — the
//! typed path has a `match` on an enum and, usually, a method; the untyped
//! path has a `match` on a string — so [`locate`] walks a short ladder and the
//! first rung that answers wins: `fn greet(`, then `::Greet =>`, then `"greet"`.
//! With a handlers type named, a `fn greet(` inside *its* impl beats one
//! anywhere else. An event nothing answers gets a [`placeholder`], shaped by
//! the [`Payload`] the widget declares so it compiles as written. Wiring it
//! into the `match` stays the application's — and on the typed path the
//! compiler asks for exactly that, at exactly that line, the moment the form
//! gains the event.
//!
//! # Reading the vocabulary
//!
//! The other direction matters when the application is already compiled and
//! the form is being redesigned: a name the application does not answer is a
//! load error, and the sooner it is seen the better. [`answered`] harvests the
//! names the code-behind answers — string arms, `…Message::Variant` arms, the
//! handlers type's methods — so the inspector can say which they are and mark
//! an event that names none of them.
//!
//! # The editor
//!
//! [`launch`] runs a command template from the settings —
//! `code --goto {file}:{line}:{column}` by default, which is Visual Studio Code
//! opening and focusing without any extension — so another editor is a line in
//! the settings file rather than a change here. When that command cannot be
//! started the platform's own opener gets the file instead, which opens it and
//! nothing more.

use std::path::{Path, PathBuf};
use std::process::Command;

use denise_render::icon::{Icon, Shape};
use denise_ui::widgets::Payload;

/// The default editor command: Visual Studio Code, at the line.
pub const DEFAULT_EDITOR: &str = "code --goto {file}:{line}:{column}";

/// How long a second press on an event's name may wait and still be the
/// second half of a double-click, in milliseconds.
pub const DOUBLE_CLICK_MS: u64 = 400;

/// The button that opens the handler: an arrow leaving the corner of a box,
/// in the same format the palette's glyphs are drawn in.
pub static OPEN: Icon = Icon::new(&[
    // The box, open at the top right where the arrow leaves.
    Shape::fore(&[
        (12, 30),
        (44, 30),
        (44, 40),
        (22, 40),
        (22, 78),
        (60, 78),
        (60, 56),
        (70, 56),
        (70, 88),
        (12, 88),
    ]),
    // The arrow's shaft, corner to corner.
    Shape::fore(&[(46, 62), (76, 32), (84, 40), (54, 70)]),
    // Its head.
    Shape::fore(&[(58, 16), (88, 16), (88, 46), (78, 46), (78, 26), (58, 26)]),
]);

// ------------------------------------------------------------------ the link

/// What the sidecar says about a form's code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The file the handlers are in, resolved against the form's directory.
    pub code: PathBuf,
    /// The type whose `impl` the handlers belong in, when the sidecar says.
    pub handlers: Option<String>,
}

/// Where the link to a form's code is kept: beside the form, as `.designer`.
pub fn sidecar(form: &Path) -> PathBuf {
    form.with_extension("designer")
}

/// The `key = value` lines of a sidecar, as they are.
fn lines_of(form: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(sidecar(form))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

/// The code file a form's sidecar names, and the handlers type if it names
/// one.
///
/// `None` when there is no sidecar, or it names no file — both of which mean
/// "ask", not "fail". A `handlers` line without a `code` line is nothing to
/// go on either.
pub fn read_link(form: &Path) -> Option<Link> {
    let lines = lines_of(form);
    let value = |key: &str| {
        lines
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let code = PathBuf::from(value("code")?);
    let code = if code.is_absolute() {
        code
    } else {
        form.parent().unwrap_or(Path::new(".")).join(code)
    };
    Some(Link {
        code,
        handlers: value("handlers"),
    })
}

/// Remembers which file holds a form's code, relative to the form where it can
/// be — a sidecar that only works on the machine that wrote it is not worth
/// checking in. A `handlers` line somebody wrote by hand is kept.
pub fn write_link(form: &Path, code: &Path) -> Result<(), String> {
    let base = form.parent().unwrap_or(Path::new("."));
    let shown = relative(base, code).unwrap_or_else(|| code.to_path_buf());
    let mut text = format!("code = {}\n", shown.to_string_lossy());
    if let Some((_, handlers)) = lines_of(form).into_iter().find(|(k, _)| k == "handlers") {
        text.push_str(&format!("handlers = {handlers}\n"));
    }
    std::fs::write(sidecar(form), text)
        .map_err(|why| format!("could not write {}: {why}", sidecar(form).display()))
}

/// `to`, written from `from` with as many `..` as it takes — or `None` when
/// they do not share a root at all, which on Windows is a different drive.
fn relative(from: &Path, to: &Path) -> Option<PathBuf> {
    let from = normal(from)?;
    let to = normal(to)?;
    let common = from
        .components()
        .zip(to.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in from.components().skip(common) {
        out.push("..");
    }
    for part in to.components().skip(common) {
        out.push(part);
    }
    Some(out)
}

/// A path made absolute and freed of `.` and `..`, without touching the disk.
fn normal(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Some(out)
}

/// A path as the status line names it: relative to where the designer was
/// started when it can be, since that is the path the person typed.
pub fn display_name(path: &Path) -> String {
    let shown = std::env::current_dir()
        .ok()
        .and_then(|cwd| relative(&cwd, path))
        .filter(|rel| !rel.starts_with(".."))
        .unwrap_or_else(|| path.to_path_buf());
    shown.to_string_lossy().into_owned()
}

// ----------------------------------------------------------------- spellings

/// The event's name as a Rust function: `set-notify` becomes `set_notify`.
pub fn snake(event: &str) -> String {
    event.replace('-', "_")
}

/// The event's name as an enum variant: `set-notify` becomes `SetNotify`.
///
/// The same spelling `denise-forms`' code generator uses, which is what makes
/// the second rung of [`locate`] find the arm it generated for.
pub fn pascal(event: &str) -> String {
    event
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// A Rust spelling back to the form's: `SetNotify` and `set_notify` both
/// become `set-notify`.
pub fn kebab(rust: &str) -> String {
    let mut out = String::with_capacity(rust.len() + 2);
    for (index, c) in rust.trim_start_matches("r#").chars().enumerate() {
        if c.is_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        } else if c == '_' {
            out.push('-');
        } else {
            out.push(c);
        }
    }
    out
}

// ------------------------------------------------------------- the impl block

/// The lines of `impl … {` for `handlers`, as a start and closing index into
/// `source.lines()`.
///
/// When the type has several, the one for a trait whose name ends in
/// `Handlers` wins — that is the trait the form's code generator will name —
/// then the inherent one, then whichever comes first.
fn impl_block(source: &str, handlers: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let heads: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim();
            line.starts_with("impl")
                && line.ends_with('{')
                && line
                    .split(|c: char| !is_word(c))
                    .any(|word| word == handlers)
        })
        .map(|(index, _)| index)
        .collect();
    let rank = |index: &usize| {
        let line = lines[*index];
        match line.find(" for ") {
            Some(at) => {
                let trait_ = line[..at].trim_start_matches("impl").trim();
                let trait_ = trait_.split('<').next().unwrap_or("").trim();
                if trait_.ends_with("Handlers") { 0 } else { 2 }
            }
            None => 1,
        }
    };
    let start = *heads.iter().min_by_key(|index| (rank(index), **index))?;
    // The closing brace: the line where the braces opened since the head come
    // back to none. Strings and comments holding braces will fool this, and a
    // placeholder landing a line off is what they will cost.
    let mut depth = 0i32;
    for (index, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            return (index > start).then_some((start, index));
        }
    }
    None
}

// --------------------------------------------------------------- the handler

/// Where `event`'s handler is in `source`, as a line and column counted from
/// one — the way an editor's `--goto` counts.
///
/// The ladder, top rung first: a function named for the event, a `match` arm
/// on its variant, the name as a string. The first that answers wins, so a
/// file with both a method and the arm that calls it lands on the method. A
/// `handlers` type puts one more rung on top: that function inside the type's
/// own impl, so two types with a `fn save` do not land on the wrong one.
pub fn locate(source: &str, event: &str, handlers: Option<&str>) -> Option<(usize, usize)> {
    let function = snake(event);
    let variant = pascal(event);
    let literal = format!("\"{event}\"");
    let (plain, raw) = (format!("fn {function}("), format!("fn r#{function}("));
    let arm = format!("::{variant}");
    let is_function = |line: &str| line.contains(&plain) || line.contains(&raw);
    handlers
        .and_then(|handlers| impl_block(source, handlers))
        .and_then(|(start, end)| {
            find(source, &function, |index, line| {
                (start..=end).contains(&index) && is_function(line)
            })
        })
        .or_else(|| find(source, &function, |_, line| is_function(line)))
        .or_else(|| {
            find(source, &variant, |_, line| {
                line.contains(&arm) && line.contains("=>")
            })
        })
        .or_else(|| find(source, &literal, |_, line| line.contains(&literal)))
}

/// The first line `matches` accepts, as a line and column counted from one,
/// the column being where `needle` starts on it.
fn find(
    source: &str,
    needle: &str,
    matches: impl Fn(usize, &str) -> bool,
) -> Option<(usize, usize)> {
    source
        .lines()
        .enumerate()
        .find(|(index, line)| matches(*index, line))
        .map(|(index, line)| {
            let column = line.find(needle).map_or(0, |at| line[..at].chars().count());
            (index + 1, column + 1)
        })
}

/// A handler for `event` that compiles as written and does nothing yet.
///
/// The parameter is the [`Payload`] the widget declares, so a checkbox's
/// handler takes its `bool` from the first line. `fired_by` says which node in
/// which form, so the function explains itself when it is found later. As a
/// `method` it takes `&mut self` and is indented for the impl it will sit in.
pub fn placeholder(
    event: &str,
    payload: Payload,
    fired_by: &str,
    form: &str,
    method: bool,
) -> String {
    let params = match payload {
        Payload::None => "",
        Payload::Bool => "on: bool",
        Payload::Index => "index: usize",
        Payload::Number => "value: f32",
    };
    let (indent, receiver) = if method {
        (
            "    ",
            if params.is_empty() {
                "&mut self"
            } else {
                "&mut self, "
            },
        )
    } else {
        ("", "")
    };
    format!(
        "{indent}/// `{event}` — fired by {fired_by} in {form}.\n\
         {indent}fn {}({receiver}{params}) {{\n\
         {indent}    todo!(\"{event}\")\n\
         {indent}}}\n",
        snake(event)
    )
}

/// The first lines of a code file the designer had to create.
pub fn header(form: &str) -> String {
    format!(
        "//! Code behind `{form}`, started by the designer.\n\
         //!\n\
         //! Reach it with a `mod` line, and hand each handler below to the form's\n\
         //! `match` — `docs/designer.md` says how a form reaches its code.\n"
    )
}

/// What [`ensure`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    /// The handler was already there.
    Found,
    /// A method was added inside the handlers type's impl.
    AddedMethod,
    /// A free function was appended — no handlers type was named, or its
    /// impl is not in this file.
    AddedFunction,
}

/// Makes sure `source` answers `event`, appending a [`placeholder`] when it
/// does not, and says where the handler now is.
pub fn ensure(
    source: &mut String,
    event: &str,
    payload: Payload,
    fired_by: &str,
    form: &str,
    handlers: Option<&str>,
) -> ((usize, usize), Ensured) {
    if let Some(at) = locate(source, event, handlers) {
        return (at, Ensured::Found);
    }
    let block = handlers.and_then(|handlers| impl_block(source, handlers));
    let ensured = match block {
        Some((_, closing)) => {
            // Before the closing brace, with a blank line between it and
            // whatever the impl already holds — unless it holds nothing.
            let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
            let mut inserted = Vec::new();
            if closing > 0
                && !lines[closing - 1].trim().is_empty()
                && !lines[closing - 1].trim_end().ends_with('{')
            {
                inserted.push(String::new());
            }
            inserted.extend(
                placeholder(event, payload, fired_by, form, true)
                    .lines()
                    .map(str::to_string),
            );
            let tail: Vec<String> = lines.drain(closing..).collect();
            lines.extend(inserted);
            lines.extend(tail);
            *source = lines.join("\n");
            source.push('\n');
            Ensured::AddedMethod
        }
        None => {
            if !source.is_empty() && !source.ends_with('\n') {
                source.push('\n');
            }
            if !source.is_empty() {
                source.push('\n');
            }
            source.push_str(&placeholder(event, payload, fired_by, form, false));
            Ensured::AddedFunction
        }
    };
    let at = locate(source, event, handlers).unwrap_or((1, 1));
    (at, ensured)
}

// ------------------------------------------------------------ the vocabulary

/// Every event name `source` answers, spelt the way a form spells them,
/// sorted and without repeats.
///
/// Three places a name can be answered, read together: a string in a `match`
/// arm (the untyped wiring), a variant of an enum whose name ends in `Message`
/// in an arm (the typed dispatch), and a method of the handlers type. Each is
/// a heuristic over text, and each errs on the side of *answering*: a name
/// wrongly thought answered is a load error later, as it would have been
/// anyway, while a name wrongly thought unanswered is a red row over nothing.
pub fn answered(source: &str, handlers: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let is_name = |word: &str| {
        !word.is_empty()
            && word.starts_with(|c: char| c.is_ascii_lowercase())
            && word
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    let block = handlers.and_then(|handlers| impl_block(source, handlers));
    for (index, line) in source.lines().enumerate() {
        if line.contains("=>") {
            // Every string on the line that could be an event name.
            let mut rest = line;
            while let Some(open) = rest.find('"') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else { break };
                let word = &after[..close];
                if is_name(word) {
                    names.push(word.to_string());
                }
                rest = &after[close + 1..];
            }
            // `…Message::Variant` before the arrow, with or without a payload.
            let head = line.split("=>").next().unwrap_or("");
            for piece in head.split("::").skip(1).zip(head.split("::")) {
                let (variant, enum_) = piece;
                let enum_ = enum_
                    .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("");
                if enum_.ends_with("Message") {
                    let variant: String = variant
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if variant.starts_with(|c: char| c.is_ascii_uppercase()) {
                        names.push(kebab(&variant));
                    }
                }
            }
        }
        if let Some((start, end)) = block
            && (start..=end).contains(&index)
            && let Some(at) = line.find("fn ")
        {
            let name: String = line[at + 3..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '#')
                .collect();
            if !name.is_empty() {
                names.push(kebab(&name));
            }
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

// ---------------------------------------------------------------- the editor

/// The editor command with its placeholders filled: a program and its
/// arguments, split on whitespace *before* the path goes in, so a path with a
/// space in it stays one argument.
pub fn command(template: &str, file: &Path, line: usize, column: usize) -> Vec<String> {
    let file = file.to_string_lossy();
    template
        .split_whitespace()
        .map(|word| {
            word.replace("{file}", &file)
                .replace("{line}", &line.to_string())
                .replace("{column}", &column.to_string())
        })
        .collect()
}

/// Opens `file` at `line:column` in the editor `template` names, or failing
/// that in whatever the platform opens a file with. Says which in the result.
pub fn launch(template: &str, file: &Path, line: usize, column: usize) -> Result<String, String> {
    let argv = command(template, file, line, column);
    let Some((program, args)) = argv.split_first() else {
        return Err(String::from("the editor setting is empty"));
    };
    match Command::new(program).args(args).spawn() {
        Ok(_) => Ok(format!("opened {}:{line} in `{program}`", file.display())),
        Err(why) => {
            let (opener, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
                ("open", &[])
            } else if cfg!(target_os = "windows") {
                ("cmd", &["/C", "start", ""])
            } else {
                ("xdg-open", &[])
            };
            Command::new(opener)
                .args(args)
                .arg(file)
                .spawn()
                .map(|_| {
                    format!(
                        "`{program}` would not start ({why}); opened {} with `{opener}` instead, \
                         which cannot go to line {line}",
                        file.display()
                    )
                })
                .map_err(|_| format!("could not start `{program}`: {why}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPED: &str = r#"
impl Typed {
    fn greet(&mut self) {
        todo!()
    }
}

impl DeniseApp for Typed {
    fn update(&mut self) {
        match message {
            HelloMessage::Greet => self.greet(),
            HelloMessage::SetNotify(on) => self.notify = on,
        }
    }
}
"#;

    const UNTYPED: &str = r#"
let built = form.build(&mut ui, root, &mut |name: &str, payload: Payload| {
    match (name, payload) {
        ("greet", Payload::None) => Some(Handler::Plain(Message::Greet)),
        _ => None,
    }
})?;
"#;

    /// The ladder's rungs, in order: a method beats the arm that calls it, the
    /// arm beats the string, and the column lands on the name itself.
    #[test]
    fn the_ladder_finds_a_method_before_an_arm_before_a_string() {
        assert_eq!(locate(TYPED, "greet", None), Some((3, 8)), "the method");
        assert_eq!(
            locate(TYPED, "set-notify", None),
            Some((12, 27)),
            "no method, so the arm — with its payload in parentheses"
        );
        // The untyped wiring's arm holds both the string and the variant, and
        // the variant is the higher rung — same line, which is what counts.
        assert_eq!(
            locate(UNTYPED, "greet", None).map(|(line, _)| line),
            Some(4)
        );
        // The string alone, where nothing spells the variant: a handler table.
        let table = "let handlers = [\n    (\"greet\", greet as fn()),\n];\n";
        assert_eq!(
            locate(table, "greet", None),
            Some((2, 6)),
            "the string rung"
        );
        assert_eq!(locate(TYPED, "cancel", None), None);
    }

    /// Two types with a `fn save`: naming the handlers type lands on its own,
    /// wherever in the file it is.
    #[test]
    fn a_handlers_type_picks_its_own_method_over_another_types() {
        let two = "impl Other {\n    fn save(&mut self) {}\n}\n\nimpl App {\n    fn save(&mut self) {}\n}\n";
        assert_eq!(locate(two, "save", None), Some((2, 8)), "first in the file");
        assert_eq!(
            locate(two, "save", Some("App")),
            Some((6, 8)),
            "the named type's"
        );
        assert_eq!(
            locate(two, "save", Some("Nobody")),
            Some((2, 8)),
            "an unknown type falls back"
        );
    }

    /// A name Rust cannot spell is a raw identifier, which is still a function.
    #[test]
    fn a_raw_identifier_is_still_found() {
        assert_eq!(locate("fn r#match() {}", "match", None), Some((1, 6)));
    }

    /// The spellings match the code generator's, or the second rung would look
    /// for a variant nobody generated — and back again for the vocabulary.
    #[test]
    fn the_names_are_spelt_the_way_the_generator_spells_them() {
        assert_eq!(snake("set-notify"), "set_notify");
        assert_eq!(pascal("set-notify"), "SetNotify");
        assert_eq!(pascal("greet"), "Greet");
        assert_eq!(pascal("full_name"), "FullName");
        assert_eq!(kebab("SetNotify"), "set-notify");
        assert_eq!(kebab("set_notify"), "set-notify");
        assert_eq!(kebab("r#match"), "match");
        assert_eq!(kebab("Greet"), "greet");
    }

    /// The placeholder takes the widget's payload, so it compiles as written
    /// and its first line says what will arrive — and as a method it takes
    /// `self` first.
    #[test]
    fn the_placeholder_is_shaped_by_the_payload() {
        let stub = placeholder(
            "set-notify",
            Payload::Bool,
            "`checkbox` name=notify",
            "forms/hello.dform",
            false,
        );
        assert!(stub.contains("fn set_notify(on: bool) {"), "{stub}");
        assert!(stub.contains("todo!(\"set-notify\")"), "{stub}");
        assert!(stub.starts_with(
            "/// `set-notify` — fired by `checkbox` name=notify in forms/hello.dform."
        ));
        assert!(placeholder("greet", Payload::None, "x", "f", false).contains("fn greet() {"));
        assert!(
            placeholder("chose", Payload::Index, "x", "f", false)
                .contains("fn chose(index: usize) {")
        );
        assert!(
            placeholder("level", Payload::Number, "x", "f", false)
                .contains("fn level(value: f32) {")
        );

        let method = placeholder("set-notify", Payload::Bool, "x", "f", true);
        assert!(
            method.contains("    fn set_notify(&mut self, on: bool) {"),
            "{method}"
        );
        assert!(method.starts_with("    /// "), "indented for the impl");
        assert!(
            placeholder("greet", Payload::None, "x", "f", true).contains("fn greet(&mut self) {")
        );
    }

    /// `ensure` leaves a file that already answers alone, appends to one that
    /// does not, and the second call finds what the first wrote.
    #[test]
    fn ensuring_appends_once_and_then_finds_it() {
        let mut source = String::from(TYPED);
        let before = source.clone();
        assert_eq!(
            ensure(&mut source, "greet", Payload::None, "b", "f", None),
            ((3, 8), Ensured::Found)
        );
        assert_eq!(source, before, "a handler that is there is not touched");

        let (at, what) = ensure(
            &mut source,
            "cancel",
            Payload::None,
            "`button`",
            "forms/x.dform",
            None,
        );
        assert_eq!(what, Ensured::AddedFunction);
        assert!(
            source.ends_with("fn cancel() {\n    todo!(\"cancel\")\n}\n"),
            "{source}"
        );
        assert_eq!(locate(&source, "cancel", None), Some(at));
        assert_eq!(
            ensure(&mut source, "cancel", Payload::None, "b", "f", None),
            (at, Ensured::Found)
        );

        // An empty file gets no leading blank line.
        let mut empty = String::new();
        ensure(&mut empty, "go", Payload::None, "b", "f", None);
        assert!(empty.starts_with("/// `go`"), "{empty}");
    }

    /// With a handlers type, the placeholder is a method inside its impl —
    /// before the closing brace, after a blank line — and the file still
    /// parses as the same shape it was.
    #[test]
    fn a_handlers_type_gets_a_method_inside_its_impl() {
        let mut source = String::from(TYPED);
        let (at, what) = ensure(
            &mut source,
            "cancel",
            Payload::None,
            "`button`",
            "f",
            Some("Typed"),
        );
        assert_eq!(what, Ensured::AddedMethod);
        let expected = "impl Typed {\n    fn greet(&mut self) {\n        todo!()\n    }\n\n    /// `cancel` — fired by `button` in f.\n    fn cancel(&mut self) {\n        todo!(\"cancel\")\n    }\n}\n\nimpl DeniseApp for Typed {";
        assert!(source.contains(expected), "{source}");
        assert_eq!(at, (8, 8), "the line the method landed on");
        assert_eq!(locate(&source, "cancel", Some("Typed")), Some(at));

        // An empty impl gets the method with no blank line before it.
        let mut bare = String::from("impl App {\n}\n");
        ensure(&mut bare, "go", Payload::Bool, "b", "f", Some("App"));
        assert_eq!(
            bare,
            "impl App {\n    /// `go` — fired by b in f.\n    fn go(&mut self, on: bool) {\n        todo!(\"go\")\n    }\n}\n"
        );

        // A type whose impl is not in this file gets a free function, and
        // says so.
        let mut elsewhere = String::from("fn main() {}\n");
        let (_, what) = ensure(&mut elsewhere, "go", Payload::None, "b", "f", Some("App"));
        assert_eq!(what, Ensured::AddedFunction);
        assert!(elsewhere.ends_with("fn go() {\n    todo!(\"go\")\n}\n"));
    }

    /// Of a type's several impls, the one for a `…Handlers` trait is where the
    /// form's handlers go; failing that the inherent one; failing that any.
    #[test]
    fn the_handlers_trait_impl_is_preferred_then_the_inherent_one() {
        let source = "impl Display for App {\n    fn fmt() {}\n}\nimpl App {\n    fn new() {}\n}\nimpl HelloHandlers for App {\n    fn greet(&mut self) {}\n}\n";
        assert_eq!(impl_block(source, "App"), Some((6, 8)), "the trait");
        let no_trait =
            "impl Display for App {\n    fn fmt() {}\n}\nimpl App {\n    fn new() {}\n}\n";
        assert_eq!(impl_block(no_trait, "App"), Some((3, 5)), "the inherent");
        let only = "impl Display for App {\n    fn fmt() {}\n}\n";
        assert_eq!(impl_block(only, "App"), Some((0, 2)), "whatever there is");
        assert_eq!(
            impl_block("impl Application {\n}\n", "App"),
            None,
            "a whole word, not a prefix"
        );
        assert_eq!(
            impl_block("impl<M> Wrapper<M> for App {\n}\n", "App"),
            Some((0, 1))
        );
    }

    /// The vocabulary is read from all three places, spelt as the form spells
    /// it, and does not pick up every enum in the file.
    #[test]
    fn the_vocabulary_is_every_name_the_code_answers() {
        let source = "\
impl App {
    fn new() -> Self { todo!() }
    fn set_notify(&mut self, on: bool) {}
}
fn wire(name: &str) {
    match (name, payload) {
        (\"greet\", Payload::None) => Some(Handler::Plain(Message::Greet)),
        (\"full-name\", _) => None,
        _ => None,
    }
    match message {
        SettingsMessage::Save => self.save(),
        SettingsMessage::SetLevel(v) => self.level = v,
        KeyCode::Escape => self.exit = true,
    }
}
";
        assert_eq!(
            answered(source, Some("App")),
            [
                "full-name",
                "greet",
                "new",
                "save",
                "set-level",
                "set-notify"
            ],
            "strings in arms, `…Message::` variants, and the impl's methods — not `KeyCode`"
        );
        assert_eq!(
            answered(source, None),
            ["full-name", "greet", "save", "set-level"],
            "no handlers type, no methods"
        );
        assert!(answered("fn main() {}", None).is_empty());
    }

    /// The template is split before the path goes in, so a path with a space
    /// stays one argument — and every placeholder is filled.
    #[test]
    fn the_command_keeps_a_path_with_spaces_whole() {
        let argv = command(DEFAULT_EDITOR, Path::new("/My Code/src/main.rs"), 12, 5);
        assert_eq!(argv, ["code", "--goto", "/My Code/src/main.rs:12:5"]);
        let zed = command("zed {file}:{line}", Path::new("a.rs"), 3, 1);
        assert_eq!(zed, ["zed", "a.rs:3"]);
    }

    /// The sidecar sits beside the form, names the code relative to it, reads
    /// back to the same file from anywhere, and keeps a `handlers` line that
    /// was written by hand.
    #[test]
    fn the_sidecar_round_trips_a_relative_link_and_keeps_the_handlers_line() {
        let dir = std::env::temp_dir().join(format!("denise-sidecar-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("forms")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let form = dir.join("forms").join("hello.dform");
        let code = dir.join("src").join("main.rs");
        assert_eq!(sidecar(&form), dir.join("forms").join("hello.designer"));
        assert_eq!(read_link(&form), None, "nothing yet");

        write_link(&form, &code).unwrap();
        let written = std::fs::read_to_string(sidecar(&form)).unwrap();
        assert_eq!(written, "code = ../src/main.rs\n", "relative to the form");
        let link = read_link(&form).unwrap();
        assert_eq!(normal(&link.code), normal(&code));
        assert_eq!(link.handlers, None);

        std::fs::write(sidecar(&form), "code = ../src/main.rs\nhandlers = App\n").unwrap();
        assert_eq!(read_link(&form).unwrap().handlers.as_deref(), Some("App"));
        write_link(&form, &dir.join("src").join("other.rs")).unwrap();
        assert_eq!(
            std::fs::read_to_string(sidecar(&form)).unwrap(),
            "code = ../src/other.rs\nhandlers = App\n",
            "rewriting the code line keeps the handlers line"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
