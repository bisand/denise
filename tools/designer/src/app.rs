//! The designer: a toolbar, three panes and a status line.

use std::path::PathBuf;

use denise::{Radius, Rect, Role, Size, theme};
use denise_forms::{Handler, Payload, Picture, Wiring};
use denise_ui::widgets::{Button, Divider, Label, List, ListItem, Panel};
use denise_ui::{Anchors, Dock, NodeId, Ui};

use crate::document::Document;
use crate::settings::Settings;

/// What the designer's own widgets emit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Message {
    /// Start a new form.
    New,
    /// Open a file, through the platform's dialog.
    Open,
    /// Write the form back where it came from.
    Save,
    /// Write the form somewhere chosen.
    SaveAs,
    /// A widget picked in the palette.
    Palette(usize),
    /// A node picked in the outline.
    Outline(usize),
    /// A message the *form under design* emits.
    ///
    /// A designer cannot know an application's message type, so every name in an
    /// open form resolves to this one. Nothing fires it while the canvas is in
    /// design mode — the scrim over the form absorbs the press — and preview mode
    /// (#99) is where it starts meaning something.
    Inert,
}

/// Fixed extents. Everything else is docked, so the window resizes for free.
const TOOLBAR: i32 = 44;
const STATUS: i32 = 26;
const PALETTE_ROW: i32 = 24;
/// Twelve rows exactly, so the viewport never cuts one in half.
const PALETTE_ROWS: i32 = PALETTE_ROW * 12;
const OUTLINE_ROW: i32 = 22;
const HEADER: i32 = 22;
const GAP: i32 = 8;

/// The designer's own chrome: the nodes that outlive whatever form is open.
struct Chrome {
    title: NodeId,
    status: NodeId,
    /// The inspector column, which the inspector panel is rebuilt inside.
    right: NodeId,
    /// The palette's scrolling viewport.
    palette_view: NodeId,
    /// The outline's scrolling viewport.
    outline_view: NodeId,
    /// The outline's list, replaced whenever a form is opened.
    outline: NodeId,
    /// The inspector's panel, replaced whenever the selection changes.
    inspector: NodeId,
    /// The node the form is built under, replaced whenever a form is opened.
    stage: NodeId,
    /// The canvas's scrolling viewport, which the stage is centred in.
    canvas: NodeId,
}

/// Builds every pane, and docks them.
///
/// The whole layout is `Dock`: a toolbar along the top, a status line along the
/// bottom, two columns against the sides, and the canvas taking what is left. So
/// the window resizes correctly without this file doing arithmetic on a resize,
/// which is the thing anchoring and docking were added for.
fn build_chrome(ui: &mut Ui<Message>, settings: Settings) -> Chrome {
    let root = ui.root();

    let toolbar = ui
        .add(
            root,
            Panel::filled(Role::Base200),
            Rect::new(0, 0, 0, TOOLBAR),
        )
        .expect("root");
    ui.set_dock(toolbar, Some(Dock::Top));

    let mut x = GAP;
    for (text, message) in [
        ("New", Message::New),
        ("Open…", Message::Open),
        ("Save", Message::Save),
        ("Save as…", Message::SaveAs),
    ] {
        let width = 8 * text.chars().count() as i32 + 24;
        ui.add(
            toolbar,
            Button::new(text, message)
                .with_role(Role::Neutral)
                .with_size(13),
            Rect::new(x, GAP, width, TOOLBAR - GAP * 2),
        );
        x += width + 6;
    }
    let title = ui
        .add(
            toolbar,
            Label::new("Untitled")
                .with_size(13)
                .with_role(Role::BaseContent),
            Rect::new(x + GAP, GAP, 420, TOOLBAR - GAP * 2),
        )
        .expect("toolbar");

    let status_bar = ui
        .add(
            root,
            Panel::filled(Role::Base200),
            Rect::new(0, 0, 0, STATUS),
        )
        .expect("root");
    ui.set_dock(status_bar, Some(Dock::Bottom));
    let status = ui
        .add(
            status_bar,
            Label::new("").with_size(11).with_role(Role::Base300),
            Rect::new(GAP, 0, 4000, STATUS),
        )
        .expect("status bar");

    let left = ui
        .add(
            root,
            Panel::filled(Role::Base100),
            Rect::new(0, 0, settings.left, 0),
        )
        .expect("root");
    ui.set_dock(left, Some(Dock::Left));

    let right = ui
        .add(
            root,
            Panel::filled(Role::Base100),
            Rect::new(0, 0, settings.right, 0),
        )
        .expect("root");
    ui.set_dock(right, Some(Dock::Right));

    let canvas = ui
        .add(root, Panel::filled(Role::Base300), Rect::new(0, 0, 0, 0))
        .expect("root");
    ui.set_dock(canvas, Some(Dock::Fill));
    // Larger than the window is reachable rather than cropped.
    ui.set_scrollable(canvas, true);

    // The left column: a palette above a divider above an outline.
    let width = settings.left - GAP * 2;
    ui.add(
        left,
        Label::new("Palette").with_size(11).with_role(Role::Primary),
        Rect::new(GAP, GAP, width, HEADER),
    );
    let split = GAP + HEADER + PALETTE_ROWS + GAP;
    ui.add(left, Divider::new(), Rect::new(GAP, split, width, 8));
    ui.add(
        left,
        Label::new("Outline").with_size(11).with_role(Role::Primary),
        Rect::new(GAP, split + 12, width, HEADER),
    );
    // Both lists hold more than their pane: twenty-five widgets will not fit in
    // three hundred pixels, and a form names as many nodes as it likes. So each
    // is a list at its full height inside a viewport that scrolls, rather than a
    // list quietly cut off at the bottom.
    let palette_view = ui
        .add(
            left,
            Panel::filled(Role::Base100),
            Rect::new(GAP, GAP + HEADER, width, PALETTE_ROWS),
        )
        .expect("left");
    ui.set_scrollable(palette_view, true);

    let outline_top = split + 12 + HEADER;
    let outline_view = ui
        .add(
            left,
            Panel::filled(Role::Base100),
            Rect::new(GAP, outline_top, width, 240),
        )
        .expect("left");
    ui.set_scrollable(outline_view, true);
    // Held top and bottom, so the outline takes whatever height the window has.
    ui.set_anchors(outline_view, Anchors::new(true, true, true, true));

    let outline = ui
        .add(
            outline_view,
            List::<Message>::inert([] as [&str; 0]),
            Rect::new(0, 0, width, 0),
        )
        .expect("outline viewport");

    let inspector = ui
        .add(right, Panel::default(), Rect::new(0, 0, settings.right, 0))
        .expect("right");
    ui.set_dock(inspector, Some(Dock::Fill));

    // Replaced by the first `show_form`; a node has to exist for it to remove.
    let stage = ui
        .add(canvas, Panel::default(), Rect::new(0, 0, 1, 1))
        .expect("canvas");

    Chrome {
        title,
        status,
        right,
        palette_view,
        outline_view,
        outline,
        inspector,
        stage,
        canvas,
    }
}

/// The designer.
pub struct Designer {
    pub ui: Ui<Message>,
    chrome: Chrome,
    document: Document,
    settings: Settings,
    /// Every widget the toolkit ships, in the order the palette lists them.
    palette: Vec<&'static str>,
    /// The named nodes of the open form, in the order the outline lists them.
    outline: Vec<(String, NodeId)>,
    selected: Option<NodeId>,
    status: String,
    exit: bool,
}

impl Designer {
    /// Builds the designer's own tree.
    pub fn new(size: Size, _scale: f32, settings: Settings, document: Document) -> Self {
        let mut ui: Ui<Message> = Ui::new(size, theme::DARK);
        let chrome = build_chrome(&mut ui, settings);
        let palette: Vec<&'static str> = denise_ui::widgets::all().iter().map(|w| w.kind).collect();

        let mut designer = Self {
            ui,
            chrome,
            document,
            settings,
            palette,
            outline: Vec::new(),
            selected: None,
            status: String::new(),
            exit: false,
        };
        designer.fill_palette();
        designer.show_form();
        designer
    }

    pub fn exit_requested(&self) -> bool {
        self.exit
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Records a new window size, to be written out on the way to exiting.
    pub fn remember_size(&mut self, size: Size) {
        self.settings.width = size.width;
        self.settings.height = size.height;
    }

    /// Asks the loop to stop after this frame.
    pub fn request_exit(&mut self) {
        self.exit = true;
    }

    /// The names the outline is showing, for tests and for #97.
    pub fn outline_names(&self) -> impl Iterator<Item = &str> {
        self.outline.iter().map(|(name, _)| name.as_str())
    }

    /// The node the inspector is describing.
    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    fn fill_palette(&mut self) {
        let items: Vec<ListItem> = self
            .palette
            .iter()
            .map(|kind| ListItem::new(*kind))
            .collect();
        let rows = items.len() as i32;
        let list = List::new(items, Message::Palette).with_row_height(PALETTE_ROW);
        // Built once from the catalogue and never changed: a widget added to
        // `denise-ui` appears here without this file learning its name. Given its
        // full height inside the viewport, so the wheel reaches the rest.
        let width = self.settings.left - GAP * 2;
        self.ui.add(
            self.chrome.palette_view,
            list,
            Rect::new(0, 0, width, rows * PALETTE_ROW),
        );
    }

    /// Rebuilds the canvas from the open document.
    ///
    /// The old stage goes and a new one takes its place, because a form's tree is
    /// not something to reconcile: it is a file, and opening one is opening one.
    pub fn show_form(&mut self) {
        self.ui.remove(self.chrome.stage);
        self.selected = None;
        self.outline.clear();

        let size = self.document.form().size();
        // Centred in the viewport if it fits, and at the margin if it does not —
        // the canvas scrolls, so a form larger than the window is reachable
        // rather than cropped.
        // Bounds, not layout: the canvas is docked, so its `layout` is the
        // placeholder it was added with and its `bounds` is where it ended up.
        let view = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        let x = ((view.width - size.width as i32) / 2).max(GAP);
        let y = ((view.height - size.height as i32) / 2).max(GAP);

        let stage = self
            .ui
            .add(
                self.chrome.canvas,
                Panel::filled(self.document.form().background()),
                Rect::new(x, y, size.width as i32, size.height as i32),
            )
            .expect("the canvas is there");
        self.chrome.stage = stage;

        let mut wiring = Design {
            base: self.document.base(),
            missing: Vec::new(),
        };
        let outcome = self
            .document
            .form()
            .build(&mut self.ui, stage, &mut wiring)
            .map(|built| {
                let mut names: Vec<(String, NodeId)> = built
                    .names()
                    .map(|(name, id)| (name.to_string(), id))
                    .collect();
                // A stable order, since `Built` is a map. File order is what #97
                // wants and what a real outline will show.
                names.sort_by(|a, b| a.0.cmp(&b.0));
                names
            });

        match outcome {
            Ok(names) => {
                self.outline = names;
                let missing = wiring.missing.len();
                let extra = if missing == 0 {
                    String::new()
                } else {
                    format!(" — {missing} picture(s) could not be loaded")
                };
                self.status = format!(
                    "{} — {}x{}, {} named node(s){extra}",
                    self.document.label(),
                    size.width,
                    size.height,
                    self.outline_names().count()
                );
            }
            Err(error) => {
                self.status = format!("could not build this form: {error}");
            }
        }

        // The scrim: an invisible sheet over the form that absorbs every press
        // and leaves the focus alone. It is what makes the canvas *design mode*
        // rather than a live form, and preview mode (#99) is hiding it.
        let scrim = Panel {
            fill: None,
            border: None,
            border_width: 0,
            radius: Radius::Box,
            backdrop: true,
        };
        if let Some(id) = self.ui.add(
            self.chrome.canvas,
            scrim,
            Rect::new(x, y, size.width as i32, size.height as i32),
        ) {
            self.ui.set_z(id, 100);
        }

        self.refresh_outline();
        self.refresh_inspector();
        self.refresh_labels();
    }

    fn refresh_outline(&mut self) {
        let items: Vec<ListItem> = self
            .outline
            .iter()
            // The name alone. A trailing kind is what the eye wants and what the
            // pane has no room for: at this width the two run into each other,
            // and an outline that cannot be read is worse than one that says
            // less. The kind is a line down in the inspector.
            .map(|(name, _)| ListItem::new(name.as_str()))
            .collect();
        let rows = items.len() as i32;
        let list = List::new(items, Message::Outline).with_row_height(OUTLINE_ROW);
        let width = self.settings.left - GAP * 2;
        self.ui.remove(self.chrome.outline);
        self.chrome.outline = self
            .ui
            .add(
                self.chrome.outline_view,
                list,
                Rect::new(0, 0, width, rows * OUTLINE_ROW),
            )
            .expect("the outline viewport is there");
    }

    /// Redraws the inspector for whatever is selected.
    ///
    /// Read-only: the editors are #93. What it proves today is that the property
    /// descriptors from #85 reach a running application without the designer
    /// naming a single widget — every line here comes from the widget itself.
    fn refresh_inspector(&mut self) {
        // Rebuilt rather than updated: there is no state in it worth keeping, and
        // a selection change replaces every row anyway.
        self.ui.remove(self.chrome.inspector);
        let inspector = self
            .ui
            .add(
                self.chrome.right,
                Panel::default(),
                Rect::new(0, 0, self.settings.right, 0),
            )
            .expect("the right pane is there");
        self.ui.set_dock(inspector, Some(Dock::Fill));
        self.chrome.inspector = inspector;

        let width = self.settings.right - GAP * 2;
        let mut y = GAP;
        let mut line = |ui: &mut Ui<Message>, text: String, role: Role, size: u16, height: i32| {
            ui.add(
                inspector,
                Label::new(text).with_role(role).with_size(size),
                Rect::new(GAP, y, width, height),
            );
            y += height + 2;
        };

        let Some(id) = self.selected() else {
            line(
                &mut self.ui,
                String::from("Nothing selected"),
                Role::BaseContent,
                13,
                18,
            );
            line(
                &mut self.ui,
                String::from("Pick a node in the outline."),
                Role::Base300,
                11,
                16,
            );
            return;
        };

        let kind = self.ui.kind(id).unwrap_or("node");
        line(&mut self.ui, kind.to_string(), Role::Primary, 15, 20);

        if let Some(bounds) = self.ui.layout(id) {
            line(
                &mut self.ui,
                format!(
                    "x {} y {} w {} h {}",
                    bounds.x, bounds.y, bounds.width, bounds.height
                ),
                Role::Base300,
                11,
                16,
            );
        }

        // Straight from the widget's own descriptor. The designer has no table.
        let properties = self.ui.properties(id);
        for property in properties {
            let value = match self.ui.get_property(id, property.name) {
                Some(value) => format!("{value:?}"),
                None if property.is_settable() => String::from("—"),
                None => String::from("(supplied when built)"),
            };
            let role = if property.is_settable() {
                Role::BaseContent
            } else {
                Role::Base300
            };
            line(
                &mut self.ui,
                format!("{}   {value}", property.name),
                role,
                11,
                16,
            );
        }
    }

    fn refresh_labels(&mut self) {
        let title = self.document.label();
        if let Some(label) = self.ui.widget_mut::<Label>(self.chrome.title) {
            label.set_text(title);
        }
        let status = self.status.clone();
        if let Some(label) = self.ui.widget_mut::<Label>(self.chrome.status) {
            label.set_text(status);
        }
    }

    /// Acts on one of the designer's own messages.
    pub fn handle(&mut self, message: Message) {
        match message {
            Message::New => {
                self.document = Document::blank();
                self.show_form();
            }
            Message::Open => {
                if let Some(path) = pick_open() {
                    self.open(path);
                }
            }
            Message::Save => self.save(None),
            Message::SaveAs => {
                if let Some(path) = pick_save() {
                    self.save(Some(path));
                }
            }
            Message::Palette(index) => {
                // Placing one is #91. Saying which is picked is what a skeleton
                // can honestly do, and it proves the catalogue reaches the pane.
                let kind = self.palette.get(index).copied().unwrap_or("?");
                self.status = format!("`{kind}` — dropping one on the canvas is #91");
                self.refresh_labels();
            }
            Message::Outline(index) => {
                self.selected = self.outline.get(index).map(|(_, id)| *id);
                self.refresh_inspector();
            }
            Message::Inert => {}
        }
    }

    /// Opens a path, reporting a failure in the status line rather than exiting.
    pub fn open(&mut self, path: PathBuf) {
        match Document::open(&path) {
            Ok(document) => {
                self.document = document;
                self.show_form();
            }
            Err(error) => {
                self.status = error;
                self.refresh_labels();
            }
        }
    }

    fn save(&mut self, to: Option<PathBuf>) {
        self.status = match self.document.save(to) {
            Ok(()) => format!("saved {}", self.document.label()),
            Err(error) => error,
        };
        self.refresh_labels();
    }
}

fn pick_open() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Denise form", &["dform"])
        .set_title("Open a form")
        .pick_file()
}

fn pick_save() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Denise form", &["dform"])
        .set_title("Save the form as")
        .set_file_name("form.dform")
        .save_file()
}

/// What the designer supplies a form it did not write.
struct Design {
    base: PathBuf,
    missing: Vec<String>,
}

impl Wiring<Message> for Design {
    /// Every name resolves.
    ///
    /// A designer cannot know an application's message type, and a form that
    /// would not open because the designer had never heard of `on-press=greet`
    /// would be useless. So every name is accepted and every one means
    /// [`Message::Inert`]; the scrim over the canvas means none of them fires.
    fn message(&mut self, _name: &str, payload: Payload) -> Option<Handler<Message>> {
        Some(match payload {
            Payload::None => Handler::Plain(Message::Inert),
            Payload::Bool => Handler::Bool(|_| Message::Inert),
            Payload::Index => Handler::Index(|_| Message::Inert),
            Payload::Number => Handler::Number(|_| Message::Inert),
        })
    }

    fn asset(&mut self, path: &str) -> Option<Picture> {
        let full = self.base.join(path);
        let bytes = std::fs::read(&full).ok().or_else(|| {
            self.missing.push(path.to_string());
            None
        })?;
        match denise_image::decode(&bytes) {
            Ok(picture) => {
                let (pixels, size) = picture.into_parts();
                Some(Picture { pixels, size })
            }
            Err(_) => {
                self.missing.push(path.to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::{ElementState, InputEvent, Point, PointerButton};

    const WINDOW: Size = Size::new(1280, 800);

    fn repo(relative: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn designer_on(form: &str) -> Designer {
        let document = Document::open(repo(form)).expect("the form opens");
        Designer::new(WINDOW, 1.0, Settings::default(), document)
    }

    fn press(designer: &mut Designer, at: Point) {
        designer.ui.handle(&[
            InputEvent::PointerMoved { position: at },
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position: at,
                modifiers: Default::default(),
            },
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                position: at,
                modifiers: Default::default(),
            },
        ]);
    }

    #[test]
    fn the_panes_dock_and_the_canvas_takes_what_is_left() {
        let designer = designer_on("forms/hello.dform");
        let settings = Settings::default();
        let canvas = designer
            .ui
            .bounds(designer.chrome.canvas)
            .expect("a canvas");

        assert_eq!(canvas.x, settings.left, "the left column is not docked");
        assert_eq!(canvas.y, TOOLBAR, "the toolbar is not docked");
        assert_eq!(
            canvas.width,
            WINDOW.width as i32 - settings.left - settings.right
        );
        assert_eq!(canvas.height, WINDOW.height as i32 - TOOLBAR - STATUS);
    }

    #[test]
    fn a_resize_moves_the_panes_without_this_file_doing_arithmetic() {
        let mut designer = designer_on("forms/hello.dform");
        designer.ui.handle(&[InputEvent::SurfaceResized {
            size: Size::new(1600, 1000),
            scale_factor: 1.0,
        }]);

        let settings = Settings::default();
        let canvas = designer
            .ui
            .bounds(designer.chrome.canvas)
            .expect("a canvas");
        assert_eq!(canvas.width, 1600 - settings.left - settings.right);
        assert_eq!(canvas.height, 1000 - TOOLBAR - STATUS);

        // The outline's *viewport* is anchored top and bottom, so it grew with
        // the window. The list inside it is sized to its own rows, which is what
        // gives the viewport something to scroll.
        let view = designer
            .ui
            .bounds(designer.chrome.outline_view)
            .expect("outline viewport");
        assert!(
            view.height > 240,
            "the outline did not follow the window: {view:?}"
        );
    }

    #[test]
    fn the_palette_lists_every_widget_the_toolkit_ships() {
        let designer = designer_on("forms/hello.dform");
        assert_eq!(designer.palette.len(), denise_ui::widgets::all().len());
        // Named by the catalogue rather than by this file, which is the point:
        // a twenty-sixth widget appears here without the designer changing.
        assert!(designer.palette.contains(&"button"));
        assert!(designer.palette.contains(&"radial-progress"));
    }

    #[test]
    fn opening_a_form_fills_the_outline_with_the_nodes_it_named() {
        let designer = designer_on("forms/reference.dform");
        let names: Vec<&str> = designer.outline_names().collect();
        assert!(names.len() > 20, "only {} names: {names:?}", names.len());
        for expected in ["header", "sidebar", "records", "volume", "scrim"] {
            assert!(names.contains(&expected), "no `{expected}` in {names:?}");
        }
    }

    #[test]
    fn the_inspector_reports_a_selected_node_from_the_widgets_own_descriptor() {
        let mut designer = designer_on("forms/reference.dform");
        let index = designer
            .outline_names()
            .position(|name| name == "volume")
            .expect("the slider is named");
        designer.handle(Message::Outline(index));

        let selected = designer.selected().expect("something is selected");
        assert_eq!(designer.ui.kind(selected), Some("slider"));

        // The inspector's rows come from `Describe`, so a slider's `min`, `max`
        // and `value` are there without this file listing them.
        let properties = designer.ui.properties(selected);
        for expected in ["min", "max", "value", "role"] {
            assert!(
                properties.iter().any(|p| p.name == expected),
                "no `{expected}` among {:?}",
                properties.iter().map(|p| p.name).collect::<Vec<_>>()
            );
        }
        assert_eq!(
            designer.ui.get_property(selected, "value"),
            Some(denise_ui::widgets::Value::Float(70.0))
        );
    }

    #[test]
    fn the_scrim_keeps_the_form_from_behaving_while_it_is_being_designed() {
        let mut designer = designer_on("forms/reference.dform");
        let index = designer
            .outline_names()
            .position(|name| name == "retry")
            .expect("the retry button is named");
        designer.handle(Message::Outline(index));
        let button = designer.selected().expect("selected");
        let bounds = designer.ui.bounds(button).expect("laid out");
        let middle = Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);

        // A press right on a button in the form under design.
        designer
            .ui
            .handle(&[InputEvent::PointerMoved { position: middle }]);
        press(&mut designer, middle);

        let messages: Vec<Message> = designer.ui.drain_messages().collect();
        assert!(
            messages.is_empty(),
            "the form under design acted on a press: {messages:?}"
        );
        assert_eq!(
            designer.ui.focused(),
            None,
            "a press on the scrim must leave the focus alone"
        );
    }

    #[test]
    fn the_designers_own_buttons_do_still_work() {
        let mut designer = designer_on("forms/reference.dform");
        assert!(designer.outline_names().count() > 20);

        // `New` is the second-leftmost thing in the toolbar, and pressing it
        // replaces the document — so the scrim stops the *form*, not the chrome.
        press(&mut designer, Point::new(GAP + 20, TOOLBAR / 2));
        let messages: Vec<Message> = designer.ui.drain_messages().collect();
        assert_eq!(messages, vec![Message::New]);

        designer.handle(Message::New);
        assert_eq!(
            designer.outline_names().count(),
            0,
            "a blank form names nothing"
        );
    }

    #[test]
    fn open_then_save_is_byte_for_byte_what_was_opened() {
        // The round trip #88 is about, in the smallest form it can be asserted:
        // the designer holds the document, not a value taken from it, so a save
        // that changed nothing changes nothing.
        let source = std::fs::read(repo("forms/reference.dform")).expect("the reference form");
        let out = std::env::temp_dir().join("denise-designer-roundtrip.dform");
        let _ = std::fs::remove_file(&out);

        let mut designer = designer_on("forms/reference.dform");
        designer.document.save(Some(out.clone())).expect("saving");

        assert_eq!(
            std::fs::read(&out).expect("reading back"),
            source,
            "saving a form nobody edited must not change a byte of it"
        );
    }

    #[test]
    fn a_form_that_does_not_parse_is_reported_rather_than_fatal() {
        let bad = std::env::temp_dir().join("denise-designer-bad.dform");
        std::fs::write(&bad, "form \"B\" version=99 width=1 height=1\n").expect("writing");

        let mut designer = designer_on("forms/hello.dform");
        designer.open(bad);

        assert!(
            designer.status.contains("99"),
            "the status line should say why: {}",
            designer.status
        );
        // And the form that was open is still open.
        assert_eq!(designer.document.form().title(), "Hello");
    }
}
