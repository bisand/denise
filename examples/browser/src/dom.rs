//! The document tree, and the sink html5ever fills it through.
//!
//! An arena, not an `Rc<RefCell<Node>>` web: nodes live in one `Vec` and a
//! handle is an index. The parser wants shared mutability while it builds —
//! its [`TreeSink`] takes `&self` — so the vector sits behind a `RefCell`
//! for exactly as long as parsing runs. [`Sink::finish`] unwraps it, and the
//! [`Dom`] every later pass reads is plain data with plain indices.
//!
//! What is kept is what rendering needs: element name, attributes, text.
//! Comments become [`NodeData::Comment`] so that sibling text does not merge
//! across them into something the author never wrote; doctypes and processing
//! instructions are dropped entirely.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;

use html5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, LocalName, ParseOpts, QualName, parse_document};

/// A parsed document. Node 0 is the document itself.
pub struct Dom {
    pub nodes: Vec<DomNode>,
}

pub struct DomNode {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub data: NodeData,
}

pub enum NodeData {
    Document,
    Element {
        name: QualName,
        attrs: Vec<(LocalName, String)>,
    },
    Text(String),
    Comment,
}

impl Dom {
    /// Parses HTML the way a browser does: tag soup in, a tree out, never an
    /// error. What the parser cannot make sense of it repairs, as the spec
    /// requires.
    pub fn parse(html: &str) -> Self {
        parse_document(Sink::default(), ParseOpts::default()).one(html)
    }

    /// The element's lowercase tag name, or `None` for anything that is not
    /// an element.
    pub fn tag(&self, idx: usize) -> Option<&str> {
        match &self.nodes[idx].data {
            NodeData::Element { name, .. } => Some(&name.local),
            _ => None,
        }
    }

    /// An attribute by name, on an element.
    pub fn attr(&self, idx: usize, attr: &str) -> Option<&str> {
        match &self.nodes[idx].data {
            NodeData::Element { attrs, .. } => attrs
                .iter()
                .find(|(name, _)| name.as_ref() == attr)
                .map(|(_, value)| value.as_str()),
            _ => None,
        }
    }

    /// Every descendant text node, concatenated. What `<title>` and
    /// `<option>` mean by their contents.
    // Forms read option and button labels through this; until they land it
    // is exercised by the tests alone.
    #[allow(dead_code)]
    pub fn text_content(&self, idx: usize) -> String {
        let mut out = String::new();
        self.collect_text(idx, &mut out);
        out
    }

    fn collect_text(&self, idx: usize, out: &mut String) {
        match &self.nodes[idx].data {
            NodeData::Text(text) => out.push_str(text),
            _ => {
                for &child in &self.nodes[idx].children {
                    self.collect_text(child, out);
                }
            }
        }
    }

    /// The first descendant element with this tag, in document order.
    pub fn find(&self, tag: &str) -> Option<usize> {
        self.find_from(0, tag)
    }

    /// Every element with this tag, in document order.
    pub fn find_all(&self, tag: &str) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_tag(0, tag, &mut out);
        out
    }

    fn collect_tag(&self, idx: usize, tag: &str, out: &mut Vec<usize>) {
        if self.tag(idx) == Some(tag) {
            out.push(idx);
        }
        for &child in &self.nodes[idx].children {
            self.collect_tag(child, tag, out);
        }
    }

    fn find_from(&self, idx: usize, tag: &str) -> Option<usize> {
        if self.tag(idx) == Some(tag) {
            return Some(idx);
        }
        self.nodes[idx]
            .children
            .iter()
            .find_map(|&child| self.find_from(child, tag))
    }
}

/// The parser's view of the arena. Alive only inside [`Dom::parse`].
#[derive(Default)]
pub struct Sink {
    nodes: RefCell<Vec<DomNode>>,
    /// A `<template>`'s contents parse into a separate subtree the spec says
    /// must exist. Nothing renders it, but the parser must be handed the same
    /// node each time it asks.
    template_contents: RefCell<HashMap<usize, usize>>,
}

impl Sink {
    fn push(&self, data: NodeData) -> usize {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(DomNode {
            parent: None,
            children: Vec::new(),
            data,
        });
        nodes.len() - 1
    }

    fn detach(&self, nodes: &mut [DomNode], target: usize) {
        if let Some(parent) = nodes[target].parent.take() {
            nodes[parent].children.retain(|&c| c != target);
        }
    }

    /// Appends under `parent` at `position` (`None` for the end), merging
    /// text into a preceding text sibling as the spec asks.
    fn insert(&self, parent: usize, position: Option<usize>, child: NodeOrText<usize>) {
        let mut nodes = self.nodes.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => {
                self.detach(&mut nodes, node);
                nodes[node].parent = Some(parent);
                let at = position.unwrap_or(nodes[parent].children.len());
                nodes[parent].children.insert(at, node);
            }
            NodeOrText::AppendText(text) => {
                let end = position.unwrap_or(nodes[parent].children.len());
                let previous = end.checked_sub(1).map(|i| nodes[parent].children[i]);
                if let Some(prev) = previous
                    && let NodeData::Text(existing) = &mut nodes[prev].data
                {
                    existing.push_str(&text);
                    return;
                }
                drop(nodes);
                let node = self.push(NodeData::Text(text.to_string()));
                let mut nodes = self.nodes.borrow_mut();
                nodes[node].parent = Some(parent);
                nodes[parent].children.insert(end, node);
            }
        }
    }
}

impl TreeSink for Sink {
    type Handle = usize;
    type Output = Dom;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Dom {
        let mut nodes = self.nodes.into_inner();
        if nodes.is_empty() {
            nodes.push(DomNode {
                parent: None,
                children: Vec::new(),
                data: NodeData::Document,
            });
        }
        Dom { nodes }
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {}

    fn get_document(&self) -> usize {
        if self.nodes.borrow().is_empty() {
            self.push(NodeData::Document);
        }
        0
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> Ref<'a, QualName> {
        Ref::map(self.nodes.borrow(), |nodes| match &nodes[*target].data {
            NodeData::Element { name, .. } => name,
            _ => unreachable!("elem_name on a non-element"),
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, _flags: ElementFlags) -> usize {
        let attrs = attrs
            .into_iter()
            .map(|a| (a.name.local, a.value.to_string()))
            .collect();
        self.push(NodeData::Element { name, attrs })
    }

    fn create_comment(&self, _text: StrTendril) -> usize {
        self.push(NodeData::Comment)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> usize {
        self.push(NodeData::Comment)
    }

    fn append(&self, parent: &usize, child: NodeOrText<usize>) {
        self.insert(*parent, None, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &usize,
        prev_element: &usize,
        child: NodeOrText<usize>,
    ) {
        if self.nodes.borrow()[*element].parent.is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
    }

    fn get_template_contents(&self, target: &usize) -> usize {
        if let Some(&contents) = self.template_contents.borrow().get(target) {
            return contents;
        }
        let contents = self.push(NodeData::Document);
        self.template_contents
            .borrow_mut()
            .insert(*target, contents);
        contents
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &usize, new_node: NodeOrText<usize>) {
        let (parent, position) = {
            let nodes = self.nodes.borrow();
            let parent = nodes[*sibling].parent.expect("sibling has a parent");
            let position = nodes[parent]
                .children
                .iter()
                .position(|&c| c == *sibling)
                .expect("sibling is its parent's child");
            (parent, position)
        };
        self.insert(parent, Some(position), new_node);
    }

    fn add_attrs_if_missing(&self, target: &usize, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        let NodeData::Element {
            attrs: existing, ..
        } = &mut nodes[*target].data
        else {
            return;
        };
        for attr in attrs {
            if !existing.iter().any(|(name, _)| *name == attr.name.local) {
                existing.push((attr.name.local, attr.value.to_string()));
            }
        }
    }

    fn remove_from_parent(&self, target: &usize) {
        let mut nodes = self.nodes.borrow_mut();
        self.detach(&mut nodes, *target);
    }

    fn reparent_children(&self, node: &usize, new_parent: &usize) {
        let mut nodes = self.nodes.borrow_mut();
        let children = std::mem::take(&mut nodes[*node].children);
        for &child in &children {
            nodes[child].parent = Some(*new_parent);
        }
        nodes[*new_parent].children.extend(children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simple_page_has_its_shape() {
        let dom = Dom::parse("<html><body><p>Hello</p></body></html>");
        let p = dom.find("p").expect("a paragraph");
        assert_eq!(dom.text_content(p), "Hello");
        assert!(dom.find("body").is_some());
    }

    #[test]
    fn the_parser_repairs_tag_soup() {
        // A <p> cannot contain a <div>; the parser closes it first. What
        // matters here is not the exact repair but that it neither panics
        // nor loses text.
        let dom = Dom::parse("<p>before<div>inside</div>after");
        let body = dom.find("body").expect("a body");
        assert!(dom.text_content(body).contains("before"));
        assert!(dom.text_content(body).contains("inside"));
        assert!(dom.text_content(body).contains("after"));
    }

    #[test]
    fn adjacent_text_merges_into_one_node() {
        // "a<!-- x -->b" keeps two text nodes (the comment sits between),
        // while character data split only by parser buffering becomes one.
        let dom = Dom::parse("<p>one &amp; two</p>");
        let p = dom.find("p").expect("a paragraph");
        assert_eq!(dom.nodes[p].children.len(), 1);
        assert_eq!(dom.text_content(p), "one & two");
    }

    #[test]
    fn attributes_survive_with_their_values() {
        let dom = Dom::parse(r#"<a href="/x" class="link">go</a>"#);
        let a = dom.find("a").expect("an anchor");
        assert_eq!(dom.attr(a, "href"), Some("/x"));
        assert_eq!(dom.attr(a, "class"), Some("link"));
        assert_eq!(dom.attr(a, "title"), None);
    }

    #[test]
    fn a_head_and_title_are_kept_for_reading() {
        let dom = Dom::parse("<title>The name</title><p>body</p>");
        let title = dom.find("title").expect("a title");
        assert_eq!(dom.text_content(title), "The name");
    }
}
