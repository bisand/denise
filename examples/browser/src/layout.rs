//! From a styled tree to positioned rectangles — the engine Denise
//! deliberately does not have, built out of the pieces it deliberately does.
//!
//! Two kinds of thing come out: block boxes stacked down the page, and
//! *flows* — inline formatting contexts where styled runs of text have been
//! broken into lines. The line breaker is the part worth reading: Denise's
//! own `wrap` takes one style for a whole paragraph, and a paragraph of HTML
//! is many. So words are measured one at a time with [`TextEngine::measure_line`],
//! lines are committed when a word will not fit, and each line's baseline is
//! the tallest ascent on it — which is what [`TextEngine::draw_line`] taking
//! a *baseline* origin was waiting for.
//!
//! Everything here is physical pixels. Styles arrive in logical pixels and
//! are multiplied once, on entry, the same "scale once at the edge" rule the
//! gallery follows; measuring and painting must agree exactly, so they use
//! the same numbers.
//!
//! What is not here, on purpose: positioning. The content flows in
//! document order, which trades fidelity for a page that is always
//! readable — the right trade for an example that fits in one file. Two
//! exceptions fidelity won: tables get real columns, because a rank beside
//! a headline is what a table *means*; and floats stand aside as bands the
//! line breaker keeps out of, because a picture with prose running past it
//! is how articles have been typeset since long before CSS.

use std::collections::HashMap;

use denise::{Color, Point, Rect, Size};
use denise_text::{FontId, TextEngine, TextStyle};

use crate::css::{ClearSide, FloatSide, Length};
use crate::dom::{Dom, NodeData};
use crate::forms::{FormsModel, RenderControl};
use crate::style::{Cascade, ComputedStyle, Display, TextAlign};

/// The faces a page speaks in. Any face that could not be loaded falls back
/// to `regular` — degraded, not broken.
#[derive(Clone, Copy)]
pub struct Fonts {
    pub regular: FontId,
    pub bold: FontId,
    pub italic: FontId,
    pub bold_italic: FontId,
    pub mono: FontId,
}

impl Fonts {
    /// One face everywhere: what a page looks like before better fonts are
    /// found, and what every miss falls back to.
    pub fn all(font: FontId) -> Self {
        Self {
            regular: font,
            bold: font,
            italic: font,
            bold_italic: font,
            mono: font,
        }
    }

    fn pick(&self, style: &ComputedStyle) -> FontId {
        if style.monospace {
            self.mono
        } else if style.bold && style.italic {
            self.bold_italic
        } else if style.bold {
            self.bold
        } else if style.italic {
            self.italic
        } else {
            self.regular
        }
    }
}

/// A run of text in one voice: one font, one size, one colour.
pub struct StyledRun {
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
    pub underline: bool,
    pub link: Option<usize>,
}

/// A slice of one run placed on one line. `width` is exact — re-measured at
/// commit so the underline and the hit rectangle match the ink.
pub struct Fragment {
    pub run: usize,
    pub range: core::ops::Range<usize>,
    pub x: i32,
    pub width: i32,
}

pub struct Line {
    /// Top of the line, relative to the flow's origin.
    pub y: i32,
    /// Baseline offset from the line's top: the tallest ascent on it.
    pub baseline: i32,
    pub height: i32,
    pub fragments: Vec<Fragment>,
}

/// An inline formatting context, fully broken into lines.
pub struct FlowLayout {
    pub runs: Vec<StyledRun>,
    pub lines: Vec<Line>,
    pub height: i32,
}

/// One thing to put on the page. The list is already in paint order:
/// backgrounds precede what sits on them.
pub enum Leaf {
    /// A block's background colour, the one raw-colour rectangle HTML needs.
    Background(Color),
    Flow(FlowLayout),
    /// A list marker: `•`, or `3.` in an `<ol>`.
    Bullet {
        text: String,
        style: TextStyle,
        color: Color,
    },
    /// `<hr>`.
    Rule,
    /// An `<img>`, waiting for its bytes. `dom` names the element so the
    /// arriving pixels find their node; `sized` says the box is final —
    /// arrival will fill it, not move it.
    Image {
        dom: usize,
        src: String,
        sized: bool,
    },
    /// A form control; what to build there is in the forms model, keyed by
    /// the same element.
    Control {
        dom: usize,
    },
}

pub struct Placed {
    /// Relative to the page's top-left, physical pixels.
    pub rect: Rect,
    pub leaf: Leaf,
}

pub struct PageLayout {
    pub leaves: Vec<Placed>,
    /// Total content height, the number the scroll range comes from.
    pub height: i32,
    /// Element ids and where they landed, for `#fragment` navigation.
    /// Physical pixels from the page top, block-level elements only.
    pub anchors: Vec<(String, i32)>,
}

/// Lays out a whole document into `width` physical pixels.
///
/// `natural` carries the pixel sizes of images that have already arrived;
/// an image absent from it and without `width`/`height` attributes gets the
/// placeholder box, and its arrival triggers the one relayout that fixes it.
//
// Eight arguments because a page *is* eight things; a struct would rename
// the problem without shrinking it.
#[allow(clippy::too_many_arguments)]
pub fn layout_page(
    dom: &Dom,
    cascade: &Cascade,
    width: i32,
    scale: f32,
    fonts: &Fonts,
    natural: &HashMap<usize, Size>,
    forms: &FormsModel,
    engine: &mut TextEngine,
) -> PageLayout {
    let mut l = Layouter {
        dom,
        cascade,
        scale,
        fonts,
        natural,
        forms,
        engine,
        leaves: Vec::new(),
        anchors: Vec::new(),
    };
    let body = dom.find("body").unwrap_or(0);
    let style = &cascade.styles[body];
    let margin = l.px4(style.margin);
    let x = margin[3];
    let y = margin[0];
    let inner = (width - margin[3] - margin[1]).max(1);
    let height = l.block(body, x, y, inner, None, &[]);
    PageLayout {
        leaves: l.leaves,
        height: y + height + margin[2],
        anchors: l.anchors,
    }
}

/// One float's shadow over the flow: within `top..bottom`, lines and blocks
/// keep out of its side. Absolute coordinates, physical pixels.
#[derive(Clone, Copy)]
struct Band {
    side: FloatSide,
    x: i32,
    width: i32,
    top: i32,
    bottom: i32,
}

/// The horizontal window left between the bands overlapping `y0..y1`,
/// within a content box starting at `cx`, `cw` wide.
fn band_window(bands: &[Band], cx: i32, cw: i32, y0: i32, y1: i32, gap: i32) -> (i32, i32) {
    let mut left = cx;
    let mut right = cx + cw;
    for band in bands {
        if band.bottom <= y0 || band.top >= y1 {
            continue;
        }
        match band.side {
            FloatSide::Left => left = left.max(band.x + band.width + gap),
            FloatSide::Right => right = right.min(band.x - gap),
        }
    }
    (left, right.max(left + 1))
}

/// The first y at which the named sides are all past, from `y` down.
fn clear_bottom(bands: &[Band], side: ClearSide, y: i32) -> i32 {
    bands
        .iter()
        .filter(|b| match side {
            ClearSide::Left => b.side == FloatSide::Left,
            ClearSide::Right => b.side == FloatSide::Right,
            ClearSide::Both => true,
        })
        .map(|b| b.bottom)
        .fold(y, i32::max)
}

struct Layouter<'a> {
    dom: &'a Dom,
    cascade: &'a Cascade,
    scale: f32,
    fonts: &'a Fonts,
    natural: &'a HashMap<usize, Size>,
    forms: &'a FormsModel,
    engine: &'a mut TextEngine,
    leaves: Vec<Placed>,
    anchors: Vec<(String, i32)>,
}

/// A box heading into a line: an image or a form control, already sized,
/// in physical pixels.
struct ObjSpec {
    dom: usize,
    width: i32,
    height: i32,
    kind: ObjKind,
}

enum ObjKind {
    Image { src: String, sized: bool },
    Control,
}

/// What a table cell says about its column's width.
enum CellWidth {
    Fixed(i32),
    Flexible,
    DontCare,
}

/// An object the breaker placed, relative to its flow's origin.
struct PlacedObject {
    dom: usize,
    kind: ObjKind,
    rect: Rect,
}

/// What one inline token is: a voice speaking, a `<br>`, or a box —
/// pixels or a widget — flowing along with the words.
enum InlineToken {
    Run(StyledRun),
    Break(TextStyle),
    Object(ObjSpec),
}

impl Layouter<'_> {
    fn px(&self, logical: i32) -> i32 {
        // Examples are std binaries; the core crates' float rules do not
        // reach here.
        (logical as f32 * self.scale).round() as i32
    }

    fn px4(&self, logical: [i32; 4]) -> [i32; 4] {
        logical.map(|v| self.px(v))
    }

    fn text_style(&self, style: &ComputedStyle) -> TextStyle {
        TextStyle {
            font: self.fonts.pick(style),
            size_px: ((f32::from(style.font_size) * self.scale).round() as u16).max(1),
        }
    }

    /// What box an `<img>` claims: attributes first, the decoded size
    /// second, a placeholder last — the box an image reserves before it
    /// arrives is what keeps the text from jumping when it does. `None`
    /// for sources the decoder will never manage (SVG, `data:`): a
    /// permanently blank reservation is worse than absence.
    fn image_spec(&self, idx: usize, style: &ComputedStyle) -> Option<ObjSpec> {
        let src = self.dom.attr(idx, "src")?;
        let path = src.split(['?', '#']).next().unwrap_or(src);
        if path.to_ascii_lowercase().ends_with(".svg") || src.starts_with("data:") {
            return None;
        }
        let attr = |name: &str| {
            self.dom
                .attr(idx, name)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .filter(|v| *v > 0)
        };
        let natural = self.natural.get(&idx).copied();
        let (w_attr, h_attr) = (attr("width"), attr("height"));
        // An author width (CSS) outranks the width attribute.
        let css_w = style
            .width
            .map(|w| match w {
                Length::Px(px) => px,
                Length::Em(em) => (f32::from(style.font_size) * em).round() as i32,
                Length::Percent(_) => w_attr.unwrap_or(300),
            })
            .filter(|w| *w > 0);
        let w_attr = css_w.or(w_attr);
        let sized = (w_attr.is_some() && h_attr.is_some()) || natural.is_some();
        let (w, h) = match (w_attr, h_attr, natural) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some(n)) => (w, (w * n.height as i32) / (n.width as i32).max(1)),
            (None, Some(h), Some(n)) => ((h * n.width as i32) / (n.height as i32).max(1), h),
            (Some(w), None, None) => (w, w * 3 / 4),
            (None, Some(h), None) => (h * 4 / 3, h),
            (None, None, Some(n)) => (n.width as i32, n.height as i32),
            (None, None, None) => (300, 150),
        };
        Some(ObjSpec {
            dom: idx,
            width: self.px(w).max(1),
            height: self.px(h).max(1),
            kind: ObjKind::Image {
                src: src.to_string(),
                sized,
            },
        })
    }

    /// Resolves an author width against the containing block, physical px.
    fn resolve_width(&self, width: Length, container: i32, style: &ComputedStyle) -> i32 {
        match width {
            Length::Px(px) => self.px(px),
            Length::Em(em) => self.px((f32::from(style.font_size) * em).round() as i32),
            Length::Percent(p) => (container as f32 * p) as i32,
        }
    }

    /// Lays out `idx` as a block whose border box starts at `(x, y)` and is
    /// `width` wide, keeping clear of `bands` — the floats of enclosing
    /// blocks. Returns the border-box height. Margins are the caller's
    /// business; a marker is the parent list's.
    fn block(
        &mut self,
        idx: usize,
        x: i32,
        y: i32,
        width: i32,
        marker: Option<String>,
        bands: &[Band],
    ) -> i32 {
        let style = self.cascade.styles[idx].clone();

        if self.dom.tag(idx) == Some("hr") {
            let height = self.px(1).max(1);
            self.leaves.push(Placed {
                rect: Rect::new(x, y, width, height),
                leaf: Leaf::Rule,
            });
            return height;
        }

        // A floated (or otherwise blockified) image is its own block: one
        // leaf, sized like any other picture, no children to walk.
        if self.dom.tag(idx) == Some("img") {
            if let Some(spec) = self.image_spec(idx, &style) {
                let w = spec.width.min(width);
                let h = if spec.width > width {
                    (spec.height * width) / spec.width.max(1)
                } else {
                    spec.height
                };
                let ObjKind::Image { src, sized } = spec.kind else {
                    unreachable!("image_spec makes images");
                };
                self.leaves.push(Placed {
                    rect: Rect::new(x, y, w, h),
                    leaf: Leaf::Image {
                        dom: idx,
                        src,
                        sized,
                    },
                });
                return h;
            }
            return 0;
        }

        // The background's height is not known until the children are; a
        // placeholder keeps its place in paint order and is patched below.
        let background = style.background.map(|color| {
            self.leaves.push(Placed {
                rect: Rect::ZERO,
                leaf: Leaf::Background(color),
            });
            self.leaves.len() - 1
        });

        let padding = self.px4(style.padding);
        let cx = x + padding[3];
        let cw = (width - padding[3] - padding[1]).max(1);
        let mut cy = y + padding[0];

        if let Some(text) = marker {
            let ts = self.text_style(&style);
            let w = self.engine.measure_line(ts, &text);
            let gap = self.px(6);
            self.leaves.push(Placed {
                rect: Rect::new(cx - w - gap, cy, w, self.engine.line_height(ts)),
                leaf: Leaf::Bullet {
                    text,
                    style: ts,
                    color: style.color,
                },
            });
        }

        // A record for the anchor table: `#fragment` scrolls here.
        if let Some(id) = self.dom.attr(idx, "id") {
            self.anchors.push((id.to_string(), y));
        }

        // A table with real rows gets real columns.
        if self.dom.tag(idx) == Some("table") {
            let rows = self.table_rows(idx);
            if !rows.is_empty() {
                cy += self.table(idx, &rows, cx, cy, cw);
                let height = (cy + padding[2]) - y;
                if let Some(leaf) = background {
                    self.leaves[leaf].rect = Rect::new(x, y, width, height);
                }
                return height;
            }
        }

        // The floats of enclosing blocks, plus this block's own as they are
        // placed — siblings see them, descendants' flows run past them.
        let mut local: Vec<Band> = bands.to_vec();
        let inherited = local.len();
        let gap = self.px(8);

        // Children: inline-level runs gather until a block interrupts them.
        let mut inline: Vec<InlineToken> = Vec::new();
        let mut ends_with_space = true; // swallows leading whitespace
        let mut pending_margin = 0;
        let mut ordinal = 0;

        let children = self.dom.nodes[idx].children.clone();
        for child in children {
            let child_style = &self.cascade.styles[child];
            match child_style.display {
                Display::None => {}
                Display::Inline => {
                    self.collect_inline(child, &mut inline, &mut ends_with_space, style.pre);
                }
                Display::Block | Display::ListItem if child_style.float.is_some() => {
                    // A float: out of the flow, against a side, remembered
                    // as a band everything after it must run around.
                    self.flush_flow(
                        &mut inline,
                        cx,
                        &mut cy,
                        cw,
                        &style,
                        &mut pending_margin,
                        &local,
                    );
                    let side = child_style.float.expect("matched");
                    let margin = self.px4(child_style.margin);
                    // An author width first; a floated image's own box
                    // second; two fifths of the container as the shrug.
                    let fw = child_style
                        .width
                        .map(|w| self.resolve_width(w, cw, child_style))
                        .or_else(|| {
                            (self.dom.tag(child) == Some("img"))
                                .then(|| self.image_spec(child, child_style))
                                .flatten()
                                .map(|spec| spec.width)
                        })
                        .unwrap_or(cw * 2 / 5)
                        .clamp(16, (cw - gap).max(16));
                    // Below every earlier float on the same side.
                    let mut fy = cy + margin[0];
                    for band in &local {
                        if band.side == side {
                            fy = fy.max(band.bottom);
                        }
                    }
                    let fx = match side {
                        FloatSide::Left => cx + margin[3],
                        FloatSide::Right => cx + cw - fw - margin[1],
                    };
                    let fh = self.block(child, fx, fy, fw, None, &local);
                    local.push(Band {
                        side,
                        x: fx,
                        width: fw,
                        top: fy,
                        bottom: fy + fh + margin[2].max(gap),
                    });
                }
                Display::Block | Display::ListItem => {
                    self.flush_flow(
                        &mut inline,
                        cx,
                        &mut cy,
                        cw,
                        &style,
                        &mut pending_margin,
                        &local,
                    );
                    ends_with_space = true;

                    let child_marker = if child_style.display == Display::ListItem {
                        ordinal += 1;
                        Some(if self.dom.tag(idx) == Some("ol") {
                            format!("{ordinal}.")
                        } else {
                            "\u{2022}".to_string()
                        })
                    } else {
                        None
                    };
                    let margin = self.px4(child_style.margin);
                    cy += pending_margin.max(margin[0]);
                    if let Some(clear) = child_style.clear {
                        cy = clear_bottom(&local, clear, cy);
                    }
                    // The window the floats leave at this height; a block
                    // that starts beside a float stays narrowed to its end,
                    // which early browsers did too.
                    let (wl, wr) = band_window(&local, cx, cw, cy, cy + 1, gap);
                    let bx = wl + margin[3];
                    let bw = (wr - wl - margin[3] - margin[1]).max(1);
                    let h = self.block(child, bx, cy, bw, child_marker, &local);
                    cy += h;
                    pending_margin = margin[2];
                }
            }
        }
        self.flush_flow(
            &mut inline,
            cx,
            &mut cy,
            cw,
            &style,
            &mut pending_margin,
            &local,
        );
        cy += pending_margin;

        // A block contains its own floats; those it merely inherited are
        // its parent's problem.
        for band in &local[inherited..] {
            cy = cy.max(band.bottom);
        }

        let height = (cy + padding[2]) - y;
        if let Some(leaf) = background {
            self.leaves[leaf].rect = Rect::new(x, y, width, height);
        }
        height
    }

    /// The rows of a table, each a list of its cells, with everything
    /// `display: none` already gone. Empty when the table has no usable
    /// rows — or too many columns to be worth eleven-pixel ones, in which
    /// case the caller linearises the way it always did.
    fn table_rows(&self, table: usize) -> Vec<Vec<usize>> {
        let mut rows: Vec<Vec<usize>> = Vec::new();
        // Rows sit either directly under the table or under its sections;
        // scan both without duplicating.
        let mut holders: Vec<usize> = vec![table];
        holders.extend(
            self.dom.nodes[table]
                .children
                .iter()
                .copied()
                .filter(|&c| matches!(self.dom.tag(c), Some("thead" | "tbody" | "tfoot"))),
        );
        for holder in holders {
            for &row in &self.dom.nodes[holder].children {
                if self.dom.tag(row) != Some("tr")
                    || self.cascade.styles[row].display == Display::None
                {
                    continue;
                }
                let cells: Vec<usize> = self.dom.nodes[row]
                    .children
                    .iter()
                    .copied()
                    .filter(|&c| {
                        matches!(self.dom.tag(c), Some("td" | "th"))
                            && self.cascade.styles[c].display != Display::None
                    })
                    .collect();
                if !cells.is_empty() {
                    rows.push(cells);
                }
            }
        }
        let columns = rows.iter().map(|r| self.row_span(r)).max().unwrap_or(0);
        if !(2..=8).contains(&columns) {
            return Vec::new();
        }
        rows
    }

    fn colspan(&self, cell: usize) -> usize {
        self.dom
            .attr(cell, "colspan")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 8)
    }

    fn row_span(&self, cells: &[usize]) -> usize {
        cells.iter().map(|&c| self.colspan(c)).sum()
    }

    /// Table layout, the simple honest kind: columns share the width
    /// equally unless a cell asks for one — a `width` attribute in px or
    /// percent, or a cell that is nothing but a sized image, the spacer
    /// idiom the old web indents comment threads with. Row height is the
    /// tallest cell's. No rowspan; a cell spans columns and that is all.
    fn table(&mut self, _table: usize, rows: &[Vec<usize>], x: i32, y: i32, width: i32) -> i32 {
        let spacing = self.px(4);
        let columns = rows.iter().map(|r| self.row_span(r)).max().unwrap_or(1);
        let avail = (width - spacing * (columns as i32 - 1)).max(columns as i32);

        // What the cells ask for, per column. A cell with prose makes its
        // whole column flexible — a rank column is only narrow because
        // *nothing* in it ever wants room; one long cell anywhere means
        // the "More" link at the bottom must not pin the story column.
        let mut requested: Vec<Option<i32>> = vec![None; columns];
        let mut flexible: Vec<bool> = vec![false; columns];
        for row in rows {
            let mut col = 0usize;
            for &cell in row {
                let span = self.colspan(cell).min(columns - col.min(columns - 1));
                if span == 1 && col < columns {
                    match self.cell_width_request(cell, avail) {
                        CellWidth::Fixed(w) => {
                            let slot = &mut requested[col];
                            *slot = Some(slot.map_or(w, |prev: i32| prev.max(w)));
                        }
                        CellWidth::Flexible => flexible[col] = true,
                        CellWidth::DontCare => {}
                    }
                }
                col += span;
            }
        }
        for (req, flex) in requested.iter_mut().zip(&flexible) {
            if *flex {
                *req = None;
            }
        }
        // Columns nobody spoke for — every cell empty — collapse to a
        // sliver rather than sharing the prose's width; if *no* column is
        // flexible, they inherit the flexibility instead.
        let sliver = self.px(16);
        if !flexible.iter().any(|&f| f) {
            flexible.fill(true);
        } else {
            for (req, flex) in requested.iter_mut().zip(&flexible) {
                if req.is_none() && !*flex {
                    *req = Some(sliver);
                }
            }
        }
        // Requests are honoured while they leave the prose room to breathe;
        // a table that asks for everything gets equal shares instead.
        let asked: i32 = requested.iter().flatten().copied().sum();
        let asked_cols = requested.iter().flatten().count();
        let mut widths: Vec<i32> = vec![0; columns];
        if asked_cols == columns || asked * 4 > avail * 3 {
            let each = avail / columns as i32;
            widths.fill(each);
        } else {
            let remainder = avail - asked;
            let each = remainder / (columns - asked_cols) as i32;
            for (w, req) in widths.iter_mut().zip(&requested) {
                *w = req.unwrap_or(each).max(8);
            }
        }

        let x_of = |widths: &[i32], col: usize| {
            x + widths[..col].iter().sum::<i32>() + spacing * col as i32
        };

        let mut cy = y;
        for row in rows {
            let mut col = 0usize;
            let mut row_height = 0;
            for &cell in row {
                let span = self
                    .colspan(cell)
                    .min(columns - col.min(columns - 1))
                    .max(1);
                let cell_w =
                    widths[col..col + span].iter().sum::<i32>() + spacing * (span as i32 - 1);
                let h = self.block(cell, x_of(&widths, col), cy, cell_w.max(8), None, &[]);
                row_height = row_height.max(h);
                col += span;
            }
            cy += row_height + spacing;
        }
        (cy - y - spacing).max(0)
    }

    /// A cell's own idea of its width: the `width` attribute in px or
    /// percent; a lone sized image inside a cell with no text, which is how
    /// spacer indentation was always written; a snippet of text short
    /// enough to be a rank or a vote count — measured, so a "17." column is
    /// as wide as "17.". Prose answers Flexible, an empty cell has no
    /// opinion at all.
    fn cell_width_request(&mut self, cell: usize, avail: i32) -> CellWidth {
        if let Some(value) = self.dom.attr(cell, "width") {
            let value = value.trim();
            if let Some(percent) = value.strip_suffix('%') {
                if let Ok(p) = percent.trim().parse::<f32>() {
                    return CellWidth::Fixed(((avail as f32 * p / 100.0) as i32).max(8));
                }
            } else if let Ok(px) = value.parse::<i32>() {
                return CellWidth::Fixed(self.px(px).max(8));
            }
        }
        let text = self.dom.text_content(cell);
        let text = text.trim();
        if text.is_empty() {
            let mut widest: Option<i32> = None;
            self.widest_sized_image(cell, &mut widest);
            return match widest {
                Some(w) => CellWidth::Fixed(self.px(w).max(8)),
                None => CellWidth::DontCare,
            };
        }
        if text.chars().count() <= 12 && !text.contains('\n') {
            let style = self.cascade.styles[cell].clone();
            let ts = self.text_style(&style);
            let measured = self.engine.measure_line(ts, text);
            return CellWidth::Fixed(measured + self.px(8));
        }
        CellWidth::Flexible
    }

    fn widest_sized_image(&self, idx: usize, widest: &mut Option<i32>) {
        if self.dom.tag(idx) == Some("img")
            && let Some(w) = self
                .dom
                .attr(idx, "width")
                .and_then(|v| v.trim().parse::<i32>().ok())
        {
            *widest = Some(widest.map_or(w, |prev| prev.max(w)));
        }
        for &child in &self.dom.nodes[idx].children {
            self.widest_sized_image(child, widest);
        }
    }

    /// Breaks the gathered inline tokens into lines and places the flow at
    /// the current position, spending any margin owed by the block above it.
    /// A group with nothing worth a line spends nothing — the margin stays
    /// pending for whatever block comes next.
    #[allow(clippy::too_many_arguments)]
    fn flush_flow(
        &mut self,
        tokens: &mut Vec<InlineToken>,
        x: i32,
        cy: &mut i32,
        width: i32,
        container: &ComputedStyle,
        pending_margin: &mut i32,
        bands: &[Band],
    ) {
        let tokens = core::mem::take(tokens);
        let has_ink = tokens.iter().any(|t| match t {
            InlineToken::Run(run) => !run.text.trim().is_empty(),
            InlineToken::Break(_) | InlineToken::Object(_) => true,
        });
        if !has_ink {
            return;
        }
        *cy += core::mem::take(pending_margin);
        let (flow, objects) = self.break_lines(tokens, width, container, Point::new(x, *cy), bands);
        let height = flow.height;
        self.leaves.push(Placed {
            rect: Rect::new(x, *cy, width, height),
            leaf: Leaf::Flow(flow),
        });
        // The objects the breaker placed become leaves of their own, on top
        // of the flow — separate nodes, because an image's pixels arrive
        // later and a control is a whole widget of its own.
        for object in objects {
            let rect = Rect::new(
                x + object.rect.x,
                *cy + object.rect.y,
                object.rect.width,
                object.rect.height,
            );
            let leaf = match object.kind {
                ObjKind::Image { src, sized } => Leaf::Image {
                    dom: object.dom,
                    src,
                    sized,
                },
                ObjKind::Control => Leaf::Control { dom: object.dom },
            };
            self.leaves.push(Placed { rect, leaf });
        }
        *cy += height;
    }

    /// Walks an inline subtree, collecting runs with whitespace already
    /// collapsed — across run boundaries too, which is why the flag threads
    /// through: `<b>bold</b> next` keeps its space, a newline between tags
    /// becomes one, not two.
    fn collect_inline(
        &mut self,
        idx: usize,
        out: &mut Vec<InlineToken>,
        ends_with_space: &mut bool,
        pre: bool,
    ) {
        let style = &self.cascade.styles[idx];
        match &self.dom.nodes[idx].data {
            NodeData::Comment => {}
            NodeData::Text(text) => {
                let collapsed = if pre {
                    text.clone()
                } else {
                    collapse_whitespace(text, ends_with_space)
                };
                if !collapsed.is_empty() {
                    out.push(InlineToken::Run(StyledRun {
                        text: collapsed,
                        style: self.text_style(style),
                        color: style.color,
                        underline: style.underline,
                        link: style.link,
                    }));
                }
            }
            NodeData::Element { .. } if self.dom.tag(idx) == Some("br") => {
                out.push(InlineToken::Break(self.text_style(style)));
                *ends_with_space = true;
            }
            NodeData::Element { .. } if self.dom.tag(idx) == Some("img") => {
                let Some(spec) = self.image_spec(idx, style) else {
                    return;
                };
                out.push(InlineToken::Object(spec));
                *ends_with_space = false;
            }
            NodeData::Element { .. }
                if matches!(
                    self.dom.tag(idx),
                    Some("input" | "select" | "textarea" | "button")
                ) =>
            {
                // Only controls the forms pass decided to draw; the rest —
                // hidden inputs, radios folded into their group's first,
                // controls outside any form — take no space.
                let Some(control) = self.forms.render.get(&idx) else {
                    return;
                };
                let ts = self.text_style(style);
                let (w, h) = match control {
                    RenderControl::Text { chars, .. } => (chars.map_or(200, |c| c * 9 + 24), 34),
                    RenderControl::Checkbox { .. } => (24, 24),
                    RenderControl::Radio { labels, .. } => (220, 30 * (labels.len().max(1) as i32)),
                    RenderControl::Select { .. } => (200, 34),
                    RenderControl::Button { label, .. } => {
                        let text = self.engine.measure_line(ts, label);
                        // Measured physically already; convert back so the
                        // one px() below scales it like everything else.
                        ((text as f32 / self.scale).round() as i32 + 28, 34)
                    }
                };
                out.push(InlineToken::Object(ObjSpec {
                    dom: idx,
                    width: self.px(w).max(1),
                    height: self.px(h).max(1),
                    kind: ObjKind::Control,
                }));
                *ends_with_space = false;
            }
            _ => {
                // Inline elements contribute nothing themselves — their
                // children do. A block that slipped inside an inline flows
                // along too; the parser has already repaired the cases that
                // matter, and readable beats faithful for the rest.
                let children = self.dom.nodes[idx].children.clone();
                for child in children {
                    if self.cascade.styles[child].display != Display::None {
                        self.collect_inline(child, out, ends_with_space, pre);
                    }
                }
            }
        }
    }

    /// The line breaker. Greedy: words go on the line until one will not
    /// fit; that word starts the next line. A word wider than the whole line
    /// stands alone and overflows — no mid-word breaking.
    //
    // `commit!` resets the pen for whatever follows it; after the final
    // commit nothing does, which the dataflow lint reads as a dead store.
    #[allow(unused_assignments)]
    fn break_lines(
        &mut self,
        tokens: Vec<InlineToken>,
        max_width: i32,
        container: &ComputedStyle,
        origin: Point,
        bands: &[Band],
    ) -> (FlowLayout, Vec<PlacedObject>) {
        let container_style = self.text_style(container);
        let mut runs: Vec<StyledRun> = Vec::new();
        let mut lines: Vec<Line> = Vec::new();
        let mut placed: Vec<PlacedObject> = Vec::new();

        // The line being filled.
        let mut frags: Vec<Fragment> = Vec::new();
        let mut objs: Vec<(ObjSpec, i32)> = Vec::new();
        let mut cursor = 0i32;
        let mut y = 0i32;
        // A space seen since the last word, and the style it was spoken in.
        let mut space: Option<TextStyle> = None;

        // Floats narrow individual lines, not the flow: each line asks what
        // window the bands leave at its height, and a line pinched to
        // nothing drops below the pinching float instead.
        let gap = self.px(8);
        let est = self.engine.line_height(container_style).max(1);
        let min_line = self.px(60).min(max_width);
        let mut line_off = 0i32;
        let mut line_w = max_width;
        macro_rules! rewindow {
            () => {{
                loop {
                    let (l, r) = band_window(
                        bands,
                        origin.x,
                        max_width,
                        origin.y + y,
                        origin.y + y + est,
                        gap,
                    );
                    line_off = l - origin.x;
                    line_w = r - l;
                    if line_w >= min_line {
                        break;
                    }
                    let below = bands
                        .iter()
                        .filter(|b| b.bottom > origin.y + y)
                        .map(|b| b.bottom - origin.y)
                        .min();
                    match below {
                        Some(next) if next > y => y = next,
                        _ => break,
                    }
                }
            }};
        }
        rewindow!();

        macro_rules! commit {
            ($fallback:expr) => {{
                // The line is as tall as its tallest ascent plus its
                // deepest descent — an image is all ascent, text keeps its
                // descenders inside the line box either way.
                let mut baseline = 0i32;
                let mut depth = 0i32;
                for frag in &mut frags {
                    let style = runs[frag.run].style;
                    // Exact at commit: the fill used summed word widths,
                    // the underline and hit rectangle deserve the real one.
                    frag.width = self
                        .engine
                        .measure_line(style, &runs[frag.run].text[frag.range.clone()]);
                    let m = self.engine.metrics(style);
                    baseline = baseline.max(m.ascent);
                    depth = depth.max(m.descent + m.line_gap);
                }
                for (spec, _) in &objs {
                    baseline = baseline.max(spec.height);
                }
                if frags.is_empty() && objs.is_empty() {
                    let m = self.engine.metrics($fallback);
                    baseline = m.ascent;
                    depth = m.descent + m.line_gap;
                }
                let height = baseline + depth;
                if container.text_align == TextAlign::Center {
                    let frag_end = frags.last().map_or(0, |f| f.x + f.width);
                    let obj_end = objs.last().map_or(0, |(s, x)| x + s.width);
                    let shift = ((line_w - frag_end.max(obj_end)) / 2).max(0);
                    for frag in &mut frags {
                        frag.x += shift;
                    }
                    for (_, x) in &mut objs {
                        *x += shift;
                    }
                }
                // The line's window offset lands here, once, on everything.
                for frag in &mut frags {
                    frag.x += line_off;
                }
                for (spec, x) in objs.drain(..) {
                    placed.push(PlacedObject {
                        dom: spec.dom,
                        kind: spec.kind,
                        rect: Rect::new(
                            x + line_off,
                            y + baseline - spec.height,
                            spec.width,
                            spec.height,
                        ),
                    });
                }
                lines.push(Line {
                    y,
                    baseline,
                    height,
                    fragments: core::mem::take(&mut frags),
                });
                y += height;
                cursor = 0;
                space = None;
                rewindow!();
            }};
        }

        for token in tokens {
            match token {
                InlineToken::Break(style) => commit!(style),
                InlineToken::Object(mut spec) => {
                    // An empty line squeezed by a float that cannot hold
                    // this box drops below the floats first.
                    if frags.is_empty()
                        && objs.is_empty()
                        && spec.width > line_w
                        && line_w < max_width
                        && let Some(below) = bands
                            .iter()
                            .filter(|b| b.bottom > origin.y + y)
                            .map(|b| b.bottom - origin.y)
                            .max()
                    {
                        y = y.max(below);
                        rewindow!();
                    }
                    // Wider than the line entirely: scale down, keeping shape.
                    if spec.width > line_w {
                        spec.height = (spec.height * line_w) / spec.width.max(1);
                        spec.width = line_w;
                    }
                    let line_empty = frags.is_empty() && objs.is_empty();
                    let space_width = match (space, line_empty) {
                        (Some(s), false) => self.engine.measure_line(s, " "),
                        _ => 0,
                    };
                    if !line_empty && cursor + space_width + spec.width > line_w {
                        commit!(container_style);
                        objs.push((spec, 0));
                        cursor = objs.last().expect("just pushed").0.width;
                    } else {
                        let x = cursor + space_width;
                        cursor = x + spec.width;
                        objs.push((spec, x));
                    }
                    space = None;
                }
                InlineToken::Run(run) => {
                    runs.push(run);
                    let ri = runs.len() - 1;
                    let style = runs[ri].style;
                    if container.pre {
                        // Verbatim: one source line per screen line, no
                        // wrapping. Horizontal overflow is clipped.
                        let text = core::mem::take(&mut runs[ri].text);
                        for (n, part) in text.split('\n').enumerate() {
                            if n > 0 {
                                commit!(style);
                            }
                            if !part.is_empty() {
                                let start = runs[ri].text.len();
                                runs[ri].text.push_str(part);
                                let width = self.engine.measure_line(style, part);
                                frags.push(Fragment {
                                    run: ri,
                                    range: start..runs[ri].text.len(),
                                    x: cursor,
                                    width,
                                });
                                cursor += width;
                            }
                        }
                        continue;
                    }

                    let mut pos = 0;
                    let text = runs[ri].text.clone();
                    while pos < text.len() {
                        if text[pos..].starts_with(' ') {
                            space = Some(style);
                            pos += 1;
                            continue;
                        }
                        let end = text[pos..]
                            .find(' ')
                            .map_or(text.len(), |offset| pos + offset);
                        let word = &text[pos..end];
                        let word_width = self.engine.measure_line(style, word);
                        let line_empty = frags.is_empty() && objs.is_empty();
                        let space_width = match (space, line_empty) {
                            (Some(s), false) => self.engine.measure_line(s, " "),
                            _ => 0,
                        };
                        if !line_empty && cursor + space_width + word_width > line_w {
                            commit!(style);
                            // The word starts the next line, without the space.
                            frags.push(Fragment {
                                run: ri,
                                range: pos..end,
                                x: 0,
                                width: word_width,
                            });
                            cursor = word_width;
                        } else {
                            // A word following its own run's earlier words —
                            // any gap between them is that run's own spaces —
                            // joins the fragment, so one draw call speaks the
                            // whole phrase. An image since then breaks the
                            // merge: text must not be drawn across it.
                            let obj_after_last = objs
                                .last()
                                .zip(frags.last())
                                .is_some_and(|((_, ox), f)| *ox >= f.x);
                            match frags.last_mut() {
                                Some(last) if last.run == ri && !obj_after_last => {
                                    last.range.end = end;
                                    last.width += space_width + word_width;
                                }
                                _ => frags.push(Fragment {
                                    run: ri,
                                    range: pos..end,
                                    x: cursor + space_width,
                                    width: word_width,
                                }),
                            }
                            cursor += space_width + word_width;
                        }
                        space = None;
                        pos = end;
                    }
                }
            }
        }
        if !frags.is_empty() || !objs.is_empty() {
            commit!(container_style);
        }

        let height = y;
        (
            FlowLayout {
                runs,
                lines,
                height,
            },
            placed,
        )
    }
}

/// HTML's whitespace rule for normal flow: any run of whitespace is one
/// space, and a space that follows a space — even across element borders —
/// is nothing at all.
fn collapse_whitespace(text: &str, ends_with_space: &mut bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_whitespace() {
            if !*ends_with_space {
                out.push(' ');
                *ends_with_space = true;
            }
        } else {
            out.push(ch);
            *ends_with_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Palette, cascade};

    fn palette() -> Palette {
        Palette {
            text: Color::rgb(20, 20, 20),
            link: Color::rgb(0, 80, 200),
            tint: Color::rgb(240, 240, 240),
        }
    }

    /// The built-in bitmap font is monospaced, which makes widths arithmetic
    /// instead of font trivia: every glyph advances by [`adv`] pixels at the
    /// 8 px size these tests run at (UA 16 px at scale 0.5).
    fn layout(html: &str, width: i32) -> PageLayout {
        let dom = Dom::parse(html);
        let c = cascade(&dom, &palette(), &crate::css::Stylesheet::parse("", 800));
        let mut engine = TextEngine::new();
        let fonts = Fonts::all(denise_text::FontId(0));
        let forms = crate::forms::extract(&dom);
        layout_page(
            &dom,
            &c,
            width,
            0.5,
            &fonts,
            &HashMap::new(),
            &forms,
            &mut engine,
        )
    }

    /// One glyph's advance in the built-in font at size 8.
    fn adv() -> i32 {
        TextEngine::new().measure_line(TextStyle::built_in(8), "x")
    }

    fn flows(page: &PageLayout) -> Vec<&FlowLayout> {
        page.leaves
            .iter()
            .filter_map(|p| match &p.leaf {
                Leaf::Flow(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn one_short_paragraph_is_one_line() {
        let page = layout("<p>hello world</p>", 800);
        let flows = flows(&page);
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].lines.len(), 1);
        // "hello world": 11 glyphs, spaces included.
        let line = &flows[0].lines[0];
        let end = line.fragments.last().unwrap();
        assert_eq!(end.x + end.width, 11 * adv());
    }

    #[test]
    fn words_wrap_at_the_width() {
        // Room for two four-glyph words and the space between, not three.
        let page = layout("<p>aaaa bbbb cccc</p>", 11 * adv());
        let flows = flows(&page);
        assert_eq!(flows[0].lines.len(), 2);
        assert_eq!(flows[0].lines[1].fragments[0].x, 0);
    }

    #[test]
    fn a_word_wider_than_the_line_stands_alone() {
        let page = layout("<p>a abcdefghijklmnopqrstuvwxyz b</p>", 40);
        let flows = flows(&page);
        assert_eq!(flows[0].lines.len(), 3);
        assert!(flows[0].lines[1].fragments[0].width > 40, "it overflows");
    }

    #[test]
    fn styles_change_mid_line_without_a_break() {
        let page = layout("<p>one <b>two</b> three</p>", 800);
        let flows = flows(&page);
        let line = &flows[0].lines[0];
        // Three runs, one line; the spaces around <b> survive.
        assert_eq!(line.fragments.len(), 3);
        assert_eq!(flows[0].runs.len(), 3);
        // "one " then "two" then " three": 4, then 4+3+1 glyphs across.
        assert_eq!(line.fragments[1].x, 4 * adv());
        assert_eq!(line.fragments[2].x, 8 * adv());
    }

    #[test]
    fn a_br_breaks_the_line() {
        let page = layout("<p>a<br>b</p>", 800);
        let flows = flows(&page);
        assert_eq!(flows[0].lines.len(), 2);
        assert_eq!(flows[0].lines[0].height, flows[0].lines[1].y);
    }

    #[test]
    fn pre_keeps_its_shape() {
        let page = layout("<pre>one  two\n   indented</pre>", 800);
        let flows = flows(&page);
        assert_eq!(flows[0].lines.len(), 2);
        // Two spaces stay two spaces: "one  two" is 8 glyphs.
        let first = &flows[0].lines[0].fragments[0];
        assert_eq!(first.width, 8 * adv());
        // The indent stays an indent.
        let second = &flows[0].lines[1].fragments[0];
        assert_eq!(
            flows[0].runs[second.run].text[second.range.clone()],
            *"   indented"
        );
    }

    #[test]
    fn blocks_stack_and_margins_collapse_between_siblings() {
        let page = layout("<p>a</p><p>b</p>", 800);
        let flows = flows(&page);
        assert_eq!(flows.len(), 2);
        let first = flows_rects(&page)[0];
        let second = flows_rects(&page)[1];
        // 16 logical of margin at scale 0.5 = 8 physical between the
        // paragraphs — collapsed, not 16.
        assert_eq!(second.y - (first.y + first.height), 8);
    }

    fn flows_rects(page: &PageLayout) -> Vec<Rect> {
        page.leaves
            .iter()
            .filter_map(|p| match &p.leaf {
                Leaf::Flow(_) => Some(p.rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn list_items_get_markers() {
        let page = layout("<ol><li>one</li><li>two</li></ol>", 800);
        let bullets: Vec<&str> = page
            .leaves
            .iter()
            .filter_map(|p| match &p.leaf {
                Leaf::Bullet { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(bullets, vec!["1.", "2."]);
    }

    #[test]
    fn the_pre_background_wraps_its_content() {
        let page = layout("<pre>x</pre>", 800);
        let bg = page
            .leaves
            .iter()
            .find_map(|p| match &p.leaf {
                Leaf::Background(_) => Some(p.rect),
                _ => None,
            })
            .expect("pre paints a background");
        assert!(bg.height >= 8, "taller than the text alone");
        assert!(bg.width > 0);
    }

    #[test]
    fn whitespace_only_markup_makes_no_flow() {
        let page = layout("<div> \n\t </div>", 800);
        assert!(flows(&page).is_empty());
    }

    #[test]
    fn a_table_lays_its_cells_side_by_side() {
        let page = layout(
            "<table><tr><td>left</td><td>right</td></tr>\
             <tr><td>a</td><td>b</td></tr></table>",
            800,
        );
        let rects = flows_rects(&page);
        assert_eq!(rects.len(), 4);
        // First row: same top, different columns.
        assert_eq!(rects[0].y, rects[1].y);
        assert!(rects[1].x > rects[0].x + 100, "a real second column");
        // Second row sits below the first.
        assert!(rects[2].y > rects[0].y);
        assert_eq!(rects[2].x, rects[0].x, "columns line up down the rows");
    }

    #[test]
    fn a_spacer_cell_indents_like_the_old_web() {
        // The comment-thread idiom: an empty cell holding a sized image.
        let page = layout(
            "<table><tr><td><img src=s.gif width=40 height=1></td>\
             <td>the comment text itself</td></tr></table>",
            800,
        );
        let rects = flows_rects(&page);
        // The text cell starts just past the 20-px spacer (40 logical at
        // scale 0.5), not at the halfway mark an equal split would give.
        assert!(rects[0].x < 40, "text at {}", rects[0].x);
    }

    #[test]
    fn a_wide_table_still_linearises() {
        let page = layout(
            "<table><tr><td>1</td><td>2</td><td>3</td><td>4</td><td>5</td>\
             <td>6</td><td>7</td><td>8</td><td>9</td></tr></table>",
            800,
        );
        let rects = flows_rects(&page);
        // Nine columns in 800px would be unreadable; they stack instead.
        assert!(rects.windows(2).all(|w| w[1].y > w[0].y));
    }

    #[test]
    fn ids_become_anchors_at_their_height() {
        let page = layout(
            "<p>opening</p><h2 id=\"verse\">a heading far down</h2>",
            800,
        );
        let anchor = page
            .anchors
            .iter()
            .find(|(id, _)| id == "verse")
            .expect("the id was recorded");
        assert!(anchor.1 > 0, "below the opening paragraph");
    }

    #[test]
    fn a_float_stands_aside_and_the_text_runs_past() {
        let page = layout(
            "<div style=\"float: right; width: 400px\">tall<br>tall<br>tall<br>tall</div>\
             <p>words words words words words words words words words words words words \
             words words words words words words words words</p>",
            800,
        );
        let rects = flows_rects(&page);
        // The float's own flow sits on the right (400 logical = 200 physical).
        assert!(
            rects[0].x >= 800 - 200 - 12,
            "float content at {}",
            rects[0].x
        );
        // The paragraph's lines keep left and stop short of the float.
        let flows = flows(&page);
        let para = flows.last().unwrap();
        let first = &para.lines[0];
        let end = first.fragments.last().map(|f| f.x + f.width).unwrap();
        assert!(end <= 800 - 200, "first line ends at {end}");
        assert_eq!(first.fragments[0].x, 0);
        assert!(para.lines.len() > 1, "the narrowed lines wrapped");
    }

    #[test]
    fn clear_drops_below_the_float() {
        let page = layout(
            "<div style=\"float: left; width: 200px\">x<br>x<br>x<br>x</div>\
             <p style=\"clear: both\">below</p>",
            800,
        );
        let rects = flows_rects(&page);
        // The cleared paragraph starts below the float's bottom, back at
        // the left edge.
        assert!(
            rects[1].y >= rects[0].y + rects[0].height,
            "below the float"
        );
        assert!(rects[1].x <= 8, "back at the left, not beside");
    }

    #[test]
    fn the_align_attribute_still_floats_an_image() {
        let page = layout(
            "<p><img src=\"pic.png\" width=\"100\" height=\"100\" align=\"right\">\
             beside the picture</p>",
            800,
        );
        let image = page
            .leaves
            .iter()
            .find_map(|p| match &p.leaf {
                Leaf::Image { .. } => Some(p.rect),
                _ => None,
            })
            .expect("the image floated as a block");
        assert!(image.x >= 800 - 50 - 12, "on the right at {}", image.x);
    }

    #[test]
    fn mixed_sizes_share_a_baseline() {
        let page = layout("<p>small <big>big</big></p>", 800);
        let flows = flows(&page);
        let line = &flows[0].lines[0];
        // The line is as tall as its tallest run and the baseline is that
        // run's ascent, so the small text rides the same baseline.
        let big_style = flows[0].runs[line.fragments[1].run].style;
        assert!(big_style.size_px > flows[0].runs[0].style.size_px);
        assert!(line.baseline >= line.height / 2);
    }
}
