//! The author's say: a working subset of CSS.
//!
//! cssparser does the tokenizing — comments, strings, escapes, the parts
//! that make naive splitting wrong — and this file does the deciding. The
//! subset is chosen by what the renderer can honour: colours, font size
//! and voice, alignment, underlines, margins, padding, and `display`.
//! Everything else is skipped *by token*, which is the property that makes
//! real-world stylesheets survivable: an unknown declaration costs a skip,
//! never a parse failure.
//!
//! Selectors: tag, `.class`, `#id`, compounds of those, and the descendant
//! combinator. A selector using anything this matcher cannot evaluate —
//! attributes, pseudo-classes — is dropped whole, because "`a:hover`
//! applied to every `a`" is worse than nothing. Specificity is the usual
//! triple; source order breaks ties.
//!
//! Matching is bucketed by the rightmost compound (id, then classes, then
//! tag), the standard trick that keeps a Wikipedia-sized sheet — thousands
//! of rules — from meeting every node in the document.

use std::collections::HashMap;

use cssparser::{Delimiter, Parser, ParserInput, Token};
use denise::Color;

use crate::dom::Dom;
use crate::style::TextAlign;

pub struct Stylesheet {
    rules: Vec<Rule>,
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

struct Rule {
    chain: Vec<Compound>,
    specificity: (u32, u32, u32),
    order: usize,
    decls: Vec<Decl>,
}

#[derive(Default, Clone)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrCheck>,
}

/// `[attr]`, `[attr=v]`, `[attr~=v]` — the three forms modern Wikipedia
/// floats its thumbnails with (`figure[typeof~=mw:File/Thumb]`). The
/// fancier operators reject their selector, as pseudo-classes do.
#[derive(Clone)]
struct AttrCheck {
    name: String,
    op: AttrOp,
    value: String,
}

#[derive(Clone, Copy, PartialEq)]
enum AttrOp {
    Exists,
    Equals,
    Includes,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    Color(Color),
    /// `None` is `background: none` / `transparent`.
    Background(Option<Color>),
    FontSize(FontSize),
    Bold(bool),
    Italic(bool),
    Mono(bool),
    Align(TextAlign),
    Underline(bool),
    Display(CssDisplay),
    /// Side index 0..4 = top, right, bottom, left. Logical px.
    Margin(usize, i32),
    Padding(usize, i32),
    Float(Option<FloatSide>),
    Clear(Option<ClearSide>),
    Width(Length),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontSize {
    Px(i32),
    /// Relative to the inherited size.
    Em(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearSide {
    Left,
    Right,
    Both,
}

/// A width, in the units a page actually uses for one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Px(i32),
    /// Of the element's own font size.
    Em(f32),
    /// Of the containing block's width.
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CssDisplay {
    None,
    Block,
    Inline,
}

impl Stylesheet {
    /// `viewport` is the width media queries are asked against, in CSS
    /// (logical) pixels.
    pub fn parse(css: &str, viewport: i32) -> Self {
        let mut sheet = Self {
            rules: Vec::new(),
            by_id: HashMap::new(),
            by_class: HashMap::new(),
            by_tag: HashMap::new(),
            universal: Vec::new(),
        };
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        parse_rules(&mut parser, &mut sheet, viewport);
        sheet
    }

    /// Also how `style=""` attributes are read — a declaration list with no
    /// selector around it.
    pub fn parse_inline(declarations: &str) -> Vec<Decl> {
        let mut input = ParserInput::new(declarations);
        let mut parser = Parser::new(&mut input);
        parse_declarations(&mut parser)
    }

    fn push(&mut self, chain: Vec<Compound>, decls: Vec<Decl>) {
        let specificity = chain.iter().fold((0, 0, 0), |acc, c| {
            (
                acc.0 + u32::from(c.id.is_some()),
                acc.1 + (c.classes.len() + c.attrs.len()) as u32,
                acc.2 + u32::from(c.tag.is_some()),
            )
        });
        let index = self.rules.len();
        let target = chain.last().expect("selectors have a target").clone();
        self.rules.push(Rule {
            chain,
            specificity,
            order: index,
            decls,
        });
        if let Some(id) = &target.id {
            self.by_id.entry(id.clone()).or_default().push(index);
        } else if let Some(class) = target.classes.first() {
            self.by_class.entry(class.clone()).or_default().push(index);
        } else if let Some(tag) = &target.tag {
            self.by_tag.entry(tag.clone()).or_default().push(index);
        } else {
            self.universal.push(index);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Every declaration that applies to `idx`, in cascade order: lowest
    /// specificity first, source order breaking ties. The caller applies
    /// them in sequence and the last word wins, which *is* the cascade.
    pub fn declarations_for(&self, dom: &Dom, idx: usize) -> Vec<&Decl> {
        let mut candidates: Vec<usize> = Vec::new();
        if let Some(id) = dom.attr(idx, "id")
            && let Some(rules) = self.by_id.get(id)
        {
            candidates.extend_from_slice(rules);
        }
        if let Some(classes) = dom.attr(idx, "class") {
            for class in classes.split_ascii_whitespace() {
                if let Some(rules) = self.by_class.get(class) {
                    candidates.extend_from_slice(rules);
                }
            }
        }
        if let Some(tag) = dom.tag(idx)
            && let Some(rules) = self.by_tag.get(tag)
        {
            candidates.extend_from_slice(rules);
        }
        candidates.extend_from_slice(&self.universal);

        let mut matched: Vec<&Rule> = candidates
            .into_iter()
            .map(|i| &self.rules[i])
            .filter(|rule| matches(dom, idx, &rule.chain))
            .collect();
        matched.sort_by_key(|rule| (rule.specificity, rule.order));
        matched.dedup_by_key(|rule| rule.order);
        matched.iter().flat_map(|rule| &rule.decls).collect()
    }
}

fn matches(dom: &Dom, idx: usize, chain: &[Compound]) -> bool {
    let (target, ancestors) = chain.split_last().expect("non-empty chain");
    if !compound_matches(dom, idx, target) {
        return false;
    }
    // Each remaining compound must match some ancestor, outward-bound.
    let mut current = dom.nodes[idx].parent;
    'chain: for compound in ancestors.iter().rev() {
        while let Some(ancestor) = current {
            current = dom.nodes[ancestor].parent;
            if compound_matches(dom, ancestor, compound) {
                continue 'chain;
            }
        }
        return false;
    }
    true
}

fn compound_matches(dom: &Dom, idx: usize, compound: &Compound) -> bool {
    if let Some(tag) = &compound.tag
        && dom.tag(idx) != Some(tag.as_str())
    {
        return false;
    }
    if let Some(id) = &compound.id
        && dom.attr(idx, "id") != Some(id.as_str())
    {
        return false;
    }
    if !compound.classes.is_empty() {
        let Some(classes) = dom.attr(idx, "class") else {
            return false;
        };
        let have: Vec<&str> = classes.split_ascii_whitespace().collect();
        if !compound.classes.iter().all(|c| have.contains(&c.as_str())) {
            return false;
        }
    }
    for check in &compound.attrs {
        let Some(value) = dom.attr(idx, &check.name) else {
            return false;
        };
        let holds = match check.op {
            AttrOp::Exists => true,
            AttrOp::Equals => value == check.value,
            AttrOp::Includes => value.split_ascii_whitespace().any(|v| v == check.value),
        };
        if !holds {
            return false;
        }
    }
    true
}

/// Comma-separated selectors; each is a whitespace chain of compounds.
/// `>` is read as a descendant, which is looser than the author meant and
/// right more often than dropping the rule. Anything this matcher cannot
/// evaluate — pseudo-classes, sibling combinators, exotic attribute
/// operators — rejects that selector alone.
fn parse_selectors(text: &str) -> Vec<Vec<Compound>> {
    text.split(',')
        .filter_map(|selector| {
            let cleaned = selector.replace('>', " ");
            let mut chain = Vec::new();
            for part in cleaned.split_ascii_whitespace() {
                if part == "*" {
                    chain.push(Compound::default());
                    continue;
                }
                if part == "+" || part == "~" {
                    return None;
                }
                // `:link` is "an unvisited link", and this browser has
                // never visited anywhere — so it means every link, and
                // dropping `a:link { color }` would unstyle half the old
                // web. Every other pseudo-class stays unanswerable.
                let part = part.strip_suffix(":link").unwrap_or(part);
                if part.contains(':') {
                    return None;
                }
                chain.push(parse_compound(part)?);
            }
            (!chain.is_empty()).then_some(chain)
        })
        .collect()
}

fn parse_compound(text: &str) -> Option<Compound> {
    let mut compound = Compound::default();
    let mut rest = text;
    if !rest.starts_with(['.', '#', '[']) {
        let end = rest.find(['.', '#', '[']).unwrap_or(rest.len());
        compound.tag = Some(rest[..end].to_ascii_lowercase());
        rest = &rest[end..];
    }
    while !rest.is_empty() {
        let kind = rest.as_bytes()[0];
        rest = &rest[1..];
        if kind == b'[' {
            let close = rest.find(']')?;
            compound.attrs.push(parse_attr_check(&rest[..close])?);
            rest = &rest[close + 1..];
            continue;
        }
        let end = rest.find(['.', '#', '[']).unwrap_or(rest.len());
        let name = &rest[..end];
        if name.is_empty() {
            return None;
        }
        match kind {
            b'.' => compound.classes.push(name.to_string()),
            b'#' => compound.id = Some(name.to_string()),
            _ => return None,
        }
        rest = &rest[end..];
    }
    Some(compound)
}

fn parse_attr_check(text: &str) -> Option<AttrCheck> {
    let text = text.trim();
    let (name, op, value) = if let Some((name, value)) = text.split_once("~=") {
        (name, AttrOp::Includes, value)
    } else if let Some((name, value)) = text.split_once('=') {
        // `^=`, `$=`, `*=`, `|=` would have left their sigil on the name.
        if name.ends_with(['^', '$', '*', '|']) {
            return None;
        }
        (name, AttrOp::Equals, value)
    } else {
        (text, AttrOp::Exists, "")
    };
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(AttrCheck {
        name: name.to_ascii_lowercase(),
        op,
        value: value.trim().trim_matches(['"', '\'']).to_string(),
    })
}

/// One run of rules: a stylesheet, or the inside of a matching `@media`
/// block — the recursion that makes nested media work for free.
fn parse_rules(parser: &mut Parser<'_, '_>, sheet: &mut Stylesheet, viewport: i32) {
    let mut start = parser.position();
    loop {
        match parser.next() {
            Err(_) => break,
            Ok(Token::CurlyBracketBlock) => {
                let prelude = parser.slice_from(start);
                let prelude = prelude[..prelude.len() - 1].to_string();
                let decls = parser
                    .parse_nested_block(|block| {
                        Ok::<_, cssparser::ParseError<'_, ()>>(parse_declarations(block))
                    })
                    .unwrap_or_default();
                if !decls.is_empty() {
                    for selector in parse_selectors(&prelude) {
                        sheet.push(selector, decls.clone());
                    }
                }
                start = parser.position();
            }
            Ok(Token::AtKeyword(name)) => {
                let media = name.eq_ignore_ascii_case("media");
                // The block must be consumed *now* — an unconsumed block is
                // skipped lazily, after the next rule's prelude would
                // already have swallowed it.
                let condition_start = parser.position();
                loop {
                    match parser.next() {
                        Err(_) => break,
                        Ok(Token::Semicolon) => break,
                        Ok(Token::CurlyBracketBlock) => {
                            let condition = {
                                let s = parser.slice_from(condition_start);
                                s[..s.len() - 1].to_string()
                            };
                            if media && media_matches(&condition, viewport) {
                                let _ = parser.parse_nested_block(|block| {
                                    parse_rules(block, sheet, viewport);
                                    Ok::<_, cssparser::ParseError<'_, ()>>(())
                                });
                            } else {
                                // Every other at-rule, and media this one
                                // medium is not: consumed and dropped.
                                let _ = parser.parse_nested_block(|block| {
                                    while block.next().is_ok() {}
                                    Ok::<_, cssparser::ParseError<'_, ()>>(())
                                });
                            }
                            break;
                        }
                        Ok(_) => {}
                    }
                }
                start = parser.position();
            }
            Ok(_) => {}
        }
    }
}

/// The half of media queries a one-medium renderer can answer honestly:
/// `screen` and `all` are what this is, `print` is not, and width limits
/// compare against the real viewport. A term it cannot evaluate — `not`,
/// `prefers-*`, unknown features — fails its alternative, because applying
/// a dark-scheme block to a light page is worse than skipping it.
fn media_matches(condition: &str, viewport: i32) -> bool {
    let condition = condition.to_ascii_lowercase();
    condition.split(',').any(|alternative| {
        let alternative = alternative.trim();
        if alternative.is_empty() {
            return false;
        }
        alternative.split(" and ").all(|term| {
            let term = term.trim().trim_start_matches("only ").trim();
            match term {
                "screen" | "all" => true,
                _ if term.starts_with("not ") => false,
                _ if term.starts_with('(') => {
                    let inner = term.trim_matches(['(', ')']);
                    let Some((feature, value)) = inner.split_once(':') else {
                        return false;
                    };
                    let px = value
                        .trim()
                        .strip_suffix("px")
                        .and_then(|v| v.trim().parse::<f32>().ok().map(|v| v.round() as i32));
                    match (feature.trim(), px) {
                        ("min-width", Some(px)) => viewport >= px,
                        ("max-width", Some(px)) => viewport <= px,
                        _ => false,
                    }
                }
                _ => false,
            }
        })
    })
}

fn parse_declarations(parser: &mut Parser<'_, '_>) -> Vec<Decl> {
    let mut out = Vec::new();
    while !parser.is_exhausted() {
        let _ = parser.parse_until_after(Delimiter::Semicolon, |one| {
            let name = one.expect_ident()?.to_ascii_lowercase();
            one.expect_colon()?;
            declaration(&name, one, &mut out);
            Ok::<_, cssparser::ParseError<'_, ()>>(())
        });
    }
    out
}

fn declaration(name: &str, value: &mut Parser<'_, '_>, out: &mut Vec<Decl>) {
    match name {
        "color" => {
            if let Some(c) = color(value) {
                out.push(Decl::Color(c));
            }
        }
        "background" | "background-color" => match value.next() {
            Ok(Token::Ident(word))
                if word.eq_ignore_ascii_case("none")
                    || word.eq_ignore_ascii_case("transparent") =>
            {
                out.push(Decl::Background(None));
            }
            Ok(token) => {
                if let Some(c) = color_from(token.clone(), value) {
                    out.push(Decl::Background(Some(c)));
                }
            }
            Err(_) => {}
        },
        "font-size" => {
            if let Some(size) = font_size(value) {
                out.push(Decl::FontSize(size));
            }
        }
        "font-weight" => match value.next() {
            Ok(Token::Ident(word)) => {
                let word = word.to_ascii_lowercase();
                if word == "bold" || word == "bolder" {
                    out.push(Decl::Bold(true));
                } else if word == "normal" || word == "lighter" {
                    out.push(Decl::Bold(false));
                }
            }
            Ok(Token::Number { value: v, .. }) => out.push(Decl::Bold(*v >= 600.0)),
            _ => {}
        },
        "font-style" => {
            if let Ok(Token::Ident(word)) = value.next() {
                let word = word.to_ascii_lowercase();
                if word == "italic" || word == "oblique" {
                    out.push(Decl::Italic(true));
                } else if word == "normal" {
                    out.push(Decl::Italic(false));
                }
            }
        }
        "font-family" => {
            let mut mono = false;
            while let Ok(token) = value.next() {
                let family = match token {
                    Token::Ident(word) => word.as_ref(),
                    Token::QuotedString(word) => word.as_ref(),
                    _ => continue,
                };
                let family = family.to_ascii_lowercase();
                if family.contains("mono")
                    || family.contains("courier")
                    || family.contains("console")
                {
                    mono = true;
                }
            }
            out.push(Decl::Mono(mono));
        }
        "text-align" => {
            if let Ok(Token::Ident(word)) = value.next() {
                match word.to_ascii_lowercase().as_str() {
                    "center" => out.push(Decl::Align(TextAlign::Center)),
                    "left" | "start" | "justify" | "right" | "end" => {
                        // Right-alignment is not rendered; left is the
                        // honest fallback for all of them.
                        out.push(Decl::Align(TextAlign::Left));
                    }
                    _ => {}
                }
            }
        }
        "text-decoration" | "text-decoration-line" => {
            while let Ok(token) = value.next() {
                if let Token::Ident(word) = token {
                    match word.to_ascii_lowercase().as_str() {
                        "underline" => out.push(Decl::Underline(true)),
                        "none" => out.push(Decl::Underline(false)),
                        _ => {}
                    }
                }
            }
        }
        "display" => {
            if let Ok(Token::Ident(word)) = value.next() {
                match word.to_ascii_lowercase().as_str() {
                    "none" => out.push(Decl::Display(CssDisplay::None)),
                    "block" | "flex" | "grid" | "table" | "list-item" | "flow-root" => {
                        out.push(Decl::Display(CssDisplay::Block));
                    }
                    "inline" | "inline-block" | "inline-flex" => {
                        out.push(Decl::Display(CssDisplay::Inline));
                    }
                    _ => {}
                }
            }
        }
        "visibility" => {
            if let Ok(Token::Ident(word)) = value.next()
                && matches!(word.to_ascii_lowercase().as_str(), "hidden" | "collapse")
            {
                out.push(Decl::Display(CssDisplay::None));
            }
        }
        "float" => {
            if let Ok(Token::Ident(word)) = value.next() {
                match word.to_ascii_lowercase().as_str() {
                    "left" => out.push(Decl::Float(Some(FloatSide::Left))),
                    "right" => out.push(Decl::Float(Some(FloatSide::Right))),
                    "none" => out.push(Decl::Float(None)),
                    _ => {}
                }
            }
        }
        "clear" => {
            if let Ok(Token::Ident(word)) = value.next() {
                match word.to_ascii_lowercase().as_str() {
                    "left" => out.push(Decl::Clear(Some(ClearSide::Left))),
                    "right" => out.push(Decl::Clear(Some(ClearSide::Right))),
                    "both" => out.push(Decl::Clear(Some(ClearSide::Both))),
                    "none" => out.push(Decl::Clear(None)),
                    _ => {}
                }
            }
        }
        "width" => match value.next() {
            Ok(Token::Dimension { value: v, unit, .. }) => {
                let v = *v;
                match unit.to_ascii_lowercase().as_str() {
                    "px" => out.push(Decl::Width(Length::Px(v.round() as i32))),
                    "em" => out.push(Decl::Width(Length::Em(v))),
                    "rem" => out.push(Decl::Width(Length::Px((v * 16.0).round() as i32))),
                    _ => {}
                }
            }
            Ok(Token::Percentage { unit_value, .. }) => {
                out.push(Decl::Width(Length::Percent(*unit_value)));
            }
            _ => {}
        },
        // A linearising renderer has no "out of flow" to put these in, and
        // what authors position absolutely is overwhelmingly overlay
        // furniture — dropdown menus, skip links, modals. Reader modes drop
        // them; so does this one. `sticky` stays: it is in-flow content
        // that merely wants to linger.
        "position" => {
            if let Ok(Token::Ident(word)) = value.next()
                && matches!(word.to_ascii_lowercase().as_str(), "absolute" | "fixed")
            {
                out.push(Decl::Display(CssDisplay::None));
            }
        }
        "margin" | "padding" => {
            let mut sides = Vec::new();
            while sides.len() < 4 {
                match length(value) {
                    Some(v) => sides.push(v),
                    None => break,
                }
            }
            let [top, right, bottom, left] = match sides.as_slice() {
                [a] => [*a; 4],
                [v, h] => [*v, *h, *v, *h],
                [t, h, b] => [*t, *h, *b, *h],
                [t, r, b, l] => [*t, *r, *b, *l],
                _ => return,
            };
            let make = if name == "margin" {
                Decl::Margin
            } else {
                Decl::Padding
            };
            out.extend([make(0, top), make(1, right), make(2, bottom), make(3, left)]);
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" => {
            if let Some(v) = length(value) {
                let side = match name.rsplit('-').next() {
                    Some("top") => 0,
                    Some("right") => 1,
                    Some("bottom") => 2,
                    Some("left") => 3,
                    _ => return,
                };
                if name.starts_with("margin") {
                    out.push(Decl::Margin(side, v));
                } else {
                    out.push(Decl::Padding(side, v));
                }
            }
        }
        // Everything else: the tokens are consumed by `parse_until_after`
        // and the page renders without whatever it was.
        _ => {}
    }
}

/// A length this renderer honours: px exactly, em and rem approximately,
/// bare zero, `auto` as zero. Anything else declines the declaration.
fn length(parser: &mut Parser<'_, '_>) -> Option<i32> {
    match parser.next().ok()? {
        Token::Dimension { value, unit, .. } => {
            let value = *value;
            match unit.to_ascii_lowercase().as_str() {
                "px" => Some(value.round() as i32),
                "em" | "rem" => Some((value * 16.0).round() as i32),
                _ => None,
            }
        }
        Token::Number { value, .. } if *value == 0.0 => Some(0),
        Token::Ident(word) if word.eq_ignore_ascii_case("auto") => Some(0),
        Token::Percentage { .. } => None,
        _ => None,
    }
}

fn font_size(parser: &mut Parser<'_, '_>) -> Option<FontSize> {
    match parser.next().ok()? {
        Token::Dimension { value, unit, .. } => {
            let value = *value;
            match unit.to_ascii_lowercase().as_str() {
                "px" | "pt" => Some(FontSize::Px(if unit.eq_ignore_ascii_case("pt") {
                    (value * 4.0 / 3.0).round() as i32
                } else {
                    value.round() as i32
                })),
                "em" => Some(FontSize::Em(value)),
                "rem" => Some(FontSize::Px((value * 16.0).round() as i32)),
                _ => None,
            }
        }
        Token::Percentage { unit_value, .. } => Some(FontSize::Em(*unit_value)),
        Token::Ident(word) => match word.to_ascii_lowercase().as_str() {
            "xx-small" => Some(FontSize::Px(9)),
            "x-small" => Some(FontSize::Px(10)),
            "small" => Some(FontSize::Px(13)),
            "medium" => Some(FontSize::Px(16)),
            "large" => Some(FontSize::Px(19)),
            "x-large" => Some(FontSize::Px(24)),
            "xx-large" => Some(FontSize::Px(32)),
            "smaller" => Some(FontSize::Em(0.83)),
            "larger" => Some(FontSize::Em(1.2)),
            _ => None,
        },
        _ => None,
    }
}

fn color(parser: &mut Parser<'_, '_>) -> Option<Color> {
    let token = parser.next().ok()?.clone();
    color_from(token, parser)
}

fn color_from(token: Token<'_>, parser: &mut Parser<'_, '_>) -> Option<Color> {
    match token {
        Token::Hash(value) | Token::IDHash(value) => hex_color(&value),
        Token::Ident(name) => named_color(&name.to_ascii_lowercase()),
        Token::Function(name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            parser
                .parse_nested_block(|args| {
                    let mut channels = Vec::new();
                    while channels.len() < 3 {
                        match args.next() {
                            Ok(Token::Number { value, .. }) => {
                                channels.push(value.clamp(0.0, 255.0) as u8);
                            }
                            Ok(Token::Percentage { unit_value, .. }) => {
                                channels.push((unit_value * 255.0).clamp(0.0, 255.0) as u8);
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    match channels.as_slice() {
                        [r, g, b] => Ok(Color::rgb(*r, *g, *b)),
                        _ => Err(args.new_custom_error::<_, ()>(())),
                    }
                })
                .ok()
        }
        _ => None,
    }
}

/// The HTML `bgcolor` attribute's idea of a colour: `#ff6600`, bare
/// `ff6600`, or a name. Pre-CSS, and still holding up the orange bar on
/// Hacker News.
pub fn parse_color_attr(value: &str) -> Option<Color> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return hex_color(hex);
    }
    named_color(&value.to_ascii_lowercase()).or_else(|| hex_color(value))
}

fn hex_color(hex: &str) -> Option<Color> {
    let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    match hex.len() {
        3 | 4 => Some(Color::rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
        6 | 8 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
        _ => None,
    }
}

fn named_color(name: &str) -> Option<Color> {
    let (r, g, b) = match name {
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "gainsboro" => (220, 220, 220),
        "whitesmoke" => (245, 245, 245),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "darkred" => (139, 0, 0),
        "orangered" => (255, 69, 0),
        "tomato" => (255, 99, 71),
        "coral" => (255, 127, 80),
        "salmon" => (250, 128, 114),
        "orange" => (255, 165, 0),
        "gold" => (255, 215, 0),
        "yellow" => (255, 255, 0),
        "khaki" => (240, 230, 140),
        "olive" => (128, 128, 0),
        "lime" => (0, 255, 0),
        "green" => (0, 128, 0),
        "darkgreen" => (0, 100, 0),
        "lightgreen" => (144, 238, 144),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "lightblue" => (173, 216, 230),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "darkblue" => (0, 0, 139),
        "royalblue" => (65, 105, 225),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "fuchsia" | "magenta" => (255, 0, 255),
        "pink" => (255, 192, 203),
        "brown" => (165, 42, 42),
        "beige" => (245, 245, 220),
        "ivory" => (255, 255, 240),
        "lavender" => (230, 230, 250),
        "transparent" => return Some(Color::rgba(0, 0, 0, 0)),
        _ => return None,
    };
    Some(Color::rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(dom: &Dom, sheet: &Stylesheet, idx: usize) -> Vec<Decl> {
        sheet
            .declarations_for(dom, idx)
            .into_iter()
            .cloned()
            .collect()
    }

    #[test]
    fn a_rule_finds_its_element() {
        let dom = Dom::parse(r#"<p class="lead big">text</p><p>plain</p>"#);
        let sheet = Stylesheet::parse(".lead { color: #ff0000; }", 800);
        let lead = dom.find("p").unwrap();
        assert_eq!(
            decls(&dom, &sheet, lead),
            vec![Decl::Color(Color::rgb(255, 0, 0))]
        );
        let plain = dom.nodes[dom.nodes[lead].parent.unwrap()].children[1];
        assert!(decls(&dom, &sheet, plain).is_empty());
    }

    #[test]
    fn specificity_outranks_source_order() {
        let dom = Dom::parse(r#"<p id="x" class="c">t</p>"#);
        let sheet = Stylesheet::parse(
            "#x { color: red; } .c { color: blue; } p { color: green; }",
            800,
        );
        let p = dom.find("p").unwrap();
        let all = decls(&dom, &sheet, p);
        // Cascade order: the id's declaration comes last and wins.
        assert_eq!(*all.last().unwrap(), Decl::Color(Color::rgb(255, 0, 0)));
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn descendants_walk_all_the_way_up() {
        let dom = Dom::parse(r#"<div class="outer"><p><b>deep</b></p></div>"#);
        let sheet = Stylesheet::parse(
            ".outer b { font-weight: bold; } .other b { color: red; }",
            800,
        );
        let b = dom.find("b").unwrap();
        assert_eq!(decls(&dom, &sheet, b), vec![Decl::Bold(true)]);
    }

    #[test]
    fn unknown_properties_do_not_take_neighbours_down() {
        let sheet = Stylesheet::parse(
            "p { grid-template-columns: repeat(3, 1fr); color: navy; transition: all .2s; \
             margin: 4px 8px; }",
            800,
        );
        let dom = Dom::parse("<p>t</p>");
        let p = dom.find("p").unwrap();
        let got = decls(&dom, &sheet, p);
        assert!(got.contains(&Decl::Color(Color::rgb(0, 0, 128))));
        assert!(got.contains(&Decl::Margin(0, 4)));
        assert!(got.contains(&Decl::Margin(1, 8)));
        assert_eq!(got.len(), 5, "colour and four margin sides");
    }

    #[test]
    fn pseudo_selectors_reject_only_their_own_rule() {
        let sheet = Stylesheet::parse("a:hover { color: red; } a { color: blue; }", 800);
        let dom = Dom::parse("<a href=x>t</a>");
        let a = dom.find("a").unwrap();
        assert_eq!(
            decls(&dom, &sheet, a),
            vec![Decl::Color(Color::rgb(0, 0, 255))]
        );
    }

    #[test]
    fn inline_style_is_a_declaration_list() {
        let got = Stylesheet::parse_inline("font-size: 2em; display: none");
        assert_eq!(
            got,
            vec![
                Decl::FontSize(FontSize::Em(2.0)),
                Decl::Display(CssDisplay::None)
            ]
        );
    }

    #[test]
    fn a_false_media_query_skips_its_block_whole() {
        let sheet = Stylesheet::parse(
            "@media (max-width: 600px) { p { display: none; } } p { color: black; }",
            800,
        );
        let dom = Dom::parse("<p>t</p>");
        let p = dom.find("p").unwrap();
        assert_eq!(
            decls(&dom, &sheet, p),
            vec![Decl::Color(Color::rgb(0, 0, 0))]
        );
    }

    #[test]
    fn a_true_media_query_admits_its_rules() {
        let sheet = Stylesheet::parse(
            "@media screen and (min-width: 600px) { p { color: navy; } } \
             @media print { p { display: none; } } \
             @media screen { @media (max-width: 2000px) { p { font-weight: bold; } } }",
            800,
        );
        let dom = Dom::parse("<p>t</p>");
        let p = dom.find("p").unwrap();
        let got = decls(&dom, &sheet, p);
        assert!(
            got.contains(&Decl::Color(Color::rgb(0, 0, 128))),
            "screen + width applies"
        );
        assert!(got.contains(&Decl::Bold(true)), "nested media recurses");
        assert!(
            !got.contains(&Decl::Display(CssDisplay::None)),
            "print stays out"
        );
    }

    #[test]
    fn media_alternatives_need_only_one_true() {
        assert!(media_matches("print, screen", 800));
        assert!(media_matches("only screen and (min-width: 100px)", 800));
        assert!(!media_matches("not screen", 800));
        assert!(
            !media_matches("(prefers-color-scheme: dark)", 800),
            "unanswerable fails"
        );
        assert!(!media_matches("screen and (min-width: 1200px)", 800));
    }

    #[test]
    fn overlays_and_invisibility_leave_the_flow() {
        let got = Stylesheet::parse_inline("position: absolute");
        assert_eq!(got, vec![Decl::Display(CssDisplay::None)]);
        let got = Stylesheet::parse_inline("visibility: hidden");
        assert_eq!(got, vec![Decl::Display(CssDisplay::None)]);
        // Sticky is in-flow content that merely lingers; relative is layout
        // this renderer ignores rather than removes.
        assert!(Stylesheet::parse_inline("position: sticky").is_empty());
        assert!(Stylesheet::parse_inline("position: relative").is_empty());
    }

    #[test]
    fn colours_in_every_costume() {
        let sheet = Stylesheet::parse(
            "p { color: #abc; } b { color: #aabbcc; } i { color: rgb(1, 2, 3); } \
             u { background: transparent; }",
            800,
        );
        let dom = Dom::parse("<p><b><i><u>t</u></i></b></p>");
        assert_eq!(
            decls(&dom, &sheet, dom.find("p").unwrap()),
            vec![Decl::Color(Color::rgb(170, 187, 204))]
        );
        assert_eq!(
            decls(&dom, &sheet, dom.find("b").unwrap()).last(),
            Some(&Decl::Color(Color::rgb(170, 187, 204)))
        );
        assert!(
            decls(&dom, &sheet, dom.find("i").unwrap()).contains(&Decl::Color(Color::rgb(1, 2, 3)))
        );
        // `transparent` clears the background rather than painting one.
        assert!(decls(&dom, &sheet, dom.find("u").unwrap()).contains(&Decl::Background(None)));
    }
}
