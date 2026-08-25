//! `denise-forms` — checking, rendering and formatting a form without an
//! application.
//!
//! A form that is hand-edited will be hand-broken, and the place to find that out
//! is a terminal or a CI run rather than a panel in a lobby.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use denise::{BufferAge, Frame, PixelFormat, Rect, Size, Theme, theme};
use denise_forms::{Form, Handler, Payload, Picture, Wiring};
use denise_ui::{Ui, Void};
use kdl::{KdlDocument, KdlNode, KdlValue};

const USAGE: &str = "\
denise-forms — the DeniseUI form file

    denise-forms check  <file.dform>...          parse, build and lint
    denise-forms render <file.dform> [out.ppm]   draw one frame, no display needed

Options
    --theme <dark|light|high-contrast>   render: which theme (default: the file's)
    --scale <factor>                     render: draw at this scale, e.g. 2 or 0.75
    --size <WxH>                         render: fit the form to this surface, by its
                                         own `scaling=` rule
    --font <path.ttf>                    render: a real font instead of the built-in one
    --quiet                              check: say nothing unless something is wrong
    --no-lint                            check: syntax and building only, no geometry
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("denise-forms: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(ExitCode::FAILURE);
    };
    let rest = &args[1..];
    match command.as_str() {
        "check" => check(rest),
        "render" => render(rest),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "-V" | "--version" | "version" => {
            println!("denise-forms {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("no command `{other}`\n\n{USAGE}")),
    }
}

/// Splits arguments into flags and file paths.
fn split(args: &[String]) -> (Vec<(&str, Option<&str>)>, Vec<&str>) {
    let mut flags = Vec::new();
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(name) = arg.strip_prefix("--") {
            // A flag that takes a value takes the next argument.
            let takes_value = matches!(name, "theme" | "font" | "scale" | "size");
            let value = if takes_value {
                i += 1;
                args.get(i).map(String::as_str)
            } else {
                None
            };
            flags.push((name, value));
        } else {
            files.push(arg);
        }
        i += 1;
    }
    (flags, files)
}

fn has(flags: &[(&str, Option<&str>)], name: &str) -> bool {
    flags.iter().any(|(f, _)| *f == name)
}

fn value<'a>(flags: &[(&'a str, Option<&'a str>)], name: &str) -> Option<&'a str> {
    flags.iter().find(|(f, _)| *f == name).and_then(|(_, v)| *v)
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

// ----------------------------------------------------------------------- check

/// Wiring that answers every name and refuses every picture.
///
/// `check` is about the file, not about an application: a message name it cannot
/// know is not an error here, and a picture it cannot decode is reported as a
/// warning rather than stopping the build. What it *does* check is that the file
/// asks for shapes that exist.
struct Checking {
    missing: Vec<String>,
}

impl Wiring<Void> for Checking {
    fn message(&mut self, _name: &str, payload: Payload) -> Option<Handler<Void>> {
        Some(match payload {
            Payload::None => Handler::Plain(Void),
            Payload::Bool => Handler::Bool(|_| Void),
            Payload::Index => Handler::Index(|_| Void),
            Payload::Number => Handler::Number(|_| Void),
        })
    }

    fn asset(&mut self, path: &str) -> Option<Picture> {
        // A stand-in, so a form with pictures still builds under `check` on a
        // machine that has none. Whether the file is actually there is a
        // separate question, reported separately.
        self.missing.push(path.to_string());
        Some(Picture {
            pixels: vec![0; 1],
            size: Size::new(1, 1),
        })
    }
}

fn check(args: &[String]) -> Result<ExitCode, String> {
    let (flags, files) = split(args);
    if files.is_empty() {
        return Err(format!("check needs a file\n\n{USAGE}"));
    }
    let quiet = has(&flags, "quiet");
    let lint = !has(&flags, "no-lint");
    let mut bad = false;

    for path in files {
        let source = read(path)?;
        let form = match Form::parse(&source) {
            Ok(form) => form,
            Err(error) => {
                println!("{path}:{error}");
                bad = true;
                continue;
            }
        };

        let mut wiring = Checking {
            missing: Vec::new(),
        };
        let mut ui: Ui<Void> = Ui::new(form.size(), form.theme());
        let root = ui.root();
        if let Err(error) = form.build(&mut ui, root, &mut wiring) {
            println!("{path}:{error}");
            bad = true;
            continue;
        }

        let mut warnings = 0;
        // A picture the file names and the filesystem does not have. A warning
        // rather than an error: a form may legitimately be checked away from the
        // assets it will ship beside.
        let base = Path::new(path).parent().unwrap_or(Path::new("."));
        for asset in &wiring.missing {
            if !base.join(asset).exists() {
                println!("{path}: warning: `{asset}` is not there, relative to this file");
                warnings += 1;
            }
        }
        if lint {
            warnings += geometry(path, &source);
        }

        if !quiet {
            let name = form.title();
            let Size { width, height } = form.size();
            let suffix = if warnings == 0 {
                String::new()
            } else {
                format!(
                    ", {warnings} warning{}",
                    if warnings == 1 { "" } else { "s" }
                )
            };
            println!("{path}: ok — \"{name}\", {width}x{height}{suffix}");
        }
    }
    Ok(if bad {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

// ------------------------------------------------------------------- the lint

/// Warns about rectangles that are legal and are usually mistakes.
///
/// Two of them: a node whose rectangle leaves its parent, and a pair of siblings
/// that overlap. Both are sometimes meant — a scrim covers the surface on
/// purpose, a `collapse`'s content sits outside it while closed — so neither is
/// an error, and both are suppressed where the file has already said something
/// that makes them expected.
///
/// This reads the *document* rather than the built tree, because it is about what
/// the file says. Anchoring and docking derive different rectangles at other
/// sizes, and a lint that chased those would report a design as broken for being
/// resizable.
///
/// It parses its own copy rather than borrowing the one inside [`Form`]. That
/// keeps `kdl` out of this crate's public API, where a major version of it would
/// otherwise become a breaking change for everybody who only wanted to load a
/// form.
fn geometry(path: &str, source: &str) -> usize {
    fn rect(node: &KdlNode) -> Option<Rect> {
        let axis = |name: &str| node.get(name).and_then(KdlValue::as_integer);
        Some(Rect::new(
            axis("x")? as i32,
            axis("y")? as i32,
            axis("w")? as i32,
            axis("h")? as i32,
        ))
    }
    fn flag(node: &KdlNode, name: &str) -> bool {
        node.get(name).and_then(KdlValue::as_bool) == Some(true)
    }
    fn label(node: &KdlNode) -> String {
        match node.get("name").and_then(KdlValue::as_string) {
            Some(name) => format!("{} `{name}`", node.name().value()),
            None => node.name().value().to_string(),
        }
    }

    /// Every node that takes part in ordinary placement.
    fn placed(parent: &KdlNode) -> Vec<&KdlNode> {
        parent
            .children()
            .map(|d| {
                d.nodes()
                    .iter()
                    .filter(|n| {
                        n.get("dock").is_none()
                            && !flag(n, "visible")
                            && n.get("visible").and_then(KdlValue::as_bool) != Some(false)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    let Ok(doc) = source.parse::<KdlDocument>() else {
        // Unreachable: `check` has already parsed this through `Form`.
        return 0;
    };
    let Some(root) = doc.nodes().first() else {
        return 0;
    };
    // The `form` node states its extent as `width`/`height` rather than as a
    // rectangle, so its own children need it spelled out — without this, the
    // nodes most likely to run off the edge are the only ones never checked.
    let surface = {
        let axis = |name: &str| root.get(name).and_then(KdlValue::as_integer);
        match (axis("width"), axis("height")) {
            (Some(w), Some(h)) => Some(Rect::new(0, 0, w as i32, h as i32)),
            _ => None,
        }
    };

    let mut warnings = 0;
    let mut work = vec![(root, surface)];
    while let Some((parent, bounds)) = work.pop() {
        // Three parents place their children somewhere other than where the
        // rectangles say, so comparing those rectangles would be nonsense. A
        // stack runs its children down its own axis. A viewport is *expected* to
        // hold more than it shows. And a `collapse`'s content sits below its
        // header — a height this lint cannot know, since it is a theme metric —
        // and is clipped away entirely while the section is closed.
        let arranges = parent.get("stack").is_some()
            || flag(parent, "scroll")
            || parent.name().value() == "collapse";
        let children = placed(parent);

        if !arranges {
            for (i, child) in children.iter().enumerate() {
                let Some(r) = rect(child) else { continue };
                if let Some(b) = bounds
                    && (r.x < 0 || r.y < 0 || r.x + r.width > b.width || r.y + r.height > b.height)
                {
                    let at = denise_forms::At::of(source, child.span().offset());
                    println!(
                        "{path}:{at}: warning: {} leaves its parent — {},{} {}x{} in {}x{}",
                        label(child),
                        r.x,
                        r.y,
                        r.width,
                        r.height,
                        b.width,
                        b.height
                    );
                    warnings += 1;
                }
                for other in &children[i + 1..] {
                    let Some(o) = rect(other) else { continue };
                    if r.intersect(&o).is_some() {
                        let at = denise_forms::At::of(source, other.span().offset());
                        println!(
                            "{path}:{at}: warning: {} overlaps {}",
                            label(other),
                            label(child)
                        );
                        warnings += 1;
                    }
                }
            }
        }

        for child in parent.children().map(|d| d.nodes()).unwrap_or_default() {
            if child.children().is_some() {
                work.push((child, rect(child)));
            }
        }
    }
    warnings
}

// ---------------------------------------------------------------------- render

#[cfg(feature = "cli")]
struct Drawing {
    base: PathBuf,
    failed: Vec<String>,
}

#[cfg(feature = "cli")]
impl Wiring<Void> for Drawing {
    fn message(&mut self, _name: &str, payload: Payload) -> Option<Handler<Void>> {
        Some(match payload {
            Payload::None => Handler::Plain(Void),
            Payload::Bool => Handler::Bool(|_| Void),
            Payload::Index => Handler::Index(|_| Void),
            Payload::Number => Handler::Number(|_| Void),
        })
    }

    fn asset(&mut self, path: &str) -> Option<Picture> {
        let full = self.base.join(path);
        let bytes = match std::fs::read(&full) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.failed.push(format!("{path}: {e}"));
                return None;
            }
        };
        match denise_image::decode(&bytes) {
            Ok(picture) => {
                let (pixels, size) = picture.into_parts();
                Some(Picture { pixels, size })
            }
            Err(e) => {
                self.failed.push(format!("{path}: {e}"));
                None
            }
        }
    }
}

fn theme_named(name: &str) -> Result<Theme, String> {
    Ok(match name {
        "dark" => theme::DARK,
        "light" => theme::LIGHT,
        "high-contrast" => theme::HIGH_CONTRAST,
        other => {
            return Err(format!(
                "no theme called `{other}`: dark, light, high-contrast"
            ));
        }
    })
}

fn render(args: &[String]) -> Result<ExitCode, String> {
    let (flags, files) = split(args);
    let Some(&path) = files.first() else {
        return Err(format!("render needs a file\n\n{USAGE}"));
    };
    let out = files.get(1).copied().unwrap_or("form.ppm");

    let source = read(path)?;
    let form = Form::parse(&source).map_err(|e| format!("{path}:{e}"))?;
    let theme = match value(&flags, "theme") {
        Some(name) => theme_named(name)?,
        None => form.theme(),
    };

    let design = form.size();
    if design.width == 0 || design.height == 0 {
        return Err(format!(
            "{path}: a form of {}x{} has nothing to draw",
            design.width, design.height
        ));
    }

    // Two ways to ask for a size, and they answer different questions. `--scale`
    // is "the same panel at a higher density": one factor, and the picture grows
    // with the form. `--size` is "this actual panel": the surface is what was
    // asked for and the form's own `scaling=` decides what it does with it,
    // which is the thing that cannot be reviewed any other way.
    let (size, fit) = match (value(&flags, "size"), value(&flags, "scale")) {
        (Some(_), Some(_)) => {
            return Err(String::from(
                "--size and --scale ask different questions; give one",
            ));
        }
        (Some(spec), None) => {
            let surface = surface_size(spec)?;
            (surface, form.fit(surface))
        }
        (None, Some(spec)) => {
            // The picture grows with the form, so the surface *is* the form and
            // there is nothing to centre it in.
            let scale = factor(spec)?;
            let grown = Rect::from_size(design).scaled(scale);
            let surface = Size::new(grown.width.max(1) as u32, grown.height.max(1) as u32);
            (
                surface,
                denise_forms::Placement {
                    x: scale,
                    y: scale,
                    rect: Rect::from_size(surface),
                },
            )
        }
        (None, None) => (design, form.fit(design)),
    };

    // Scaled once, here, or every widget is the old size inside a new rectangle.
    let mut ui: Ui<Void> = Ui::new(size, theme.scaled(fit.uniform()));

    if let Some(font) = value(&flags, "font") {
        add_font(&mut ui, font)?;
    }

    let root = ui.root();
    // The form's own rectangle, where the fit put it. At 1:1 with no `--size`
    // this is the whole surface and the panel is what the form would have been
    // drawn on anyway.
    let stage = ui
        .add(
            root,
            denise_ui::widgets::Panel::filled(form.background()),
            fit.rect,
        )
        .ok_or_else(|| String::from("the root would not take a child"))?;
    let mut wiring = Drawing {
        base: Path::new(path)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf(),
        failed: Vec::new(),
    };
    let outcome = form.build_fitted(&mut ui, stage, fit, &mut wiring);
    // The loader's own reasons first: `build` reports only that a picture could
    // not be had, and "no such file" or "not a PNG" is the half worth reading.
    for failure in &wiring.failed {
        eprintln!("denise-forms: {failure}");
    }
    outcome.map_err(|e| format!("{path}:{e}"))?;

    // Cropped rather than fitted, which is what `scaling=none` means when the
    // surface is too small. Visible in the picture, but say it too: somebody
    // rendering at a panel's size is asking whether it fits.
    if fit.rect.x < 0 || fit.rect.y < 0 {
        eprintln!(
            "denise-forms: {path} is {}x{} and says `scaling={}`, so it is cropped by \
             {}x{} on this surface",
            design.width,
            design.height,
            denise_forms::Scaling::NAMES[form.scaling() as usize],
            (-fit.rect.x * 2).max(0),
            (-fit.rect.y * 2).max(0),
        );
    }

    write_ppm(&mut ui, size, out)?;
    eprintln!("wrote {out} at {}x{}", size.width, size.height);
    Ok(ExitCode::SUCCESS)
}

/// `--scale 2`, `--scale 0.75`.
fn factor(spec: &str) -> Result<f32, String> {
    let scale: f32 = spec
        .parse()
        .map_err(|_| format!("`{spec}` is not a scale; try 2 or 0.75"))?;
    // Not zero, not negative, and not so large that the picture is measured in
    // gigabytes before anything says why.
    if !(0.05..=16.0).contains(&scale) {
        return Err(format!("a scale of {scale} is outside 0.05 to 16"));
    }
    Ok(scale)
}

/// `--size 1920x1080`.
fn surface_size(spec: &str) -> Result<Size, String> {
    let wrong = || format!("`{spec}` is not a size; try 1920x1080");
    let (w, h) = spec.split_once(['x', 'X']).ok_or_else(wrong)?;
    let parse = |text: &str| -> Result<u32, String> {
        let value: u32 = text.trim().parse().map_err(|_| wrong())?;
        (1..=16384)
            .contains(&value)
            .then_some(value)
            .ok_or_else(|| format!("{value} is outside 1 to 16384"))
    };
    Ok(Size::new(parse(w)?, parse(h)?))
}

#[cfg(feature = "truetype")]
fn add_font(ui: &mut Ui<Void>, path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("font");
    let source = denise_text::TrueTypeSource::from_bytes(name, &bytes)
        .map_err(|e| format!("{path}: {e}"))?;
    ui.add_font(Box::new(source));
    Ok(())
}

#[cfg(not(feature = "truetype"))]
fn add_font(_ui: &mut Ui<Void>, _path: &str) -> Result<(), String> {
    Err(String::from(
        "this build has no TrueType support; rebuild with `--features truetype`",
    ))
}

fn write_ppm(ui: &mut Ui<Void>, size: Size, path: &str) -> Result<(), String> {
    use std::io::Write as _;

    let mut pixels = vec![0u32; (size.width as usize) * (size.height as usize)];
    {
        let mut frame = Frame::new(
            &mut pixels,
            size,
            size.width,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .map_err(|e| format!("{e:?}"))?;
        ui.paint(&mut frame);
    }

    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut out = std::io::BufWriter::new(file);
    let write = |out: &mut std::io::BufWriter<std::fs::File>| -> std::io::Result<()> {
        write!(out, "P6\n{} {}\n255\n", size.width, size.height)?;
        for word in &pixels {
            out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
        }
        out.flush()
    };
    write(&mut out).map_err(|e| format!("{path}: {e}"))
}
