//! What each node looks like, decided before anything is measured.
//!
//! One preorder walk turns the DOM into a `Vec<ComputedStyle>` parallel to
//! the arena. The defaults a browser ships — headings large and bold, links
//! coloured and underlined, `pre` in a typewriter voice — are a Rust
//! function here rather than a stylesheet, because a stylesheet is a way to
//! let *other people* change the rules, and nobody else edits these.
//! Author CSS, when it arrives, cascades on top of what this pass produces.
//!
//! Sizes are logical pixels; the layout pass owns the one multiply by the
//! display scale, the same rule the gallery follows.

use denise::Color;

use crate::css::{CssDisplay, Decl, FontSize, Stylesheet};
use crate::dom::{Dom, NodeData};

/// How a node participates in layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Display {
    Block,
    Inline,
    ListItem,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextAlign {
    Left,
    Center,
}

/// Everything layout and paint will ever ask about a node.
///
/// Colours are raw [`Color`], not theme roles: a page's palette belongs to
/// its author, which is exactly the case the role-based widgets rightly
/// refuse to serve.
#[derive(Clone, Debug)]
pub struct ComputedStyle {
    pub display: Display,
    /// Logical pixels.
    pub font_size: u16,
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
    pub color: Color,
    pub background: Option<Color>,
    pub underline: bool,
    /// Top, right, bottom, left. Logical pixels.
    pub margin: [i32; 4],
    pub padding: [i32; 4],
    pub text_align: TextAlign,
    /// Whitespace is honoured, lines break only at `\n`.
    pub pre: bool,
    /// Index into [`Cascade::links`] when this node is inside an `<a href>`.
    pub link: Option<usize>,
}

/// The three colours the defaults need from outside: what the theme calls
/// text, what it calls a link, and a whisper of a tint behind code.
pub struct Palette {
    pub text: Color,
    pub link: Color,
    pub tint: Color,
}

/// The output of the style pass.
pub struct Cascade {
    /// Parallel to `Dom::nodes`.
    pub styles: Vec<ComputedStyle>,
    /// Raw `href` values in document order, unresolved. `ComputedStyle::link`
    /// indexes here.
    pub links: Vec<String>,
}

pub fn cascade(dom: &Dom, palette: &Palette, sheet: &Stylesheet) -> Cascade {
    let root = ComputedStyle {
        display: Display::Block,
        font_size: 16,
        bold: false,
        italic: false,
        monospace: false,
        color: palette.text,
        background: None,
        underline: false,
        margin: [0; 4],
        padding: [0; 4],
        text_align: TextAlign::Left,
        pre: false,
        link: None,
    };
    let mut out = Cascade {
        styles: vec![root.clone(); dom.nodes.len()],
        links: Vec::new(),
    };
    style_into(dom, 0, &root, palette, sheet, &mut out);
    out
}

fn style_into(
    dom: &Dom,
    idx: usize,
    parent: &ComputedStyle,
    palette: &Palette,
    sheet: &Stylesheet,
    out: &mut Cascade,
) {
    let style = match &dom.nodes[idx].data {
        // Text has no style of its own; it speaks with its parent's voice —
        // but it is always inline-level, whatever box its parent makes.
        NodeData::Text(_) => ComputedStyle {
            display: Display::Inline,
            ..parent.clone()
        },
        NodeData::Comment => ComputedStyle {
            display: Display::None,
            ..parent.clone()
        },
        NodeData::Document => inherit(parent),
        NodeData::Element { name, .. } => {
            let mut style = ua_defaults(&name.local, parent, palette);
            if style.link.is_none()
                && name.local.as_ref() == "a"
                && let Some(href) = dom.attr(idx, "href")
            {
                out.links.push(href.to_string());
                style.link = Some(out.links.len() - 1);
                style.color = palette.link;
                style.underline = true;
            }
            // The pre-CSS presentation attributes still holding up the old
            // web: they sit below every stylesheet in the cascade.
            if let Some(bg) = dom
                .attr(idx, "bgcolor")
                .and_then(crate::css::parse_color_attr)
            {
                style.background = Some(bg);
            }
            if name.local.as_ref() == "font"
                && let Some(c) = dom
                    .attr(idx, "color")
                    .and_then(crate::css::parse_color_attr)
            {
                style.color = c;
            }
            if dom
                .attr(idx, "align")
                .is_some_and(|a| a.eq_ignore_ascii_case("center"))
            {
                style.text_align = TextAlign::Center;
            }
            // The author's turn: matched rules in cascade order, then the
            // style attribute, which outranks them all.
            if !sheet.is_empty() {
                for decl in sheet.declarations_for(dom, idx) {
                    apply(decl, &mut style, parent);
                }
            }
            if let Some(inline) = dom.attr(idx, "style") {
                for decl in &Stylesheet::parse_inline(inline) {
                    apply(decl, &mut style, parent);
                }
            }
            style
        }
    };
    out.styles[idx] = style;
    let style = out.styles[idx].clone();
    for &child in &dom.nodes[idx].children {
        style_into(dom, child, &style, palette, sheet, out);
    }
}

/// One declaration onto one computed style. Relative font sizes resolve
/// against the parent — the inheritance chain has already run, so `em`
/// means what the author meant.
fn apply(decl: &Decl, style: &mut ComputedStyle, parent: &ComputedStyle) {
    match decl {
        Decl::Color(color) => style.color = *color,
        Decl::Background(background) => style.background = *background,
        Decl::FontSize(size) => {
            let px = match size {
                FontSize::Px(px) => *px,
                FontSize::Em(factor) => (f32::from(parent.font_size) * factor).round() as i32,
            };
            style.font_size = px.clamp(6, 96) as u16;
        }
        Decl::Bold(bold) => style.bold = *bold,
        Decl::Italic(italic) => style.italic = *italic,
        Decl::Mono(mono) => style.monospace = *mono,
        Decl::Align(align) => style.text_align = *align,
        Decl::Underline(underline) => style.underline = *underline,
        Decl::Display(display) => {
            style.display = match display {
                CssDisplay::None => Display::None,
                CssDisplay::Block => Display::Block,
                CssDisplay::Inline => Display::Inline,
            };
        }
        Decl::Margin(side, value) => style.margin[*side] = *value,
        Decl::Padding(side, value) => style.padding[*side] = *value,
    }
}

/// What a child starts from: the inherited half of its parent, none of the
/// box half.
fn inherit(parent: &ComputedStyle) -> ComputedStyle {
    ComputedStyle {
        display: Display::Inline,
        background: None,
        margin: [0; 4],
        padding: [0; 4],
        ..parent.clone()
    }
}

/// The browser's own opinion of each tag, before any author gets a say.
fn ua_defaults(tag: &str, parent: &ComputedStyle, palette: &Palette) -> ComputedStyle {
    let mut s = inherit(parent);
    // A helper for the heading pattern: a size, bold, and a margin that
    // scales with the size the way 0.67em–1em margins do.
    let heading = |size: u16, s: &mut ComputedStyle| {
        s.display = Display::Block;
        s.font_size = size;
        s.bold = true;
        let v = (i32::from(size) * 2) / 3;
        s.margin = [v, 0, v, 0];
    };
    match tag {
        // The parts of a page that are about the page, not on it.
        "head" | "script" | "style" | "template" | "noscript" | "meta" | "link" | "title"
        | "base" | "datalist" | "param" | "source" | "track" | "area" => {
            s.display = Display::None;
        }

        "html" => s.display = Display::Block,
        "body" => {
            s.display = Display::Block;
            s.margin = [8, 8, 8, 8];
        }

        "h1" => heading(32, &mut s),
        "h2" => heading(24, &mut s),
        "h3" => heading(19, &mut s),
        "h4" => heading(16, &mut s),
        "h5" => heading(13, &mut s),
        "h6" => heading(11, &mut s),

        "p" | "dl" => {
            s.display = Display::Block;
            s.margin = [16, 0, 16, 0];
        }
        "ul" | "ol" => {
            s.display = Display::Block;
            s.margin = [16, 0, 16, 0];
            s.padding = [0, 0, 0, 40];
        }
        "li" => s.display = Display::ListItem,
        "dd" => {
            s.display = Display::Block;
            s.margin = [0, 0, 0, 40];
        }
        "blockquote" | "figure" => {
            s.display = Display::Block;
            s.margin = [16, 40, 16, 40];
        }
        "pre" => {
            s.display = Display::Block;
            s.monospace = true;
            s.pre = true;
            s.margin = [16, 0, 16, 0];
            s.padding = [8, 8, 8, 8];
            s.background = Some(palette.tint);
        }
        "code" | "kbd" | "samp" | "tt" => s.monospace = true,

        // Generic containers.
        "div" | "address" | "article" | "aside" | "footer" | "header" | "main" | "nav"
        | "section" | "figcaption" | "fieldset" | "form" | "caption" | "details" | "summary"
        | "dt" | "hr" => {
            s.display = Display::Block;
        }
        // The table family, linearised: every cell a block of its own —
        // readable is the promise; columns are not. A table also stops an
        // inherited `text-align` at its border, because the ancestral
        // `<center>` was centring the *table*, not every line in it.
        "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" => {
            s.display = Display::Block;
            s.text_align = TextAlign::Left;
        }
        "th" => {
            s.display = Display::Block;
            s.text_align = TextAlign::Left;
            s.bold = true;
        }

        "b" | "strong" => s.bold = true,
        "i" | "em" | "cite" | "var" | "dfn" => s.italic = true,
        "u" | "ins" => s.underline = true,
        "small" => s.font_size = 13,
        "big" => s.font_size = 19,
        "center" => {
            s.display = Display::Block;
            s.text_align = TextAlign::Center;
        }

        // Everything unknown flows inline in its parent's voice, which is
        // HTML's own rule for unknown tags.
        _ => {}
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette {
            text: Color::rgb(20, 20, 20),
            link: Color::rgb(0, 80, 200),
            tint: Color::rgb(240, 240, 240),
        }
    }

    fn styled(html: &str) -> (Dom, Cascade) {
        let dom = Dom::parse(html);
        let cascade = cascade(&dom, &palette(), &Stylesheet::parse(""));
        (dom, cascade)
    }

    #[test]
    fn author_rules_land_and_inline_style_outranks_them() {
        let dom = Dom::parse(
            r#"<style>p { color: red; margin-top: 4px; }</style>
               <p style="color: navy">t</p>"#,
        );
        let sheet = Stylesheet::parse(&dom.text_content(dom.find("style").unwrap()));
        let c = cascade(&dom, &palette(), &sheet);
        let p = dom.find("p").unwrap();
        assert_eq!(c.styles[p].color, Color::rgb(0, 0, 128), "inline wins");
        assert_eq!(c.styles[p].margin[0], 4, "the sheet's margin stands");
    }

    #[test]
    fn display_none_silences_a_subtree() {
        let dom = Dom::parse(r#"<div class="nav">chrome</div><p>content</p>"#);
        let sheet = Stylesheet::parse(".nav { display: none; }");
        let c = cascade(&dom, &palette(), &sheet);
        let div = dom.find("div").unwrap();
        assert_eq!(c.styles[div].display, Display::None);
    }

    #[test]
    fn em_sizes_compound_through_the_tree() {
        let dom =
            Dom::parse(r#"<div style="font-size: 20px"><p style="font-size: 2em">t</p></div>"#);
        let c = cascade(&dom, &palette(), &Stylesheet::parse(""));
        let p = dom.find("p").unwrap();
        assert_eq!(c.styles[p].font_size, 40);
    }

    #[test]
    fn a_heading_is_large_bold_and_a_block() {
        let (dom, c) = styled("<h1>title</h1>");
        let h1 = dom.find("h1").unwrap();
        let s = &c.styles[h1];
        assert_eq!(s.display, Display::Block);
        assert_eq!(s.font_size, 32);
        assert!(s.bold);
    }

    #[test]
    fn bold_inherits_into_nested_text() {
        let (dom, c) = styled("<p><b>loud <i>and slanted</i></b></p>");
        let i = dom.find("i").unwrap();
        assert!(c.styles[i].bold, "bold flows into the italic span");
        assert!(c.styles[i].italic);
        // The text inside speaks with the same voice.
        let text = dom.nodes[i].children[0];
        assert!(c.styles[text].bold && c.styles[text].italic);
    }

    #[test]
    fn a_link_is_coloured_underlined_and_recorded() {
        let (dom, c) = styled(r#"<a href="/x">go</a>"#);
        let a = dom.find("a").unwrap();
        let s = &c.styles[a];
        assert_eq!(s.link, Some(0));
        assert!(s.underline);
        assert_eq!(c.links, vec!["/x".to_string()]);
        // The anchor's text is clickable too.
        let text = dom.nodes[a].children[0];
        assert_eq!(c.styles[text].link, Some(0));
    }

    #[test]
    fn an_anchor_without_href_is_not_a_link() {
        let (dom, c) = styled("<a name=x>here</a>");
        let a = dom.find("a").unwrap();
        assert_eq!(c.styles[a].link, None);
        assert!(c.links.is_empty());
    }

    #[test]
    fn margins_do_not_inherit() {
        let (dom, c) = styled("<p><span>in</span></p>");
        let span = dom.find("span").unwrap();
        assert_eq!(c.styles[span].margin, [0; 4]);
        assert_eq!(c.styles[span].display, Display::Inline);
    }

    #[test]
    fn script_and_head_are_gone() {
        let (dom, c) = styled("<head><title>t</title></head><body><script>x</script>hi</body>");
        let script = dom.find("script").unwrap();
        let head = dom.find("head").unwrap();
        assert_eq!(c.styles[script].display, Display::None);
        assert_eq!(c.styles[head].display, Display::None);
    }
}
