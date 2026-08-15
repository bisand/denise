//! The browser, apart from what it runs on.
//!
//! The chrome — Back, Forward, Reload, the URL bar, a spinner — is stock
//! Denise widgets speaking theme roles. The page below is the other half of
//! the demonstration: custom widgets carrying an author's own colours, built
//! from the same trait with the same one required method. Navigation is a
//! message loop: widgets emit, [`App::handle`] drains, and the network
//! answers arrive on the same cadence through [`Net::done`].

use std::time::Instant;

use denise::{InputEvent, Point, Rect, Role, Size, Theme};
use denise_text::{GlyphSource, TrueTypeSource};
use denise_ui::widgets::{Button, Panel, Spinner, TextInput};
use denise_ui::{Motion, NodeId, Ui, Void};
use url::Url;

use crate::dom::Dom;
use crate::history::History;
use crate::layout::{Fonts, layout_page};
use crate::net::{FetchDone, FetchKind, Fetched, Net};
use crate::page::{self, Page};
use crate::style::{Palette, cascade};

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Enter in the URL bar.
    UrlSubmitted,
    /// A link in a flow, by index into the current page's link table.
    Navigate(usize),
    Back,
    Forward,
    Reload,
    /// A submit control, or Enter in a form's text input.
    SubmitForm(usize),
    /// A page `Select` asking for its popup, by index into `Page::selects`.
    OpenSelect(usize),
    /// The open popup's choice.
    SelectChanged(usize),
    /// A control whose change nobody reacts to: values are read at submit.
    Noop,
}

fn select_changed(index: usize) -> Message {
    Message::SelectChanged(index)
}

/// Chrome geometry, logical pixels.
const BAR: i32 = 48;
const PAD: i32 = 8;
const BUTTON_W: i32 = 36;
const BUTTON_H: i32 = 32;

pub struct App {
    pub ui: Ui<Message>,
    scale: f32,
    /// The surface, physical pixels.
    size: Size,
    fonts: Fonts,
    palette: Palette,
    chrome: Chrome,
    page: Option<Page>,
    /// The current page's source and address, kept so a resize can relayout
    /// without refetching.
    source: Option<(String, Url)>,
    pending: Option<Pending>,
    net: Net,
    history: History,
    started: Instant,
    /// Decoded sizes of the current page's images, fed back into layout.
    natural: std::collections::HashMap<usize, Size>,
    /// Decoded pixels, so a relayout or resize refills boxes without
    /// refetching a byte.
    pixels: std::collections::HashMap<usize, (Vec<u32>, Size)>,
    /// Image fetches in flight, request id to element.
    inflight: std::collections::HashMap<u64, usize>,
    /// Linked stylesheets: absolute URL to arrived text. An error caches an
    /// empty sheet so a dead link is asked exactly once.
    css_cache: std::collections::HashMap<String, String>,
    /// Stylesheet fetches in flight, request id to their cache key.
    css_inflight: std::collections::HashMap<u64, String>,
    /// An image without a final box arrived; relayout once, after the
    /// frame's arrivals are all in.
    needs_relayout: bool,
    /// The page `Select` whose popup is open, if any.
    open_select_target: Option<NodeId>,
}

struct Chrome {
    bar: NodeId,
    back: NodeId,
    forward: NodeId,
    reload: NodeId,
    url: NodeId,
    spinner: NodeId,
    content: NodeId,
}

struct Pending {
    id: u64,
    /// Whether arrival makes a new history entry (a click) or replaces the
    /// current one (Back, Forward, Reload).
    push: bool,
    /// Scroll position to restore once the page is up.
    restore: Point,
}

/// What either backend hands the app builder.
pub type Font = Option<(String, Box<dyn GlyphSource>)>;

impl App {
    pub fn new(size: Size, scale: f32, font: Font, motion: Motion, start: Option<String>) -> Self {
        let theme = Theme::BUILT_IN[0].scaled(scale);
        let palette = Palette {
            text: theme.color(Role::BaseContent),
            link: theme.color(Role::Primary),
            tint: theme.color(Role::Base200),
        };
        let mut ui = Ui::new(size, theme);
        ui.set_motion(motion);
        let fonts = load_fonts(&mut ui, font);
        let chrome_style = denise_text::TextStyle {
            font: fonts.regular,
            size_px: ((15.0 * scale).round() as u16).max(1),
        };
        let chrome = build_chrome(&mut ui, size, scale, chrome_style);

        let mut app = Self {
            ui,
            scale,
            size,
            fonts,
            palette,
            chrome,
            page: None,
            source: None,
            pending: None,
            net: Net::start(),
            history: History::default(),
            started: Instant::now(),
            natural: std::collections::HashMap::new(),
            pixels: std::collections::HashMap::new(),
            inflight: std::collections::HashMap::new(),
            css_cache: std::collections::HashMap::new(),
            css_inflight: std::collections::HashMap::new(),
            needs_relayout: false,
            open_select_target: None,
        };
        match start.as_deref().and_then(to_url) {
            Some(url) => {
                app.history.push(url.clone());
                app.navigate(url, false, Point::ZERO);
            }
            None => app.show_welcome(),
        }
        app
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// A fetch is in flight — the page itself, or any of its images; the
    /// backends poll while this holds.
    pub fn loading(&self) -> bool {
        self.pending.is_some() || !self.inflight.is_empty() || !self.css_inflight.is_empty()
    }

    /// Keys and events the application claims before or beside the tree:
    /// Alt with an arrow is history, a resize is a relayout.
    pub fn claim(&mut self, events: &[InputEvent]) {
        use denise::{ElementState, KeyCode, Modifiers};
        for event in events {
            match event {
                InputEvent::SurfaceResized { size, scale_factor } => {
                    self.on_resize(*size, *scale_factor);
                }
                InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    modifiers,
                    ..
                } if modifiers.contains(Modifiers::ALT) => match code {
                    KeyCode::ArrowLeft => self.on_message(Message::Back),
                    KeyCode::ArrowRight => self.on_message(Message::Forward),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Once a frame: what the network delivered, then what the tree said.
    /// Image arrivals that move text are batched into one relayout at the
    /// end, however many came in together.
    pub fn handle(&mut self, _now_ms: u64) {
        for done in self.net.done() {
            self.on_fetch(done);
        }
        if core::mem::take(&mut self.needs_relayout)
            && let Some((html, url)) = self.source.clone()
        {
            let scroll = self.ui.scroll(self.chrome.content);
            self.show_page(html, url, scroll);
        }
        let messages: Vec<Message> = self.ui.drain_messages().collect();
        for message in messages {
            self.on_message(message);
        }
    }

    pub fn on_message(&mut self, message: Message) {
        match message {
            Message::UrlSubmitted => {
                let text = self
                    .ui
                    .widget::<TextInput<Message>>(self.chrome.url)
                    .map(|input| input.text().to_string())
                    .unwrap_or_default();
                if let Some(url) = to_url(&text) {
                    self.navigate(url, true, Point::ZERO);
                }
            }
            Message::Navigate(index) => {
                let target = self
                    .page
                    .as_ref()
                    .and_then(|p| p.links.get(index).cloned().flatten());
                if let Some(url) = target {
                    self.navigate(url, true, Point::ZERO);
                }
            }
            Message::Back => {
                let scroll = self.ui.scroll(self.chrome.content);
                self.history.save_scroll(scroll);
                if let Some(entry) = self.history.back() {
                    let (url, restore) = (entry.url.clone(), entry.scroll);
                    self.navigate(url, false, restore);
                }
            }
            Message::Forward => {
                let scroll = self.ui.scroll(self.chrome.content);
                self.history.save_scroll(scroll);
                if let Some(entry) = self.history.forward() {
                    let (url, restore) = (entry.url.clone(), entry.scroll);
                    self.navigate(url, false, restore);
                }
            }
            Message::Reload => {
                if let Some(entry) = self.history.current() {
                    let url = entry.url.clone();
                    let restore = self.ui.scroll(self.chrome.content);
                    self.navigate(url, false, restore);
                }
            }
            Message::SubmitForm(index) => self.submit(index),
            Message::OpenSelect(index) => {
                if let Some(node) = self
                    .page
                    .as_ref()
                    .and_then(|p| p.selects.get(index).copied())
                {
                    self.open_select_target = Some(node);
                    denise_ui::widgets::open_select(&mut self.ui, node, select_changed);
                }
            }
            Message::SelectChanged(index) => {
                if let Some(node) = self.open_select_target.take()
                    && let Some(widget) = self
                        .ui
                        .widget_mut::<denise_ui::widgets::Select<Message>>(node)
                {
                    widget.set_selected(Some(index));
                }
                self.ui.close_popup();
            }
            Message::Noop => {}
        }
    }

    /// Reads the form's live widgets, serialises the successful controls,
    /// and goes where the action says — as a query for GET, a body for
    /// POST. A `file:` form gets a preview page instead of a request, which
    /// keeps the whole flow demonstrable offline.
    fn submit(&mut self, form_index: usize) {
        use crate::forms::{FieldKind, Method};
        use denise_ui::widgets::{Checkbox, RadioGroup, Select};

        let Some(page) = &self.page else {
            return;
        };
        let Some(form) = page.forms.forms.get(form_index) else {
            return;
        };
        let mut pairs: Vec<(String, String)> = Vec::new();
        for field in &form.fields {
            let Some(name) = &field.name else {
                continue;
            };
            let node = page.controls.get(&field.dom).copied();
            let value = match &field.kind {
                FieldKind::Hidden { value } => Some(value.clone()),
                FieldKind::Text => node
                    .and_then(|n| self.ui.widget::<TextInput<Message>>(n))
                    .map(|w| w.text().to_string()),
                FieldKind::Checkbox { value } => node
                    .and_then(|n| self.ui.widget::<Checkbox<Message>>(n))
                    .filter(|w| w.checked())
                    .map(|_| value.clone()),
                FieldKind::Radio { values } => node
                    .and_then(|n| self.ui.widget::<RadioGroup<Message>>(n))
                    .and_then(|w| values.get(w.selected()).cloned()),
                FieldKind::Select { values } => node
                    .and_then(|n| self.ui.widget::<Select<Message>>(n))
                    .and_then(|w| w.selected())
                    .and_then(|i| values.get(i).cloned()),
            };
            if let Some(value) = value {
                pairs.push((name.clone(), value));
            }
        }
        let body = crate::forms::urlencoded(&pairs);
        let method = form.method;
        let action = form.action.clone();
        let base = page.base.clone();

        let mut target = match action.as_deref() {
            Some(action) if !action.is_empty() => match base.join(action) {
                Ok(url) => url,
                Err(_) => return,
            },
            _ => base,
        };
        if target.scheme() == "file" {
            // No request leaves the machine; the page shows what one would
            // have said. The preview goes through the ordinary pipeline,
            // which makes it a test of that pipeline too.
            self.preview_submission(method, &target, &pairs);
            return;
        }
        match method {
            Method::Get => {
                target.set_query(if body.is_empty() { None } else { Some(&body) });
                self.navigate(target, true, Point::ZERO);
            }
            Method::Post => {
                let scroll = self.ui.scroll(self.chrome.content);
                self.history.save_scroll(scroll);
                let id = self.net.fetch(FetchKind::Page, target.clone(), Some(body));
                self.pending = Some(Pending {
                    id,
                    push: true,
                    restore: Point::ZERO,
                });
                self.ui.set_visible(self.chrome.spinner, true);
                self.set_url_bar(target.as_str());
            }
        }
    }

    fn preview_submission(
        &mut self,
        method: crate::forms::Method,
        target: &Url,
        pairs: &[(String, String)],
    ) {
        let verb = match method {
            crate::forms::Method::Get => "GET",
            crate::forms::Method::Post => "POST",
        };
        let mut rows = String::new();
        for (name, value) in pairs {
            rows.push_str(&format!("{} = {}\n", escape(name), escape(value)));
        }
        let html = format!(
            "<html><body><h1>Form submission, previewed</h1>\
             <p>A <b>{verb}</b> to <code>{}</code> would have carried:</p>\
             <pre>{rows}</pre>\
             <p>Pages from <code>file:</code> get a preview instead of a
             request; a page from the network submits for real.
             <b>Back</b> returns to the form.</p></body></html>",
            escape(target.as_str()),
        );
        let url = Url::parse("about:submitted").expect("a fixed URL");
        self.history.push(url.clone());
        self.show_page(html, url, Point::ZERO);
    }

    fn navigate(&mut self, url: Url, push: bool, restore: Point) {
        if push {
            let scroll = self.ui.scroll(self.chrome.content);
            self.history.save_scroll(scroll);
        }
        // A fragment on the page already showing is a scroll, not a fetch:
        // a table of contents would be unusable if every entry re-downloaded
        // the article it points into.
        if url.fragment().is_some()
            && self
                .source
                .as_ref()
                .is_some_and(|(_, current)| same_document(current, &url))
        {
            if push {
                self.history.push(url.clone());
            }
            self.set_url_bar(url.as_str());
            self.refresh_nav_buttons();
            self.scroll_to_fragment(&url);
            return;
        }
        if url.scheme() == "about" {
            if push {
                self.history.push(url.clone());
            }
            self.pending = None;
            self.show_welcome_at(url);
            return;
        }
        let id = self.net.fetch(FetchKind::Page, url.clone(), None);
        self.pending = Some(Pending { id, push, restore });
        self.ui.set_visible(self.chrome.spinner, true);
        self.set_url_bar(url.as_str());
    }

    fn on_fetch(&mut self, done: FetchDone) {
        match done.kind {
            FetchKind::Page => {
                let Some(pending) = &self.pending else {
                    return;
                };
                if pending.id != done.id {
                    // A response from a navigation that was navigated past.
                    return;
                }
                let Pending { push, restore, .. } = self.pending.take().expect("checked");
                self.ui.set_visible(self.chrome.spinner, false);

                let html = match done.result {
                    Ok(Fetched::Text(html)) => html,
                    Ok(Fetched::Bytes(_)) => error_page("that was not a page"),
                    Err(e) => error_page(&e),
                };
                if push {
                    self.history.push(done.final_url.clone());
                } else {
                    self.history.replace(done.final_url.clone());
                }
                self.show_page(html, done.final_url, restore);
            }
            FetchKind::Style => {
                let Some(key) = self.css_inflight.remove(&done.id) else {
                    return;
                };
                let css = match done.result {
                    Ok(Fetched::Text(css)) => css,
                    // A stylesheet that will not come stops being waited
                    // for; the empty entry is what stops re-asking.
                    _ => String::new(),
                };
                self.css_cache.insert(key, css);
                self.needs_relayout = true;
            }
            FetchKind::Image { .. } => {
                let Some(dom) = self.inflight.remove(&done.id) else {
                    // A picture for a page that is no longer up.
                    return;
                };
                let Ok(Fetched::Bytes(bytes)) = done.result else {
                    // The placeholder stays; a missing picture is not an
                    // event worth interrupting reading for.
                    return;
                };
                let Ok(picture) = denise_image::decode(&bytes) else {
                    return;
                };
                let (pixels, size) = picture.into_parts();
                let sized = self
                    .page
                    .as_ref()
                    .and_then(|p| p.images.iter().find(|j| j.dom == dom))
                    .is_some_and(|j| j.sized);
                if let Some(node) = self
                    .page
                    .as_ref()
                    .and_then(|p| p.images.iter().find(|j| j.dom == dom))
                    .map(|j| j.node)
                    && let Some(widget) = self.ui.widget_mut::<denise_ui::widgets::Image>(node)
                {
                    widget.set_pixels(pixels.clone(), size);
                }
                self.natural.insert(dom, size);
                self.pixels.insert(dom, (pixels, size));
                if !sized {
                    self.needs_relayout = true;
                }
            }
        }
    }

    /// The whole pipeline, one direction: text to tree.
    fn show_page(&mut self, html: String, url: Url, restore: Point) {
        // A different address means a different page: its media caches and
        // fetches mean nothing here. The same address is a relayout — a
        // resize, an image arrival — and the caches are the point.
        if self.source.as_ref().map(|(_, u)| u) != Some(&url) {
            self.natural.clear();
            self.pixels.clear();
            self.inflight.clear();
            self.css_cache.clear();
            self.css_inflight.clear();
            self.needs_relayout = false;
        }
        let dom = Dom::parse(&html);
        let forms = crate::forms::extract(&dom);

        // Relative hrefs resolve against <base href>, when the page names
        // one, else against where the page actually came from.
        let base = dom
            .find("base")
            .and_then(|b| dom.attr(b, "href"))
            .and_then(|href| url.join(href).ok())
            .unwrap_or_else(|| url.clone());

        // The author's stylesheets: the page's own <style> blocks, then
        // linked sheets — arrived ones inline, missing ones fetched. Each
        // arrival relayouts the same page with more of the truth.
        let mut css = String::new();
        for style_el in dom.find_all("style") {
            css.push_str(&dom.text_content(style_el));
            css.push('\n');
        }
        let mut queued_css = 0;
        for link in dom.find_all("link") {
            let rel = dom.attr(link, "rel").unwrap_or_default();
            if !rel
                .to_ascii_lowercase()
                .split_ascii_whitespace()
                .any(|r| r == "stylesheet")
            {
                continue;
            }
            if let Some(media) = dom.attr(link, "media") {
                let media = media.to_ascii_lowercase();
                if !media.contains("screen") && !media.contains("all") {
                    continue;
                }
            }
            let Some(href) = dom.attr(link, "href") else {
                continue;
            };
            let Ok(target) = base.join(href) else {
                continue;
            };
            match target.scheme() {
                "http" | "https" => {}
                "file" if url.scheme() == "file" => {}
                _ => continue,
            }
            let key = target.to_string();
            if let Some(text) = self.css_cache.get(&key) {
                css.push_str(text);
                css.push('\n');
            } else if !self.css_inflight.values().any(|k| *k == key) && queued_css < 8 {
                let id = self.net.fetch(FetchKind::Style, target, None);
                self.css_inflight.insert(id, key);
                queued_css += 1;
            }
        }
        // Media queries ask about CSS pixels, which are our logical ones.
        let viewport = (self.size.width as f32 / self.scale).round() as i32;
        let sheet = crate::css::Stylesheet::parse(&css, viewport);
        let styled = cascade(&dom, &self.palette, &sheet);
        let links = styled
            .links
            .iter()
            .map(|href| base.join(href).ok())
            .collect();

        if let Some(old) = self.page.take() {
            self.ui.remove(old.root);
        }
        let width = self.size.width as i32;
        let layout = layout_page(
            &dom,
            &styled,
            width,
            self.scale,
            &self.fonts,
            &self.natural,
            &forms,
            self.ui.text_mut(),
        );
        let control_style = denise_text::TextStyle {
            font: self.fonts.regular,
            size_px: ((15.0 * self.scale).round() as u16).max(1),
        };
        self.page = Some(page::build(
            &mut self.ui,
            self.chrome.content,
            width,
            layout,
            links,
            forms,
            base.clone(),
            control_style,
        ));
        self.ui.set_scroll(self.chrome.content, restore);

        // Fill what is already decoded, fetch what is not. The cap keeps a
        // gallery page from queueing its whole archive.
        let jobs: Vec<(usize, String, NodeId)> = self
            .page
            .as_ref()
            .expect("just built")
            .images
            .iter()
            .map(|j| (j.dom, j.src.clone(), j.node))
            .collect();
        let mut queued = 0;
        for (dom, src, node) in jobs {
            if let Some((pixels, size)) = self.pixels.get(&dom) {
                if let Some(widget) = self.ui.widget_mut::<denise_ui::widgets::Image>(node) {
                    widget.set_pixels(pixels.clone(), *size);
                }
                continue;
            }
            if self.inflight.values().any(|&d| d == dom) || queued >= 40 {
                continue;
            }
            let Ok(target) = base.join(&src) else {
                continue;
            };
            // A page from the network does not get to read local files.
            match target.scheme() {
                "http" | "https" => {}
                "file" if url.scheme() == "file" => {}
                _ => continue,
            }
            let id = self.net.fetch(FetchKind::Image { dom }, target, None);
            self.inflight.insert(id, dom);
            queued += 1;
        }
        self.set_url_bar(url.as_str());
        self.refresh_nav_buttons();
        self.source = Some((html, url.clone()));
        // A freshly arrived page with a fragment opens at the fragment.
        if restore == Point::ZERO && url.fragment().is_some() {
            self.scroll_to_fragment(&url);
        }
    }

    fn refresh_nav_buttons(&mut self) {
        self.ui
            .set_enabled(self.chrome.back, self.history.can_back());
        self.ui
            .set_enabled(self.chrome.forward, self.history.can_forward());
    }

    fn scroll_to_fragment(&mut self, url: &Url) {
        let Some(fragment) = url.fragment() else {
            return;
        };
        let target = self.page.as_ref().and_then(|p| {
            p.anchors
                .iter()
                .find(|(id, _)| id == fragment)
                .map(|&(_, y)| y)
        });
        if let Some(y) = target {
            self.ui.set_scroll(self.chrome.content, Point::new(0, y));
        }
    }

    fn set_url_bar(&mut self, text: &str) {
        let shown = if text == "about:welcome" { "" } else { text };
        if let Some(input) = self.ui.widget_mut::<TextInput<Message>>(self.chrome.url) {
            input.set_text(shown);
        }
    }

    fn show_welcome(&mut self) {
        let url = Url::parse("about:welcome").expect("a fixed URL");
        self.history.push(url.clone());
        self.show_welcome_at(url);
    }

    fn show_welcome_at(&mut self, url: Url) {
        self.show_page(WELCOME.to_string(), url, Point::ZERO);
    }

    fn on_resize(&mut self, size: Size, scale: f32) {
        if size == self.size && (scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.size = size;
        self.scale = scale;
        place_chrome(&mut self.ui, &self.chrome, size, scale);
        if let Some((html, url)) = self.source.clone() {
            let scroll = self.ui.scroll(self.chrome.content);
            self.show_page(html, url, scroll);
        }
    }
}

/// Adds the chrome once; `place_chrome` owns every rectangle so a resize
/// runs the same code as startup.
fn build_chrome(
    ui: &mut Ui<Message>,
    size: Size,
    scale: f32,
    style: denise_text::TextStyle,
) -> Chrome {
    let root = ui.root();
    let zero = Rect::ZERO;
    let bar = ui
        .add(root, Panel::filled(Role::Base200), zero)
        .expect("root exists");
    let back = ui
        .add(
            bar,
            Button::new("\u{2190}", Message::Back).with_style(style),
            zero,
        )
        .expect("bar exists");
    let forward = ui
        .add(
            bar,
            Button::new("\u{2192}", Message::Forward).with_style(style),
            zero,
        )
        .expect("bar exists");
    let reload = ui
        .add(bar, ReloadButton { pressed: false }, zero)
        .expect("bar exists");
    let url = ui
        .add(
            bar,
            TextInput::<Message>::new()
                .with_placeholder("address")
                .with_max_chars(2048)
                .with_style(style)
                .with_submit(Message::UrlSubmitted),
            zero,
        )
        .expect("bar exists");
    let spinner = ui.add(bar, Spinner::new(), zero).expect("bar exists");
    ui.set_visible(spinner, false);
    let content = ui.add(root, Void, zero).expect("root exists");
    ui.set_scrollable(content, true);

    let chrome = Chrome {
        bar,
        back,
        forward,
        reload,
        url,
        spinner,
        content,
    };
    place_chrome(ui, &chrome, size, scale);
    ui.set_enabled(chrome.back, false);
    ui.set_enabled(chrome.forward, false);
    chrome
}

fn place_chrome(ui: &mut Ui<Message>, chrome: &Chrome, size: Size, scale: f32) {
    let px = |v: i32| (v as f32 * scale).round() as i32;
    let w = size.width as i32;
    let h = size.height as i32;
    let button_y = px((BAR - BUTTON_H) / 2);
    ui.set_layout(chrome.bar, Rect::new(0, 0, w, px(BAR)));
    for (i, id) in [chrome.back, chrome.forward, chrome.reload]
        .into_iter()
        .enumerate()
    {
        let x = px(PAD) + (i as i32) * px(BUTTON_W + 4);
        ui.set_layout(id, Rect::new(x, button_y, px(BUTTON_W), px(BUTTON_H)));
    }
    let url_x = px(PAD) + 3 * px(BUTTON_W + 4);
    let spinner_side = px(24);
    let url_w = (w - url_x - spinner_side - 2 * px(PAD)).max(px(60));
    ui.set_layout(chrome.url, Rect::new(url_x, button_y, url_w, px(BUTTON_H)));
    ui.set_layout(
        chrome.spinner,
        Rect::new(
            w - spinner_side - px(PAD),
            px((BAR - 24) / 2),
            spinner_side,
            spinner_side,
        ),
    );
    ui.set_layout(
        chrome.content,
        Rect::new(0, px(BAR), w, (h - px(BAR)).max(0)),
    );
}

/// The reload control, drawn rather than typed.
///
/// U+21BB was the obvious label and Arial was the reason it could not be:
/// its character map claims the arrow, then draws the `.notdef` box. No
/// probing of fonts survives a font that lies, so the icon is ink the
/// rasteriser owns — an arc and an arrowhead, correct in every face because
/// it uses none.
struct ReloadButton {
    pressed: bool,
}

impl denise_ui::Widget<Message> for ReloadButton {
    fn paint(&self, ctx: &mut denise_ui::PaintCtx<'_>, canvas: &mut denise_render::Canvas<'_>) {
        use denise::theme::Radius;
        use denise_render::TURN;
        use denise_ui::VisualState;

        let (mut bg, fg) = ctx.theme.pair(Role::Primary);
        if ctx.state.contains(VisualState::PRESSED) {
            bg = bg.mix(fg, 64);
        } else if ctx.state.contains(VisualState::HOVERED) {
            bg = bg.mix(fg, 24);
        }
        let b = ctx.bounds;
        canvas.fill_rounded_rect(b, ctx.theme.radius(Radius::Field), bg);

        let centre = Point::new(b.x + b.width / 2, b.y + b.height / 2);
        let radius = (b.width.min(b.height) * 5 / 16).max(4);
        let thickness = (radius / 4).max(2);
        // Three o'clock round to twelve, clockwise: the gap is top-right
        // and the head sits at twelve pointing the way the arc was going.
        canvas.stroke_arc(centre, radius, thickness, TURN / 4, TURN * 3 / 4, fg);

        let tip = Point::new(centre.x + thickness / 2, centre.y - radius);
        let head = (radius as f32 * 0.55).max(3.0);
        let barb = |side: f32| {
            // Back from the tip (travel is +x at twelve), splayed to either
            // side of the direction of travel.
            Point::new(
                tip.x - head.round() as i32,
                tip.y + (head * 0.5 * side).round() as i32,
            )
        };
        let (a, c) = (barb(1.0), barb(-1.0));
        canvas.draw_line(tip, a, fg);
        canvas.draw_line(tip, c, fg);
        canvas.draw_line(a, c, fg);
    }

    fn on_event(
        &mut self,
        event: &denise_ui::Event<'_>,
        ctx: &mut denise_ui::EventCtx<'_, Message>,
    ) -> denise_ui::Handled {
        use denise::{ElementState, InputEvent, PointerButton};
        use denise_ui::{Event, Handled};
        let Event::Input(InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position,
            ..
        }) = event
        else {
            return Handled::No;
        };
        let inside = ctx.bounds.contains(*position);
        match state {
            ElementState::Down if inside => {
                self.pressed = true;
                Handled::Yes
            }
            ElementState::Up => {
                let fire = self.pressed && inside;
                self.pressed = false;
                if fire {
                    ctx.emit(Message::Reload);
                    Handled::Yes
                } else {
                    Handled::No
                }
            }
            _ => Handled::No,
        }
    }

    fn accepts_pointer(&self) -> bool {
        true
    }
}

/// The regular face comes from `system-font`, like every example. The other
/// faces are proper browser business — HTML means *bold* — so they are
/// probed from the same directories by well-known names, and any face not
/// found falls back to regular: degraded, never broken.
fn load_fonts(ui: &mut Ui<Message>, font: Font) -> Fonts {
    let regular = match font {
        Some((_, source)) => ui.add_font(source),
        None => denise_text::FontId(0),
    };
    let mut fonts = Fonts::all(regular);
    if regular == denise_text::FontId(0) {
        // The built-in bitmap font has one weight; probing real files for
        // bold beside it would pair faces that share nothing.
        return fonts;
    }

    let faces: [(&[&str], &mut denise_text::FontId); 4] = [
        (
            &[
                "DejaVuSans-Bold.ttf",
                "LiberationSans-Bold.ttf",
                "NotoSans-Bold.ttf",
                "Arial Bold.ttf",
                "arialbd.ttf",
            ],
            &mut fonts.bold,
        ),
        (
            &[
                "DejaVuSans-Oblique.ttf",
                "LiberationSans-Italic.ttf",
                "NotoSans-Italic.ttf",
                "Arial Italic.ttf",
                "ariali.ttf",
            ],
            &mut fonts.italic,
        ),
        (
            &[
                "DejaVuSans-BoldOblique.ttf",
                "LiberationSans-BoldItalic.ttf",
                "NotoSans-BoldItalic.ttf",
                "Arial Bold Italic.ttf",
                "arialbi.ttf",
            ],
            &mut fonts.bold_italic,
        ),
        (
            &[
                "DejaVuSansMono.ttf",
                "LiberationMono-Regular.ttf",
                "NotoSansMono-Regular.ttf",
                "Courier New.ttf",
                "cour.ttf",
            ],
            &mut fonts.mono,
        ),
    ];
    for (names, slot) in faces {
        if let Some((name, bytes)) = find_face(names)
            && let Ok(source) = TrueTypeSource::from_bytes(&name, &bytes)
        {
            *slot = ui.add_font(Box::new(source));
        }
    }
    fonts
}

fn find_face(names: &[&str]) -> Option<(String, Vec<u8>)> {
    for dir in system_font::FONT_DIRS {
        if let Some(found) = find_in(std::path::Path::new(dir), names, 3) {
            return found.into();
        }
    }
    None
}

fn find_in(dir: &std::path::Path, names: &[&str], depth: u8) -> Option<(String, Vec<u8>)> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if let Some(file) = path.file_name().and_then(|n| n.to_str())
            && names.contains(&file)
            && let Ok(bytes) = std::fs::read(&path)
        {
            return Some((file.to_string(), bytes));
        }
    }
    if depth > 0 {
        for sub in subdirs {
            if let Some(found) = find_in(&sub, names, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// A typed address, forgivingly: no scheme means `https`, and a path that
/// exists on disk means the file itself — which is how the fixtures are
/// reached from the command line.
fn to_url(text: &str) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(text)
        && !url.cannot_be_a_base()
    {
        return Some(url);
    }
    if text.starts_with("about:") {
        return Url::parse(text).ok();
    }
    let path = std::path::Path::new(text);
    if path.exists()
        && let Ok(canonical) = path.canonicalize()
        && let Ok(url) = Url::from_file_path(canonical)
    {
        return Some(url);
    }
    Url::parse(&format!("https://{text}")).ok()
}

/// The same page, fragments aside.
fn same_document(a: &Url, b: &Url) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.set_fragment(None);
    b.set_fragment(None);
    a == b
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn error_page(reason: &str) -> String {
    format!(
        "<html><body><h1>Could not load the page</h1>\
         <p>{}</p>\
         <p>The address bar is still yours; so is <b>Back</b>.</p></body></html>",
        escape(reason)
    )
}

/// The page shown before any address is typed — rendered by the same
/// pipeline as everything after it, which makes it the smallest test the
/// browser runs on itself.
const WELCOME: &str = r#"<html><body>
<h1>Denise browses</h1>
<p>This is a web page, rendered by a UI toolkit for embedded panels.
Every heading, paragraph and list item on screen is a <b>Denise widget</b>;
the text engine wrapping this sentence is the one the widgets share.</p>
<h2>What works</h2>
<ul>
<li>Server-rendered pages, read the way <i>lynx</i> reads them, in proportional type</li>
<li>Links, history, and a scroll wheel</li>
<li>No JavaScript, on purpose &#8212; pages that need it will say very little</li>
</ul>
<h2>Try one</h2>
<p>Type an address above, or start with
<a href="https://example.com">example.com</a>.</p>
<hr>
<p><small>Part of the Denise examples. The kiosk build drives a bare panel
over DRM; this window is the same application.</small></p>
</body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn pump(app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let now = app.elapsed_ms();
            app.ui.tick(now);
            app.handle(now);
            if !app.loading() {
                return;
            }
            assert!(Instant::now() < deadline, "the fetch thread went quiet");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn fixture(name: &str) -> String {
        format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    /// The whole pipeline, no display: fetch a file, follow its relative
    /// link, come back. What a click does, minus the click.
    #[test]
    fn navigation_and_history_round_trip() {
        let mut app = App::new(
            Size::new(800, 600),
            1.0,
            None,
            Motion::None,
            Some(fixture("basic.html")),
        );
        pump(&mut app);
        let page = app.page.as_ref().expect("a page");
        assert!(page.links.len() >= 2, "the fixture has links");
        // Link 0 is the in-page fragment: a scroll, not a fetch.
        let anchor = page.links[0].clone().expect("the fragment resolved");
        assert_eq!(anchor.fragment(), Some("verse"));
        assert!(!app.history.can_back(), "one entry so far");
        app.on_message(Message::Navigate(0));
        assert!(!app.loading(), "no fetch for a fragment");
        assert!(
            app.ui.scroll(app.chrome.content).y > 0,
            "scrolled to the verse section"
        );
        assert!(app.history.can_back(), "the jump is a history entry");

        // Link 1 leaves the page for real.
        let target = app.page.as_ref().unwrap().links[1]
            .clone()
            .expect("the relative link resolved");
        assert!(target.as_str().ends_with("second.html"));
        app.on_message(Message::Navigate(1));
        pump(&mut app);
        let (_, here) = app.source.as_ref().expect("a source");
        assert!(here.as_str().ends_with("second.html"));
        assert!(app.history.can_back());

        app.on_message(Message::Back);
        pump(&mut app);
        let (_, here) = app.source.as_ref().expect("a source");
        // Back lands on the fragment entry the jump created.
        assert!(here.as_str().ends_with("basic.html#verse"));
        assert!(app.history.can_forward());
    }

    /// A page that cannot be fetched still ends as a page — the error page,
    /// through the same pipeline.
    #[test]
    fn an_unreachable_page_becomes_an_error_page() {
        let mut app = App::new(
            Size::new(800, 600),
            1.0,
            None,
            Motion::None,
            Some(fixture("does-not-exist.html")),
        );
        pump(&mut app);
        let (html, _) = app.source.as_ref().expect("a source");
        assert!(html.contains("Could not load"));
    }

    /// A whole form, submitted with its seeded values: the fields land in
    /// the preview page, serialised, radio group and select resolved to
    /// their chosen values.
    #[test]
    fn a_file_form_submits_into_a_preview() {
        let mut app = App::new(
            Size::new(900, 700),
            1.0,
            None,
            Motion::None,
            Some(fixture("form.html")),
        );
        pump(&mut app);
        {
            let page = app.page.as_ref().expect("a page");
            assert_eq!(page.forms.forms.len(), 1);
            assert!(!page.controls.is_empty(), "widgets were bound");
            assert_eq!(page.selects.len(), 1);
        }
        app.on_message(Message::SubmitForm(0));
        pump(&mut app);
        let (html, url) = app.source.as_ref().expect("a source");
        assert_eq!(url.as_str(), "about:submitted");
        for expected in [
            "who = seed",
            "keep = on",
            "size = medium",
            "colour = g",
            "notes = two words",
            "token = fixture-1",
        ] {
            assert!(html.contains(expected), "missing {expected:?} in {html}");
        }
        // The empty password still submits, as an empty value.
        assert!(html.contains("pw = "));
        assert!(app.history.can_back(), "the form is one Back away");
    }

    /// The same flow against a real search engine, typing included: the
    /// M4 demo, as a test. Ignored by default: it needs a network and
    /// someone else's uptime.
    ///
    /// ```text
    /// cargo test -p browser -- --ignored
    /// ```
    #[test]
    #[ignore = "talks to lite.duckduckgo.com"]
    fn a_real_search_round_trips() {
        let dir = std::env::temp_dir().join("denise-browser-test");
        std::fs::create_dir_all(&dir).unwrap();
        let form = dir.join("net-form.html");
        std::fs::write(
            &form,
            r#"<form action="https://lite.duckduckgo.com/lite/" method="get">
               <input name="q" value="raspberry pi">
               <input type="submit" value="Search"></form>"#,
        )
        .unwrap();
        let mut app = App::new(
            Size::new(900, 700),
            1.0,
            None,
            Motion::None,
            Some(form.to_string_lossy().into_owned()),
        );
        pump(&mut app);
        app.on_message(Message::SubmitForm(0));
        pump(&mut app);
        let (html, url) = app.source.as_ref().expect("a source");
        assert!(url.as_str().contains("q=raspberry+pi"), "went to {url}");
        assert!(
            html.to_ascii_lowercase().contains("raspberry"),
            "no results in a {}-byte page",
            html.len()
        );
        let page = app.page.as_ref().expect("a page");
        assert!(!page.links.is_empty(), "results are links");
    }

    #[test]
    fn addresses_are_read_forgivingly() {
        assert_eq!(
            to_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert!(to_url("  ").is_none());
        assert_eq!(to_url("https://a.example/x").unwrap().path(), "/x");
        let fixture = fixture("basic.html");
        assert_eq!(to_url(&fixture).unwrap().scheme(), "file");
    }
}
