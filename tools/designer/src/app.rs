//! The designer: a toolbar, three panes and a status line.

use std::path::PathBuf;

use denise::{
    ElementState, InputEvent, KeyCode, Point, PointerButton, Radius, Rect, Role, Size, theme,
};
use denise_forms::{Edit, Handler, Literal, Payload, Picture, Placed, Wiring};
use denise_ui::widgets::{
    Button, Divider, Label, List, ListItem, Panel, Property, PropertyKind, TextInput, Value,
    open_select,
};
use denise_ui::{Anchors, Dock, NodeId, Ui};

use crate::canvas::{Drag, Grip, Guide, place, snap, topmost};
use crate::document::Document;
use crate::history::History;
use crate::inspector::{Editor, Field, Inspector, show_value};
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
    /// Put the last edit back.
    Undo,
    /// Do again what was put back.
    Redo,
    /// Take an inspector row's property out of the file, so its default stands.
    Reset(usize),
    /// Open an inspector row's dropdown.
    ///
    /// A `Select` holds the message itself, so the row can name itself here.
    /// Choosing cannot: the open list reports through a `fn(usize) -> M`, which
    /// carries the option and not the row, so the designer remembers which
    /// dropdown it opened.
    OpenChoice(usize),
    /// An option chosen in whichever dropdown is open.
    Chose(usize),
    /// Find a picture for an inspector row, through the platform's dialog.
    Browse(usize),
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
/// Eleven rows exactly, so the viewport never cuts one in half.
const PALETTE_ROWS: i32 = PALETTE_ROW * 11;
/// The field that filters the palette.
const FILTER: i32 = 26;
/// How far a press has to travel before it is a drag rather than a click.
const THRESHOLD: i32 = 4;
const OUTLINE_ROW: i32 = 22;
const HEADER: i32 = 22;
const GAP: i32 = 8;

/// A widget on its way from the palette onto the canvas.
///
/// Two ways in, and they share most of their machinery: dragging one across, and
/// clicking the palette and then drawing a rectangle where it goes. A press on a
/// palette row is both until the pointer moves — which is what makes the two a
/// state machine rather than two code paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placing {
    /// Nothing is being placed.
    Idle,
    /// A press went down on a palette row and has not travelled yet.
    Pressed {
        /// What it was on.
        kind: &'static str,
        /// Where it went down.
        from: Point,
    },
    /// A ghost is following the pointer across to the canvas.
    Carrying {
        /// What is being carried.
        kind: &'static str,
        /// The outline drawn at the pointer.
        ghost: NodeId,
    },
    /// A palette row was clicked: the next drag on the canvas draws the
    /// rectangle, and Enter puts one down without drawing anything.
    Armed {
        /// What is armed.
        kind: &'static str,
    },
    /// Drawing the rectangle for an armed widget.
    Drawing {
        /// What is being drawn.
        kind: &'static str,
        /// The corner the pointer went down on.
        from: Point,
        /// The outline drawn between there and the pointer.
        ghost: NodeId,
    },
}

impl Placing {
    /// What is being placed, if anything is.
    const fn kind(self) -> Option<&'static str> {
        match self {
            Placing::Idle => None,
            Placing::Pressed { kind, .. }
            | Placing::Carrying { kind, .. }
            | Placing::Armed { kind }
            | Placing::Drawing { kind, .. } => Some(kind),
        }
    }

    /// The outline being drawn for it, if one is.
    const fn ghost(self) -> Option<NodeId> {
        match self {
            Placing::Carrying { ghost, .. } | Placing::Drawing { ghost, .. } => Some(ghost),
            _ => None,
        }
    }

    /// Whether the pointer is currently carrying or drawing something.
    const fn moving(self) -> bool {
        matches!(
            self,
            Placing::Pressed { .. } | Placing::Carrying { .. } | Placing::Drawing { .. }
        )
    }
}

/// The designer's own chrome: the nodes that outlive whatever form is open.
struct Chrome {
    title: NodeId,
    status: NodeId,
    /// The two buttons that go grey when there is nothing to put back.
    undo_button: NodeId,
    redo_button: NodeId,
    /// The palette's scrolling viewport.
    palette_view: NodeId,
    /// The field that filters the palette.
    filter: NodeId,
    /// The palette's list, replaced whenever the filter changes.
    palette: NodeId,
    /// The outline's scrolling viewport.
    outline_view: NodeId,
    /// The outline's list, replaced whenever a form is opened.
    outline: NodeId,
    /// The inspector's scrolling viewport. The pane inside it is replaced
    /// whenever the selection changes; this outlives every form.
    inspector_view: NodeId,
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
    let mut history_buttons = Vec::new();
    for (text, message) in [
        ("New", Message::New),
        ("Open…", Message::Open),
        ("Save", Message::Save),
        ("Save as…", Message::SaveAs),
        ("Undo", Message::Undo),
        ("Redo", Message::Redo),
    ] {
        let width = 8 * text.chars().count() as i32 + 24;
        let id = ui.add(
            toolbar,
            Button::new(text, message)
                .with_role(Role::Neutral)
                .with_size(13),
            Rect::new(x, GAP, width, TOOLBAR - GAP * 2),
        );
        if matches!(message, Message::Undo | Message::Redo)
            && let Some(id) = id
        {
            history_buttons.push(id);
        }
        x += width + 6;
    }
    let (undo_button, redo_button) = (history_buttons[0], history_buttons[1]);
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
    let split = GAP + HEADER + FILTER + PALETTE_ROWS + GAP;
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
    let filter = ui
        .add(
            left,
            TextInput::<Message>::new()
                .with_placeholder("filter")
                .with_size(12)
                .with_max_chars(32),
            Rect::new(GAP, GAP + HEADER, width, FILTER - 2),
        )
        .expect("left");
    let palette_view = ui
        .add(
            left,
            Panel::filled(Role::Base100),
            Rect::new(GAP, GAP + HEADER + FILTER, width, PALETTE_ROWS),
        )
        .expect("left");
    ui.set_scrollable(palette_view, true);
    // Replaced by the first `fill_palette`; a node has to exist for it to
    // remove.
    let palette = ui
        .add(palette_view, Panel::default(), Rect::new(0, 0, 1, 1))
        .expect("palette viewport");

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

    // A form node with twenty properties of its own and fourteen the tree owns
    // is taller than any pane, so the rows go in a viewport that scrolls.
    let inspector_view = ui
        .add(right, Panel::default(), Rect::new(0, 0, settings.right, 0))
        .expect("right");
    ui.set_dock(inspector_view, Some(Dock::Fill));
    ui.set_scrollable(inspector_view, true);

    // Replaced by the first `show_form`; a node has to exist for it to remove.
    let stage = ui
        .add(canvas, Panel::default(), Rect::new(0, 0, 1, 1))
        .expect("canvas");

    Chrome {
        title,
        status,
        undo_button,
        redo_button,
        palette_view,
        filter,
        palette,
        outline_view,
        outline,
        inspector_view,
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
    /// The ones the filter is letting through, as indices into `palette`.
    shown: Vec<usize>,
    /// What the filter field last held.
    filter: String,
    /// A widget on its way onto the canvas.
    placing: Placing,
    /// The named nodes of the open form, in the order the outline lists them.
    outline: Vec<(String, NodeId)>,
    selected: Option<NodeId>,
    /// Every node of the open form, and where in the file it came from.
    placed: Vec<Placed>,
    /// What is selected, by file path rather than by `NodeId`: a path survives a
    /// rebuild, and every edit rebuilds.
    selection: Vec<Vec<usize>>,
    drag: Option<Drag>,
    /// The selection outline, its handles and any alignment guides. Rebuilt
    /// whenever any of them moves, and removed before the form is.
    overlay: Vec<NodeId>,
    snapping: bool,
    grid: i32,
    /// The right pane, rebuilt whenever the selection changes.
    inspector: Option<Inspector>,
    /// The row whose dropdown is open, and the popup it opened.
    choosing: Option<(usize, NodeId)>,
    /// Whether the canvas is behind the file.
    ///
    /// Some edits cannot be shown by setting something on a widget — a property
    /// taken away has to come back as the widget's own default, and a message
    /// name is not a value any `Value` can carry. Those are written to the file
    /// and the form is rebuilt from it, which cannot happen mid-keystroke
    /// without taking the caret out of the field being typed in. So it happens
    /// when the caret leaves.
    stale: bool,
    history: History,
    /// Set when a close was refused because of unsaved work; a second ask goes
    /// through.
    warned: bool,
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
            shown: Vec::new(),
            filter: String::new(),
            placing: Placing::Idle,
            outline: Vec::new(),
            selected: None,
            placed: Vec::new(),
            selection: Vec::new(),
            drag: None,
            overlay: Vec::new(),
            snapping: true,
            grid: 4,
            inspector: None,
            choosing: None,
            stale: false,
            history: History::new(),
            warned: false,
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
    ///
    /// Unsaved work stops it the first time and says so. A modal would be the
    /// better question and needs a second window; asking twice is the honest
    /// version of it until then, and it is at least impossible to lose a form to
    /// one keystroke.
    pub fn request_exit(&mut self) {
        if self.history.is_dirty() && !self.warned {
            self.warned = true;
            self.status = format!(
                "{} has unsaved changes — Save, or ask again to discard them",
                self.document.label()
            );
            self.refresh_labels();
            return;
        }
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

    /// Redraws the palette for whatever the filter is letting through.
    ///
    /// Built from the catalogue and never from a list here: a widget added to
    /// `denise-ui` appears without this file learning its name. Given its full
    /// height inside the viewport, so the wheel reaches the rest.
    fn fill_palette(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        self.shown = self
            .palette
            .iter()
            .enumerate()
            .filter(|(_, kind)| needle.is_empty() || kind.contains(&needle))
            .map(|(index, _)| index)
            .collect();

        let items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|index| ListItem::new(self.palette[*index]))
            .collect();
        let rows = items.len().max(1) as i32;
        let armed = self
            .placing
            .kind()
            .and_then(|kind| self.shown.iter().position(|i| self.palette[*i] == kind));
        let list = List::new(items, Message::Palette)
            .with_row_height(PALETTE_ROW)
            .with_selected(armed);

        let width = self.settings.left - GAP * 2;
        self.ui.remove(self.chrome.palette);
        self.chrome.palette = self
            .ui
            .add(
                self.chrome.palette_view,
                list,
                Rect::new(0, 0, width, rows * PALETTE_ROW),
            )
            .expect("the palette viewport is there");
    }

    /// The kind a palette row stands for.
    fn palette_kind(&self, row: usize) -> Option<&'static str> {
        self.shown.get(row).map(|index| self.palette[*index])
    }

    /// Rebuilds the canvas from the open document.
    ///
    /// The old stage goes and a new one takes its place, because a form's tree is
    /// not something to reconcile: it is a file, and opening one is opening one.
    pub fn show_form(&mut self) {
        for id in std::mem::take(&mut self.overlay) {
            self.ui.remove(id);
        }
        self.ui.remove(self.chrome.stage);
        self.selected = None;
        self.outline.clear();
        self.placed.clear();
        self.drag = None;

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
                self.placed = built.placed().to_vec();
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

        // A form may ask for the caret with `focus=#true`, and a form under
        // design must not have it: the caret is behaviour, design mode is the
        // absence of behaviour, and the first thing typed belongs to the
        // inspector rather than to a field in the form being drawn.
        self.ui.focus(None);

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

        // The selection is held by path, so it survives a rebuild — but the
        // `NodeId` behind it does not, and neither does the overlay drawn on it.
        let surviving: Vec<Vec<usize>> = self
            .selection
            .iter()
            .filter(|path| self.node_id(path).is_some())
            .cloned()
            .collect();
        self.selection = surviving;
        self.selected = self.selection.last().and_then(|path| self.node_id(path));
        self.refresh_outline();
        self.refresh_inspector();
        self.refresh_overlay();
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
    /// Every row here comes from a descriptor — the widget's own for what the
    /// widget holds, and [`denise_forms::NODE_PROPERTIES`] for what the tree
    /// holds. This file names no widget and no property, which is what keeps a
    /// twenty-sixth widget from needing a line of it.
    fn refresh_inspector(&mut self) {
        // Rebuilt rather than reconciled: a selection change replaces every row,
        // and a row holds no state worth carrying across one.
        if let Some(inspector) = self.inspector.take() {
            self.ui.remove(inspector.content);
        }
        self.close_choice();
        let width = self.settings.right;
        let paths = self.selection.clone();
        let ids: Vec<NodeId> = paths.iter().filter_map(|path| self.node_id(path)).collect();

        if ids.is_empty() {
            let header = [
                (String::from("Nothing selected"), Role::BaseContent, 13),
                (
                    String::from("Pick a node on the canvas or in the outline."),
                    Role::Base300,
                    11,
                ),
            ];
            self.inspector = Some(Inspector::build(
                &mut self.ui,
                self.chrome.inspector_view,
                width,
                &header,
                &[],
            ));
            return;
        }

        let header = if ids.len() == 1 {
            let node = self.placed.iter().find(|p| p.path == paths[0]);
            let kind = node.map_or("node", |p| p.kind);
            let name = node
                .and_then(|p| p.name.clone())
                .unwrap_or_else(|| String::from("(this node has no name)"));
            [
                (kind.to_string(), Role::Primary, 15),
                // The node's own name, which is worth reading: it is what the
                // application will ask the form for.
                (name, Role::BaseContent, 11),
            ]
        } else {
            [
                (format!("{} selected", ids.len()), Role::Primary, 15),
                (
                    String::from("What they have in common; an edit goes to all of them."),
                    Role::Base300,
                    11,
                ),
            ]
        };

        let fields = self.fields(&paths, &ids);
        self.inspector = Some(Inspector::build(
            &mut self.ui,
            self.chrome.inspector_view,
            width,
            &header,
            &fields,
        ));
    }

    /// A row per property the selection has in common.
    ///
    /// With several selected that is the *intersection*, so an edit is never
    /// offered that only some of them could take.
    fn fields(&self, paths: &[Vec<usize>], ids: &[NodeId]) -> Vec<Field> {
        let used = self.message_names();
        let hint = (!used.is_empty()).then(|| format!("Used in this form: {}", used.join(", ")));

        let mut fields: Vec<Field> = denise_forms::NODE_PROPERTIES
            .iter()
            .map(|property| self.field(paths, ids, property, true, None))
            .collect();

        for property in self.ui.properties(ids[0]) {
            let shared = ids[1..].iter().all(|id| {
                self.ui
                    .properties(*id)
                    .iter()
                    .any(|p| p.name == property.name)
            });
            if !shared {
                continue;
            }
            // A message field cannot offer a dropdown — a name not used yet is
            // exactly what somebody is usually typing — so what the form
            // already uses goes in the tooltip instead.
            let hint = matches!(property.kind, PropertyKind::Message(_))
                .then(|| hint.clone())
                .flatten();
            fields.push(self.field(paths, ids, property, false, hint));
        }
        fields
    }

    /// One row: what to show in it, and whether the file wrote it.
    fn field(
        &self,
        paths: &[Vec<usize>],
        ids: &[NodeId],
        property: &'static Property,
        node: bool,
        hint: Option<String>,
    ) -> Field {
        let mut value: Option<String> = None;
        let mut agreed = true;
        let mut written = true;
        let mut resettable = true;

        for (path, id) in paths.iter().zip(ids) {
            let mine = self.value_of(path, *id, property, node);
            let entry = self.document.form().property(path, property.name).is_some();
            resettable &= entry;
            written &= entry || self.argument_for(path, *id, property, node).is_some();
            match &value {
                None => value = Some(mine),
                Some(seen) if *seen != mine => agreed = false,
                Some(_) => {}
            }
        }

        Field {
            property,
            node,
            // Several nodes disagreeing is an empty editor rather than one of
            // their values, which would be a lie about the other.
            value: agreed.then_some(value).flatten(),
            written,
            resettable,
            hint,
        }
    }

    /// What one node's property currently is, as a field shows it.
    fn value_of(&self, path: &[usize], id: NodeId, property: &Property, node: bool) -> String {
        if node {
            // The rectangle comes from the tree rather than the file, so the
            // four fields follow a drag on the canvas as it happens.
            if let Some(axis) = axis_of(property.name) {
                let rect = self.ui.layout(id).unwrap_or(Rect::ZERO);
                return [rect.x, rect.y, rect.width, rect.height][axis].to_string();
            }
            return self
                .document
                .form()
                .property(path, property.name)
                .unwrap_or_else(|| String::from(node_default(property.name)));
        }
        match property.kind {
            // The widget holds neither: a message is a value of the
            // application's own type and an asset is decoded pixels. The file
            // is where both are written, so the file is what the row shows.
            PropertyKind::Message(_) | PropertyKind::Asset => self
                .document
                .form()
                .property(path, property.name)
                .unwrap_or_default(),
            _ => self
                .ui
                .get_property(id, property.name)
                .map(|value| show_value(&value))
                .unwrap_or_default(),
        }
    }

    /// The node's positional argument, when *that* is what supplies a property.
    ///
    /// `label "Heading"` carries its text as an argument, which is how every
    /// form in this repo is written. Editing that text has to change the
    /// argument: adding `text="…"` beside it would leave the file saying one
    /// thing and the screen showing another.
    fn argument_for(
        &self,
        path: &[usize],
        id: NodeId,
        property: &Property,
        node: bool,
    ) -> Option<String> {
        if node
            || !matches!(property.kind, PropertyKind::Text)
            || self.document.form().property(path, property.name).is_some()
        {
            return None;
        }
        let argument = self.document.form().argument(path)?;
        // Only when the widget agrees that this is where its value came from,
        // which is exact: nothing else could have set it.
        let held = self.ui.get_property(id, property.name)?;
        (show_value(&held) == argument).then_some(argument)
    }

    /// Every message name this form already uses.
    fn message_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for placed in &self.placed {
            for property in self.ui.properties(placed.id) {
                if !matches!(property.kind, PropertyKind::Message(_)) {
                    continue;
                }
                if let Some(name) = self.document.form().property(&placed.path, property.name)
                    && !names.contains(&name)
                {
                    names.push(name);
                }
            }
        }
        names.sort();
        names
    }

    fn refresh_labels(&mut self) {
        // A button that cannot do anything says so, rather than doing nothing.
        let (can_undo, can_redo) = (self.history.can_undo(), self.history.can_redo());
        self.ui.set_enabled(self.chrome.undo_button, can_undo);
        self.ui.set_enabled(self.chrome.redo_button, can_redo);

        let title = self.document.label();
        if let Some(label) = self.ui.widget_mut::<Label>(self.chrome.title) {
            label.set_text(title);
        }
        let status = self.status.clone();
        if let Some(label) = self.ui.widget_mut::<Label>(self.chrome.status) {
            label.set_text(status);
        }
    }

    // -------------------------------------------------------------- inspector

    /// Applies whatever has been typed, ticked or dragged in the inspector.
    ///
    /// Called once a frame, after the tree has seen the events. Nothing in the
    /// pane emits a message per keystroke — this toolkit's widgets take
    /// `fn(T) -> M` function pointers, which cannot carry a row index — so the
    /// pane is asked what it holds instead, and a difference from what it was
    /// given is somebody having edited it.
    pub fn poll(&mut self) {
        // The filter is read the same way and for the same reason: a
        // `TextInput` reports nothing as it is typed into, so it is asked.
        let filter = self
            .ui
            .widget::<TextInput<Message>>(self.chrome.filter)
            .map(|field| field.text().to_string())
            .unwrap_or_default();
        if filter != self.filter {
            self.filter = filter;
            self.fill_palette();
        }

        let Some(mut inspector) = self.inspector.take() else {
            return;
        };
        let changed = inspector.changed(&self.ui);
        self.inspector = Some(inspector);
        for (row, text) in changed {
            self.commit(row, text);
        }
        self.settle();
    }

    /// Rebuilds the canvas from the file, once the caret is out of the way.
    ///
    /// See [`Designer::stale`] for what is waiting and why it has to.
    fn settle(&mut self) {
        if !self.stale {
            return;
        }
        let typing = self.ui.focused().is_some_and(|id| {
            self.inspector.as_ref().is_some_and(|pane| {
                pane.rows.iter().any(|row| {
                    matches!(row.editor, Editor::Field(_) | Editor::Slid { .. })
                        && row.editor.focusable() == id
                })
            })
        });
        if typing {
            return;
        }
        self.stale = false;
        self.reload_from_document();
    }

    /// Writes one inspector row to every selected node.
    ///
    /// The canvas follows as it is typed, through the same `set` the engine
    /// calls when it loads a form — so the inspector cannot show something the
    /// engine could not load, and a value the widget refuses is reported and
    /// not written.
    fn commit(&mut self, row: usize, text: String) {
        let Some((property, node, deferred)) = self.inspector.as_ref().and_then(|pane| {
            let row = pane.rows.get(row)?;
            Some((
                row.property,
                row.node,
                matches!(row.editor, Editor::Field(_) | Editor::Slid { .. }),
            ))
        }) else {
            return;
        };

        let paths = self.selection.clone();
        if paths.is_empty() {
            return;
        }

        // An empty editor means "no value", and no value is what a default is:
        // the schema does not write one, so clearing the field takes the
        // property out of the file.
        let Ok(written) = (!text.trim().is_empty())
            .then(|| self.interpret(property, &text))
            .transpose()
        else {
            return;
        };

        let mut live = true;
        let mut edits: Vec<Edit> = Vec::new();
        for path in &paths {
            let Some(id) = self.node_id(path) else {
                continue;
            };
            match &written {
                None => {
                    edits.push(Edit::property(path, property.name, None));
                    live = false;
                }
                Some((literal, value)) => {
                    // The node's own argument, where that is what supplies this
                    // property. See `argument_for`.
                    if self.argument_for(path, id, property, node).is_some() {
                        edits.push(Edit::Argument {
                            path: path.clone(),
                            value: literal.clone(),
                        });
                    } else {
                        edits.push(Edit::property(path, property.name, Some(literal.clone())));
                    }
                    match value {
                        Some(value) if !node => {
                            if let Some(Err(refused)) =
                                self.ui.set_property(id, property.name, value.clone())
                            {
                                self.complain(&refused.to_string());
                                return;
                            }
                        }
                        // The tree owns its geometry through typed calls rather
                        // than a property bag, and `x` is the one worth wiring
                        // by hand: a node that did not move as its `x` was typed
                        // would be a strange thing to look at.
                        _ if node => live &= self.place(id, property.name, &text),
                        _ => live = false,
                    }
                }
            }
        }

        self.complain("");
        match edits.len() {
            0 => return,
            1 => self.edit(edits.remove(0)),
            _ => self.edit(Edit::Many(edits)),
        }

        if live {
            self.refresh_overlay();
            return;
        }
        // Deferred for a field, because rebuilding would take the caret out of
        // it; at once for a box or a dropdown, where there is no caret to lose.
        if deferred {
            self.stale = true;
        } else {
            self.reload_from_document();
        }
    }

    /// Puts a tree-owned property on the node, reporting whether it could.
    ///
    /// Only the rectangle. Everything else the tree owns — docking, anchoring,
    /// stacking, the name — changes where *other* nodes go as well, so it is
    /// written to the file and the form is rebuilt from it, which is the only
    /// thing that gets all of them right.
    fn place(&mut self, id: NodeId, name: &str, text: &str) -> bool {
        let (Some(axis), Ok(value)) = (axis_of(name), text.parse::<i32>()) else {
            return false;
        };
        let Some(was) = self.ui.layout(id) else {
            return false;
        };
        let mut axes = [was.x, was.y, was.width, was.height];
        axes[axis] = value;
        self.ui
            .set_layout(id, Rect::new(axes[0], axes[1], axes[2], axes[3]));
        true
    }

    /// What a row's text means, as the file writes it and as the widget takes
    /// it.
    ///
    /// The second half is `None` for a message and for an asset, which no
    /// `Value` can carry, and for everything the tree owns.
    #[allow(clippy::type_complexity)]
    fn interpret(
        &mut self,
        property: &Property,
        text: &str,
    ) -> Result<(Literal, Option<Value>), ()> {
        let refuse = |designer: &mut Self, why: String| {
            designer.complain(&why);
            Err(())
        };
        match property.kind {
            PropertyKind::Text | PropertyKind::Color => {
                Ok((Literal::text(text), Some(Value::text(text))))
            }
            PropertyKind::Bool => {
                let flag = text == "#true";
                Ok((Literal::Flag(flag), Some(Value::Bool(flag))))
            }
            PropertyKind::Int { .. } => match text.trim().parse::<i64>() {
                Ok(number) => Ok((
                    Literal::Int(number),
                    Some(Value::Int(
                        number.clamp(i32::MIN.into(), i32::MAX.into()) as i32
                    )),
                )),
                Err(_) => refuse(self, format!("`{}` takes a whole number", property.name)),
            },
            PropertyKind::Float { .. } => match text.trim().parse::<f64>() {
                // Written the way it was typed: somebody who types `70` into a
                // float means `value=70`, and a file that answered `value=70.0`
                // would be editing a line they did not.
                Ok(number) => Ok((
                    if number.fract() == 0.0 && number.abs() < 1e15 {
                        Literal::Int(number as i64)
                    } else {
                        Literal::Float(number)
                    },
                    Some(Value::Float(number as f32)),
                )),
                Err(_) => refuse(self, format!("`{}` takes a number", property.name)),
            },
            PropertyKind::Enum(names) => match names.iter().find(|name| **name == text) {
                Some(name) => Ok((Literal::name(*name), Some(Value::Enum(name)))),
                None => refuse(
                    self,
                    format!("`{}` is one of: {}", property.name, names.join(", ")),
                ),
            },
            // A message name is written bare, as `on-press=save`; a path is a
            // string. Neither is something a widget can be handed.
            PropertyKind::Message(_) => Ok((Literal::name(text), None)),
            PropertyKind::Asset => Ok((Literal::text(text), None)),
            _ => refuse(
                self,
                format!("`{}` is a kind this pane cannot edit yet", property.name),
            ),
        }
    }

    /// Says why a value was refused, in the pane and in the status line.
    fn complain(&mut self, why: &str) {
        if let Some(mut pane) = self.inspector.take() {
            pane.complain(&mut self.ui, why);
            self.inspector = Some(pane);
        }
        if !why.is_empty() {
            self.status = why.to_string();
            self.refresh_labels();
        }
    }

    /// Puts the rectangle on screen into the four fields that show it.
    ///
    /// A drag writes to the tree and not to the file until the button comes up,
    /// so the pane is told directly rather than rebuilt — and told in a way that
    /// does not count as somebody having typed it.
    fn sync_rect(&mut self) {
        let (Some(mut pane), Some(id)) = (self.inspector.take(), self.selected()) else {
            return;
        };
        if let Some(rect) = self.ui.layout(id) {
            let axes = [rect.x, rect.y, rect.width, rect.height];
            for index in 0..pane.rows.len() {
                let name = pane.rows[index].property.name;
                if pane.rows[index].node
                    && let Some(axis) = axis_of(name)
                {
                    pane.show(&mut self.ui, index, axes[axis].to_string());
                }
            }
        }
        self.inspector = Some(pane);
    }

    /// Closes an open dropdown, if one is open.
    fn close_choice(&mut self) {
        if self.choosing.take().is_some() {
            self.ui.close_popup();
        }
    }

    // ------------------------------------------------------------ design mode

    /// Reads the events before the tree does, and hands back what the tree
    /// should still see.
    ///
    /// Everything over the canvas is design mode's: a press there selects,
    /// drags or resizes, and the tree never learns of it. Everything else — the
    /// toolbar, the palette, the outline — passes through untouched, which is
    /// why the panes go on working while the form does not.
    pub fn input(&mut self, events: &[InputEvent]) -> Vec<InputEvent> {
        let mut forward = Vec::with_capacity(events.len());
        for event in events {
            if !self.claim(event) {
                forward.push(event.clone());
            }
        }
        forward
    }

    /// Whether design mode took this event.
    fn claim(&mut self, event: &InputEvent) -> bool {
        let canvas = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        let palette = self
            .ui
            .bounds(self.chrome.palette_view)
            .unwrap_or(Rect::ZERO);

        match event {
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Down,
                position,
                modifiers,
            } => {
                // The palette's own presses are design mode's: a row has to be
                // able to start a drag, and a `List` that saw the press would
                // have selected on it and swallowed the rest.
                if palette.contains(*position) {
                    self.press_palette(*position);
                    return true;
                }
                if !canvas.contains(*position) {
                    // A press anywhere else — the inspector, the toolbar —
                    // gives up whatever the palette had armed.
                    self.cancel_placing();
                    return false;
                }
                if let Placing::Armed { kind } = self.placing {
                    self.begin_drawing(kind, *position);
                    return true;
                }
                self.press(*position, modifiers.contains(denise::Modifiers::SHIFT));
                true
            }
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ElementState::Up,
                position,
                ..
            } => {
                if self.placing.moving() {
                    self.drop_at(*position);
                    return true;
                }
                if canvas.contains(*position) || self.drag.is_some() {
                    self.release();
                    return true;
                }
                false
            }
            InputEvent::PointerMoved { position } => {
                if self.placing.moving() {
                    self.carry_to(*position);
                    return true;
                }
                if self.drag.is_some() {
                    self.drag_to(*position);
                    return true;
                }
                false
            }
            InputEvent::Key {
                code,
                state: ElementState::Down,
                modifiers,
                ..
            } => self.key(*code, *modifiers),
            _ => false,
        }
    }

    // --------------------------------------------------------------- placing

    /// A press on a palette row: a click until the pointer travels.
    fn press_palette(&mut self, at: Point) {
        let view = self
            .ui
            .bounds(self.chrome.palette_view)
            .unwrap_or(Rect::ZERO);
        let scroll = self.ui.scroll(self.chrome.palette_view);
        let row = (at.y - view.y + scroll.y) / PALETTE_ROW;
        let Some(kind) = usize::try_from(row).ok().and_then(|r| self.palette_kind(r)) else {
            self.cancel_placing();
            return;
        };
        self.placing = Placing::Pressed { kind, from: at };
    }

    /// The pointer moved while something was being placed.
    fn carry_to(&mut self, at: Point) {
        match self.placing {
            Placing::Pressed { kind, from } => {
                if (at.x - from.x).abs() + (at.y - from.y).abs() < THRESHOLD {
                    return;
                }
                let size = denise_forms::default_size(kind);
                let rect = Rect::new(at.x, at.y, size.width as i32, size.height as i32);
                let Some(ghost) = self.add_ghost(kind, rect) else {
                    return;
                };
                self.placing = Placing::Carrying { kind, ghost };
            }
            Placing::Carrying { kind, ghost } => {
                let size = denise_forms::default_size(kind);
                let rect = Rect::new(at.x, at.y, size.width as i32, size.height as i32);
                self.ui.set_layout(ghost, self.to_client(rect));
            }
            Placing::Drawing { from, ghost, .. } => {
                let rect = self.to_client(between(from, at));
                self.ui.set_layout(ghost, rect);
            }
            Placing::Idle | Placing::Armed { .. } => {}
        }
    }

    /// A press on the canvas with a palette row armed: the WinForms way.
    fn begin_drawing(&mut self, kind: &'static str, at: Point) {
        let Some(ghost) = self.add_ghost(kind, Rect::new(at.x, at.y, 0, 0)) else {
            return;
        };
        self.placing = Placing::Drawing {
            kind,
            from: at,
            ghost,
        };
    }

    /// The pointer came up on whatever was being placed.
    fn drop_at(&mut self, at: Point) {
        let placing = std::mem::replace(&mut self.placing, Placing::Idle);
        if let Some(ghost) = placing.ghost() {
            self.ui.remove(ghost);
        }
        match placing {
            // Never travelled, so it was a click: arm it, and the next drag on
            // the canvas draws where it goes.
            Placing::Pressed { kind, .. } => {
                self.placing = Placing::Armed { kind };
                self.status = format!(
                    "`{kind}` — drag a rectangle on the canvas, or press Enter to put one down"
                );
                self.fill_palette();
                self.refresh_labels();
            }
            Placing::Carrying { kind, .. } => {
                let size = denise_forms::default_size(kind);
                self.insert_widget(
                    kind,
                    Rect::new(at.x, at.y, size.width as i32, size.height as i32),
                );
            }
            Placing::Drawing { kind, from, .. } => {
                let drawn = between(from, at);
                // A press that never travelled is not a rectangle; it means
                // "one of these, here, at whatever size it usually is".
                let size = denise_forms::default_size(kind);
                let rect = if drawn.width < THRESHOLD || drawn.height < THRESHOLD {
                    Rect::new(from.x, from.y, size.width as i32, size.height as i32)
                } else {
                    drawn
                };
                self.insert_widget(kind, rect);
            }
            Placing::Idle | Placing::Armed { .. } => {}
        }
    }

    /// Gives up whatever was armed or being carried.
    fn cancel_placing(&mut self) {
        let placing = std::mem::replace(&mut self.placing, Placing::Idle);
        if let Some(ghost) = placing.ghost() {
            self.ui.remove(ghost);
        }
        if placing != Placing::Idle {
            self.fill_palette();
        }
    }

    /// The outline that follows the pointer while something is being placed.
    ///
    /// On the **root**, above every pane, because a drag that starts in the
    /// palette and ends on the canvas crosses two subtrees and a ghost inside
    /// either would be clipped by it.
    fn add_ghost(&mut self, kind: &'static str, rect: Rect) -> Option<NodeId> {
        let outline = Panel {
            fill: None,
            border: Some(Role::Accent),
            border_width: 1,
            radius: Radius::Box,
            backdrop: false,
        };
        let root = self.ui.root();
        let ghost = self.ui.add(root, outline, self.to_client(rect))?;
        self.ui.set_z(ghost, 1000);
        self.ui.add(
            ghost,
            Label::new(kind).with_size(11).with_role(Role::Accent),
            Rect::new(2, 2, 200, 14),
        );
        Some(ghost)
    }

    /// A screen rectangle in the coordinates a child of the root is placed in.
    ///
    /// Docking leaves a **client area** — what is left once the toolbar, the
    /// status line and the two columns have taken their edges — and a child of
    /// the root that is not itself docked is placed inside that, which is the
    /// same rule WinForms follows. The `Dock::Fill` canvas *is* that area, so
    /// its origin is the offset.
    fn to_client(&self, rect: Rect) -> Rect {
        let client = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        Rect::new(
            rect.x - client.x,
            rect.y - client.y,
            rect.width,
            rect.height,
        )
    }

    /// Puts a new widget in the form, at a rectangle in screen coordinates.
    ///
    /// The parent is whatever container the top-left corner landed in, and the
    /// rectangle written to the file is relative to it — which is the only space
    /// a form file knows.
    pub fn insert_widget(&mut self, kind: &'static str, screen: Rect) {
        let stage = self.ui.bounds(self.chrome.stage).unwrap_or(Rect::ZERO);
        let corner = Point::new(screen.x, screen.y);
        if !stage.contains(corner) {
            self.status = format!("`{kind}` goes on the form; that is beside it");
            self.refresh_labels();
            return;
        }

        let parent = self.container_at(corner);
        let origin = parent
            .as_ref()
            .and_then(|path| self.path_bounds(path))
            .unwrap_or(stage);
        let mut rect = Rect::new(
            screen.x - origin.x,
            screen.y - origin.y,
            screen.width,
            screen.height,
        );
        if self.snapping {
            rect = snap(rect, self.grid);
        }

        let parent = parent.unwrap_or_default();
        let index = self.child_count(&parent);
        let text = denise_forms::seed(kind, rect);

        self.history.separate();
        self.edit(Edit::Insert {
            parent: parent.clone(),
            index,
            text,
        });

        // Selected the moment it lands, so the inspector is describing the thing
        // that was just put down. The path survives the rebuild; the `NodeId`
        // would not.
        let mut path = parent;
        path.push(index);
        self.selection = vec![path];
        self.reload_from_document();
        // One click, one widget: the row stops being armed once it has been
        // put down, which is what a hand expects and what stops a stray press
        // on the canvas placing a second one.
        self.fill_palette();
        self.status = format!("placed a `{kind}`");
        self.refresh_labels();
    }

    /// The deepest node under a point that can hold children of its own.
    ///
    /// `None` for the form itself. What counts as a container is
    /// [`denise_forms::owns_children`] and not a list here: a `select` holds
    /// options and a `table` holds columns, and dropping a button on either has
    /// missed.
    fn container_at(&self, at: Point) -> Option<Vec<usize>> {
        let containers: Vec<Placed> = self
            .placed
            .iter()
            .filter(|node| denise_forms::owns_children(node.kind))
            .cloned()
            .collect();
        topmost(
            &containers,
            |p| {
                self.ui
                    .bounds(p.id)
                    .map(|r| (r, self.ui.visible(p.id), self.ui.z(p.id)))
            },
            at,
        )
        .map(|node| node.path.clone())
    }

    /// How many children a node has, so a new one goes after them.
    fn child_count(&self, parent: &[usize]) -> usize {
        self.placed
            .iter()
            .filter(|node| node.path.len() == parent.len() + 1 && node.path.starts_with(parent))
            .count()
    }

    /// Puts an armed widget down without drawing a rectangle for it.
    ///
    /// For the keyboard, and for anybody who wants one of something at whatever
    /// size it usually is. Steps down and across until it is not exactly on top
    /// of something already there, so pressing Enter twice gives two widgets
    /// rather than one hiding another.
    pub fn place_armed(&mut self) {
        let Placing::Armed { kind } = self.placing else {
            return;
        };
        let stage = self.ui.bounds(self.chrome.stage).unwrap_or(Rect::ZERO);
        let size = denise_forms::default_size(kind);
        let step = self.grid.max(1) * 2;
        let mut at = Point::new(stage.x + step, stage.y + step);
        while self
            .placed
            .iter()
            .filter_map(|node| self.ui.bounds(node.id))
            .any(|bounds| bounds.x == at.x && bounds.y == at.y)
        {
            at = Point::new(at.x + step, at.y + step);
            if !stage.contains(at) {
                at = Point::new(stage.x + step, stage.y + step);
                break;
            }
        }
        self.insert_widget(
            kind,
            Rect::new(at.x, at.y, size.width as i32, size.height as i32),
        );
    }

    fn press(&mut self, at: Point, add: bool) {
        // A handle on what is already selected comes first: its handles stick
        // out past the node, and a press on one means resize rather than
        // "select whatever is under there".
        if let Some(path) = self.selection.last().cloned()
            && let Some(bounds) = self.path_bounds(&path)
            && let Some(grip) = Grip::at(bounds, at)
            && matches!(grip, Grip::Resize { .. })
        {
            self.begin(grip, at, path);
            return;
        }

        let hit = topmost(
            &self.placed,
            |p| {
                self.ui
                    .bounds(p.id)
                    .map(|r| (r, self.ui.visible(p.id), self.ui.z(p.id)))
            },
            at,
        )
        .map(|p| p.path.clone());
        let Some(path) = hit else {
            if !add {
                self.selection.clear();
                self.selected = None;
                self.refresh_inspector();
                self.refresh_overlay();
            }
            return;
        };

        if add {
            if let Some(already) = self.selection.iter().position(|p| *p == path) {
                self.selection.remove(already);
            } else {
                self.selection.push(path.clone());
            }
        } else if !self.selection.contains(&path) {
            self.selection = vec![path.clone()];
        }
        self.selected = self.node_id(&path);
        // Working on something else ends the run, as leaving a field would.
        self.history.separate();
        self.refresh_inspector();

        if let Some(bounds) = self.path_bounds(&path)
            && let Some(grip) = Grip::at(bounds, at)
        {
            self.begin(grip, at, path);
        }
        self.refresh_overlay();
    }

    fn begin(&mut self, grip: Grip, at: Point, path: Vec<usize>) {
        // A drag is its own step, whatever was being nudged before it.
        self.history.separate();
        let Some(origin) = self.node_id(&path).and_then(|id| self.ui.layout(id)) else {
            return;
        };
        self.drag = Some(Drag {
            grip,
            from: at,
            origin,
            path,
            moved: false,
        });
    }

    fn drag_to(&mut self, to: Point) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if to == drag.from {
            return;
        }
        drag.moved = true;
        let drag = drag.clone();
        let Some(id) = self.node_id(&drag.path) else {
            return;
        };

        let siblings = self.siblings_of(&drag.path);
        let placement = place(&drag, to, &siblings, self.grid, self.snapping);

        // The tree moves so the person can see it. The *file* does not, until
        // the button comes up: one drag is one edit, which is what keeps a move
        // to a one-line diff and will make it one undo step.
        self.ui.set_layout(id, placement.rect);
        self.refresh_overlay_with(&placement.guides, &drag.path);
        self.sync_rect();
        self.status = format!(
            "{} {},{} {}x{}",
            self.placed
                .iter()
                .find(|p| p.path == drag.path)
                .map_or("node", |p| p.kind),
            placement.rect.x,
            placement.rect.y,
            placement.rect.width,
            placement.rect.height
        );
        self.refresh_labels();
    }

    fn release(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if !drag.moved {
            // A press that never moved was a selection, and a selection must not
            // touch the file.
            self.refresh_overlay();
            return;
        }
        let Some(rect) = self.node_id(&drag.path).and_then(|id| self.ui.layout(id)) else {
            return;
        };
        self.write_rect(&drag.path, rect, drag.origin);
        self.refresh_overlay();
    }

    /// Applies an edit through the history, so it can be undone.
    ///
    /// The one door: nothing else in this crate touches the document, which is
    /// what makes "undo works for everything" a property of the code rather than
    /// a promise about remembering.
    fn edit(&mut self, edit: Edit) {
        match self.document.form_mut().apply(edit) {
            Ok(inverse) => {
                self.history.record(inverse);
                self.document.set_dirty(self.history.is_dirty());
            }
            Err(error) => self.status = error.to_string(),
        }
        self.refresh_labels();
    }

    /// Writes a node's rectangle back to the document, and only what changed.
    ///
    /// One edit for the whole rectangle, so a drag that moved *and* resized is
    /// one step. A single property on its own stays a single `Number`, which is
    /// what lets a run of nudges coalesce.
    fn write_rect(&mut self, path: &[usize], rect: Rect, was: Rect) {
        let mut edits: Vec<Edit> = [
            ("x", rect.x, was.x),
            ("y", rect.y, was.y),
            ("w", rect.width, was.width),
            ("h", rect.height, was.height),
        ]
        .into_iter()
        .filter(|(_, new, old)| new != old)
        .map(|(name, new, _)| Edit::number(path, name, Some(i64::from(new))))
        .collect();

        match edits.len() {
            0 => {}
            1 => self.edit(edits.remove(0)),
            _ => self.edit(Edit::Many(edits)),
        }
    }

    /// Undoes one step, and rebuilds the canvas from what is left.
    pub fn undo(&mut self) {
        let outcome = self.history.undo(self.document.form_mut());
        self.after_history(outcome, "nothing left to undo");
    }

    /// Redoes one step.
    pub fn redo(&mut self) {
        let outcome = self.history.redo(self.document.form_mut());
        self.after_history(outcome, "nothing to redo");
    }

    fn after_history(&mut self, outcome: Result<bool, denise_forms::Error>, empty: &str) {
        match outcome {
            Ok(true) => {
                self.document.set_dirty(self.history.is_dirty());
                let (undone, redoable) = self.history.depth();
                self.reload_from_document();
                self.status = format!("{undone} to undo, {redoable} to redo");
                self.refresh_labels();
            }
            Ok(false) => {
                self.status = empty.to_string();
                self.refresh_labels();
            }
            Err(error) => {
                self.status = error.to_string();
                self.refresh_labels();
            }
        }
    }

    fn key(&mut self, code: KeyCode, modifiers: denise::Modifiers) -> bool {
        let shift = modifiers.contains(denise::Modifiers::SHIFT);
        // Control everywhere, and Command as well, which is what a hand on a Mac
        // reaches for. Either counts, so one binding serves all three platforms.
        let command = modifiers.contains(denise::Modifiers::CTRL)
            || modifiers.contains(denise::Modifiers::SUPER);
        let step = if shift { self.grid * 2 } else { 1 };
        let nudge = match code {
            KeyCode::ArrowLeft => Some((-step, 0)),
            KeyCode::ArrowRight => Some((step, 0)),
            KeyCode::ArrowUp => Some((0, -step)),
            KeyCode::ArrowDown => Some((0, step)),
            _ => None,
        };
        if let Some((dx, dy)) = nudge {
            if self.selection.is_empty() {
                return false;
            }
            self.nudge(dx, dy);
            return true;
        }
        // A key that would also be a keystroke is only design mode's while
        // nothing in the designer's own chrome has the caret. Nothing does yet;
        // the inspector's editors (#93) are what make this matter. Ctrl-Z is not
        // among them: undo is undo wherever the caret is.
        let typing = self.ui.focused().is_some();
        // Ctrl on every platform, and Command as well on a Mac, where it is what
        // the fingers reach for.
        if command && matches!(code, KeyCode::Z) {
            if shift {
                self.redo();
            } else {
                self.undo();
            }
            return true;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter
                if matches!(self.placing, Placing::Armed { .. }) =>
            {
                self.place_armed();
                true
            }
            KeyCode::Escape if self.placing != Placing::Idle => {
                self.cancel_placing();
                self.status = String::from("nothing armed");
                self.refresh_labels();
                true
            }
            KeyCode::G if !typing => {
                self.toggle_snapping();
                true
            }
            KeyCode::Escape if !self.selection.is_empty() => {
                self.selection.clear();
                self.selected = None;
                self.refresh_inspector();
                self.refresh_overlay();
                true
            }
            KeyCode::Delete | KeyCode::Backspace if !self.selection.is_empty() && !typing => {
                self.delete_selection();
                true
            }
            KeyCode::Tab => {
                self.cycle(shift);
                true
            }
            _ => false,
        }
    }

    /// Moves the selection by whole pixels, writing each step to the file.
    ///
    /// A nudge is deliberately not a drag: there is no release to commit on, so
    /// each press of the key is its own edit.
    pub fn nudge(&mut self, dx: i32, dy: i32) {
        for path in self.selection.clone() {
            let Some(id) = self.node_id(&path) else {
                continue;
            };
            let Some(was) = self.ui.layout(id) else {
                continue;
            };
            let rect = Rect::new(was.x + dx, was.y + dy, was.width, was.height);
            self.ui.set_layout(id, rect);
            self.write_rect(&path, rect, was);
        }
        self.refresh_overlay();
        self.sync_rect();
    }

    /// Takes the selection out of the form.
    ///
    /// Deepest path first, so removing one does not shift the index of another
    /// still to go.
    pub fn delete_selection(&mut self) {
        let mut paths = self.selection.clone();
        // Deepest and last first, so removing one does not shift the index of
        // another still to go.
        paths.sort();
        paths.reverse();
        self.history.separate();
        self.edit(Edit::Many(paths.iter().map(|p| Edit::remove(p)).collect()));
        self.selection.clear();
        self.reload_from_document();
    }

    /// Selects the next node in file order.
    pub fn cycle(&mut self, backwards: bool) {
        if self.placed.is_empty() {
            return;
        }
        let current = self
            .selection
            .last()
            .and_then(|path| self.placed.iter().position(|p| p.path == *path));
        let next = match (current, backwards) {
            (Some(i), false) => (i + 1) % self.placed.len(),
            (Some(i), true) => (i + self.placed.len() - 1) % self.placed.len(),
            (None, true) => self.placed.len() - 1,
            (None, false) => 0,
        };
        self.selection = vec![self.placed[next].path.clone()];
        self.selected = Some(self.placed[next].id);
        self.refresh_inspector();
        self.refresh_overlay();
    }

    /// Rebuilds the canvas from the document, which an edit has changed.
    fn reload_from_document(&mut self) {
        match self.document.reparse() {
            Ok(()) => self.show_form(),
            Err(error) => {
                self.status = format!("the edit made a form that will not load: {error}");
                self.refresh_labels();
            }
        }
    }

    /// Turns snapping on or off, and says which it is now.
    pub fn toggle_snapping(&mut self) {
        self.snapping = !self.snapping;
        self.status = if self.snapping {
            format!("snapping to a {}px grid and to siblings", self.grid)
        } else {
            String::from("snapping off")
        };
        self.refresh_labels();
    }

    /// Picks a widget up from the palette and holds it over the form.
    ///
    /// For `--snapshot`, which has no pointer: a ghost exists only while a drag
    /// is happening, so a picture of one has to be taken mid-flight. Nothing is
    /// placed, because nothing was let go of.
    pub fn carry(&mut self, kind: &str, over: Point) -> bool {
        let Some(kind) = self.palette.iter().find(|name| **name == kind).copied() else {
            return false;
        };
        let stage = self.ui.bounds(self.chrome.stage).unwrap_or(Rect::ZERO);
        let from = self
            .ui
            .bounds(self.chrome.palette_view)
            .map_or(Point::new(0, 0), |view| {
                Point::new(view.x + view.width / 2, view.y + PALETTE_ROW / 2)
            });
        self.placing = Placing::Pressed { kind, from };
        self.carry_to(Point::new(stage.x + over.x, stage.y + over.y));
        matches!(self.placing, Placing::Carrying { .. })
    }

    /// Selects a node by the name the form gave it.
    ///
    /// For `--snapshot`, which has no pointer: a picture of a designer with
    /// nothing selected shows none of what a designer does.
    pub fn select_named(&mut self, name: &str) -> bool {
        let Some(path) = self
            .placed
            .iter()
            .find(|p| p.name.as_deref() == Some(name))
            .map(|p| p.path.clone())
        else {
            return false;
        };
        self.selection = vec![path.clone()];
        self.selected = self.node_id(&path);
        self.refresh_inspector();
        self.refresh_overlay();
        true
    }

    /// Takes hold of the selection and drags it, without letting go.
    ///
    /// Also for `--snapshot`: the alignment guides exist only while a drag is
    /// happening, so a picture of them has to be taken mid-drag. Nothing is
    /// committed, because nothing was released.
    pub fn drag_selection(&mut self, dx: i32, dy: i32) {
        let Some(path) = self.selection.last().cloned() else {
            return;
        };
        let Some(bounds) = self.path_bounds(&path) else {
            return;
        };
        let from = Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
        self.press(from, false);
        self.drag_to(Point::new(from.x + dx, from.y + dy));
    }

    fn node_id(&self, path: &[usize]) -> Option<NodeId> {
        self.placed.iter().find(|p| p.path == path).map(|p| p.id)
    }

    fn path_bounds(&self, path: &[usize]) -> Option<Rect> {
        self.node_id(path).and_then(|id| self.ui.bounds(id))
    }

    /// The layouts of everything sharing a node's parent, in the same space as
    /// its own — which is what an alignment guide is measured against.
    fn siblings_of(&self, path: &[usize]) -> Vec<Rect> {
        let Some((_, parent)) = path.split_last() else {
            return Vec::new();
        };
        self.placed
            .iter()
            .filter(|p| p.path.len() == path.len() && p.path.starts_with(parent) && p.path != path)
            .filter_map(|p| self.ui.layout(p.id))
            .collect()
    }

    fn refresh_overlay(&mut self) {
        self.refresh_overlay_with(&[], &[]);
    }

    /// Draws the selection: an outline, eight handles, a name tag, and any
    /// alignment guides.
    ///
    /// Above the form rather than in it, so nothing a form contains has to know
    /// it is being designed — the overlay is the designer's, drawn in the
    /// designer's colours, and removed when the form is.
    fn refresh_overlay_with(&mut self, guides: &[Guide], dragging: &[usize]) {
        for id in std::mem::take(&mut self.overlay) {
            self.ui.remove(id);
        }
        let canvas = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        let scroll = self.ui.scroll(self.chrome.canvas);
        let to_canvas = |r: Rect| {
            Rect::new(
                r.x - canvas.x + scroll.x,
                r.y - canvas.y + scroll.y,
                r.width,
                r.height,
            )
        };

        let mut added: Vec<NodeId> = Vec::new();
        let mut add = |ui: &mut Ui<Message>, rect: Rect, panel: Panel, z: i32| {
            if let Some(id) = ui.add(self.chrome.canvas, panel, to_canvas(rect)) {
                ui.set_z(id, z);
                added.push(id);
            }
        };

        // The guides first, so a handle is never hidden under one.
        if let Some(parent) = self
            .placed
            .iter()
            .find(|p| p.path == dragging)
            .and_then(|p| p.parent)
            .and_then(|id| self.ui.bounds(id))
        {
            for guide in guides {
                let line = if guide.vertical {
                    Rect::new(parent.x + guide.at, parent.y, 1, parent.height)
                } else {
                    Rect::new(parent.x, parent.y + guide.at, parent.width, 1)
                };
                add(&mut self.ui, line, Panel::filled(Role::Accent), 190);
            }
        }

        let selection = self.selection.clone();
        for (index, path) in selection.iter().enumerate() {
            let Some(bounds) = self.path_bounds(path) else {
                continue;
            };
            let outline = Panel {
                fill: None,
                border: Some(Role::Primary),
                border_width: 1,
                radius: Radius::Box,
                backdrop: false,
            };
            add(&mut self.ui, bounds, outline, 200);

            // Handles on the last one only: with several selected they would be
            // a thicket, and only one of them could be resized anyway.
            if index + 1 == selection.len() {
                for corner in Grip::HANDLES {
                    add(
                        &mut self.ui,
                        Grip::handle_rect(bounds, corner),
                        Panel::filled(Role::Primary),
                        210,
                    );
                }
            }
        }
        self.overlay = added;

        // The name tag goes last: it is a label rather than a panel, and it says
        // what is selected without the eye going to the inspector.
        if let Some(path) = self.selection.last()
            && let Some(bounds) = self.path_bounds(path)
            && let Some(node) = self.placed.iter().find(|p| p.path == *path)
        {
            let text = node.name.as_deref().map_or_else(
                || node.kind.to_string(),
                |name| format!("{} {name}", node.kind),
            );
            let tag = Rect::new(bounds.x, bounds.y - 15, 200, 14);
            if let Some(id) = self.ui.add(
                self.chrome.canvas,
                Label::new(text).with_size(10).with_role(Role::Primary),
                to_canvas(tag),
            ) {
                self.ui.set_z(id, 220);
                self.overlay.push(id);
            }
        }
    }

    /// Acts on one of the designer's own messages.
    pub fn handle(&mut self, message: Message) {
        match message {
            Message::New => {
                self.document = Document::blank();
                self.history = History::new();
                self.warned = false;
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
            Message::Palette(row) => {
                // Only reachable from the keyboard: design mode takes the
                // palette's presses so that a row can start a drag. Arming it
                // is what a click means, and this is the same thing.
                if let Some(kind) = self.palette_kind(row) {
                    self.placing = Placing::Armed { kind };
                    self.status = format!(
                        "`{kind}` — drag a rectangle on the canvas, or press Enter to put one down"
                    );
                    self.fill_palette();
                    self.refresh_labels();
                }
            }
            Message::Outline(index) => {
                // By path, like every other way of selecting: the inspector and
                // the overlay both work from the selection rather than from a
                // `NodeId`, because a path survives the rebuild an edit causes.
                let Some(id) = self.outline.get(index).map(|(_, id)| *id) else {
                    return;
                };
                let Some(path) = self
                    .placed
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.path.clone())
                else {
                    return;
                };
                self.history.separate();
                self.selection = vec![path];
                self.selected = Some(id);
                self.refresh_inspector();
                self.refresh_overlay();
            }
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            Message::Reset(row) => self.reset(row),
            Message::OpenChoice(row) => self.open_choice(row),
            Message::Chose(option) => self.chose(option),
            Message::Browse(row) => self.browse(row),
            Message::Inert => {}
        }
    }

    /// Takes a property out of the file, so the widget's own default stands.
    pub fn reset(&mut self, row: usize) {
        let Some(name) = self
            .inspector
            .as_ref()
            .and_then(|pane| pane.rows.get(row))
            .map(|row| row.property.name)
        else {
            return;
        };
        let edits: Vec<Edit> = self
            .selection
            .iter()
            .map(|path| Edit::property(path, name, None))
            .collect();
        if edits.is_empty() {
            return;
        }
        self.history.separate();
        self.edit(Edit::Many(edits));
        // A default is only knowable by building the widget again.
        self.reload_from_document();
    }

    /// Drops a row's list of names open.
    pub fn open_choice(&mut self, row: usize) {
        self.close_choice();
        let Some(select) = self
            .inspector
            .as_ref()
            .and_then(|pane| pane.rows.get(row))
            .map(|row| row.editor.focusable())
        else {
            return;
        };
        if let Some(popup) = open_select(&mut self.ui, select, Message::Chose) {
            self.choosing = Some((row, popup));
        }
    }

    /// Applies an option chosen in whichever dropdown is open.
    ///
    /// The row's `Select` is set rather than the property written: the next
    /// poll sees the dropdown holding something new and commits it, which is
    /// the same path a typed value takes.
    pub fn chose(&mut self, option: usize) {
        let Some((row, _)) = self.choosing else {
            return;
        };
        self.close_choice();
        let Some(pane) = self.inspector.take() else {
            return;
        };
        if let Some(Editor::Choice { select, options }) = pane.rows.get(row).map(|r| &r.editor) {
            let (select, chosen) = (*select, options.get(option).copied());
            if let (Some(widget), Some(_)) = (
                self.ui
                    .widget_mut::<denise_ui::widgets::Select<Message>>(select),
                chosen,
            ) {
                widget.set_selected(Some(option));
            }
        }
        self.inspector = Some(pane);
    }

    /// Finds a picture for an asset row, through the platform's dialog.
    ///
    /// The path written is relative to the form file, because that is what the
    /// engine resolves it against — a form carried to another machine with its
    /// pictures beside it goes on working.
    pub fn browse(&mut self, row: usize) {
        let Some(chosen) = pick_picture() else {
            return;
        };
        let base = self.document.base();
        let relative = chosen
            .strip_prefix(&base)
            .unwrap_or(&chosen)
            .to_string_lossy()
            .into_owned();
        let Some(mut pane) = self.inspector.take() else {
            return;
        };
        pane.show(&mut self.ui, row, String::new());
        self.inspector = Some(pane);
        self.commit(row, relative);
        // A field was written to, so nothing will settle on its own.
        self.stale = false;
        self.reload_from_document();
    }

    /// Opens a path, reporting a failure in the status line rather than exiting.
    pub fn open(&mut self, path: PathBuf) {
        match Document::open(&path) {
            Ok(document) => {
                self.document = document;
                self.history = History::new();
                self.warned = false;
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
            Ok(()) => {
                self.history.saved();
                self.document.set_dirty(false);
                self.warned = false;
                format!("saved {}", self.document.label())
            }
            Err(error) => error,
        };
        self.refresh_labels();
    }
}

/// The rectangle between two corners, whichever way round they were given.
fn between(from: Point, to: Point) -> Rect {
    Rect::new(
        from.x.min(to.x),
        from.y.min(to.y),
        (to.x - from.x).abs(),
        (to.y - from.y).abs(),
    )
}

/// Which of a rectangle's four numbers a tree-owned property is, if it is one.
const fn axis_of(name: &str) -> Option<usize> {
    Some(match name.as_bytes() {
        b"x" => 0,
        b"y" => 1,
        b"w" => 2,
        b"h" => 3,
        _ => return None,
    })
}

/// What a tree-owned property means when the file does not write it.
///
/// The widget-owned ones report their own defaults through `Describe::get`.
/// These have nobody to ask: the tree applies them from the file and a file
/// that says nothing means the tree was never told.
const fn node_default(name: &str) -> &'static str {
    match name.as_bytes() {
        b"visible" | b"enabled" => "#true",
        b"scroll" | b"focus" => "#false",
        b"z" => "0",
        // `name`, `tooltip`, `stack`, `anchor`, `dock`: nothing at all.
        _ => "",
    }
}

fn pick_open() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Denise form", &["dform"])
        .set_title("Open a form")
        .pick_file()
}

/// What stands in for a picture that would not load.
///
/// A checkerboard, which is what every tool that has ever had to draw "there
/// is supposed to be an image here" draws. Sixteen pixels, scaled by whatever
/// rectangle the form gave it.
fn missing_picture() -> Picture {
    const LIGHT: u32 = 0xFF45_475A;
    const DARK: u32 = 0xFF1E_1E2E;
    let mut pixels = Vec::with_capacity(16 * 16);
    for y in 0..16 {
        for x in 0..16 {
            pixels.push(if (x / 8 + y / 8) % 2 == 0 {
                LIGHT
            } else {
                DARK
            });
        }
    }
    Picture {
        pixels,
        size: Size::new(16, 16),
    }
}

fn pick_picture() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Picture", &["png", "jpg", "jpeg", "gif"])
        .set_title("Find a picture")
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

    /// Loads a picture, and draws a hole rather than refusing the form.
    ///
    /// Answering `None` fails the *whole build*: a picture is not optional to an
    /// `Image`, so a path that has moved would mean a form the designer could
    /// not open at all. A designer is exactly where a missing picture has to be
    /// survivable — it is usually being pointed at a file that does not exist
    /// yet — so what comes back is a placeholder, and the status line says how
    /// many.
    fn asset(&mut self, path: &str) -> Option<Picture> {
        let full = self.base.join(path);
        let decoded = std::fs::read(&full)
            .ok()
            .and_then(|bytes| denise_image::decode(&bytes).ok());
        match decoded {
            Some(picture) => {
                let (pixels, size) = picture.into_parts();
                Some(Picture { pixels, size })
            }
            None => {
                self.missing.push(path.to_string());
                Some(missing_picture())
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

    /// Drives the designer the way the event loop does: design mode first, and
    /// the tree gets what is left.
    fn feed(designer: &mut Designer, events: &[InputEvent]) {
        let rest = designer.input(events);
        designer.ui.handle(&rest);
    }

    fn button(state: ElementState, at: Point) -> InputEvent {
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position: at,
            modifiers: Default::default(),
        }
    }

    fn click_at(designer: &mut Designer, at: Point, shift: bool) {
        let modifiers = if shift {
            denise::Modifiers::SHIFT
        } else {
            denise::Modifiers::NONE
        };
        feed(
            designer,
            &[
                InputEvent::PointerMoved { position: at },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Down,
                    position: at,
                    modifiers,
                },
                button(ElementState::Up, at),
            ],
        );
    }

    fn drag_from_to(designer: &mut Designer, from: Point, to: Point) {
        feed(
            designer,
            &[
                InputEvent::PointerMoved { position: from },
                button(ElementState::Down, from),
                InputEvent::PointerMoved { position: to },
                button(ElementState::Up, to),
            ],
        );
    }

    fn press_key(designer: &mut Designer, code: KeyCode, shift: bool) {
        press_with(
            designer,
            code,
            if shift {
                denise::Modifiers::SHIFT
            } else {
                denise::Modifiers::NONE
            },
        );
    }

    fn press_with(designer: &mut Designer, code: KeyCode, modifiers: denise::Modifiers) {
        feed(
            designer,
            &[InputEvent::Key {
                code,
                state: ElementState::Down,
                repeat: false,
                modifiers,
            }],
        );
    }

    fn middle(designer: &Designer, path: &[usize]) -> Point {
        let b = designer.path_bounds(path).expect("laid out");
        Point::new(b.x + b.width / 2, b.y + b.height / 2)
    }

    fn path_named(designer: &Designer, name: &str) -> Vec<usize> {
        designer
            .placed
            .iter()
            .find(|p| p.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no node named `{name}`"))
            .path
            .clone()
    }

    fn text(designer: &Designer) -> String {
        designer.document.form().text()
    }

    /// The lines that differ, which must be the same count on both sides.
    fn diff(before: &str, after: &str) -> Vec<String> {
        let a: Vec<&str> = before.lines().collect();
        let b: Vec<&str> = after.lines().collect();
        assert_eq!(a.len(), b.len(), "an edit changed the number of lines");
        a.iter()
            .zip(&b)
            .filter(|(x, y)| x != y)
            .map(|(_, y)| (*y).to_string())
            .collect()
    }

    // --------------------------------------------------------------- selection

    #[test]
    fn a_click_selects_a_label_even_though_a_running_form_would_not() {
        // The whole reason design mode hit-tests for itself: a `Label` answers
        // `false` to `accepts_pointer`, so the tree would send this press
        // straight past it.
        let mut designer = designer_on("forms/reference.dform");
        let label = designer
            .placed
            .iter()
            .find(|p| p.kind == "label" && p.name.is_none())
            .expect("the reference form has unnamed labels")
            .path
            .clone();
        let at = middle(&designer, &label);

        click_at(&mut designer, at, false);
        assert_eq!(designer.selection, vec![label]);
        assert_eq!(
            designer.ui.kind(designer.selected().unwrap()),
            Some("label")
        );
    }

    #[test]
    fn an_invisible_node_is_not_what_a_click_finds() {
        // `reference.dform` ends with a full-surface `scrim` that is
        // `visible=#false`. It is the last node in the file and the highest `z`,
        // so it is on top of everything — and clicking the canvas must find what
        // can actually be seen.
        let mut designer = designer_on("forms/reference.dform");
        let scrim = path_named(&designer, "scrim");
        assert!(!designer.ui.visible(designer.node_id(&scrim).unwrap()));

        let at = middle(&designer, &path_named(&designer, "notify"));
        click_at(&mut designer, at, false);
        assert_ne!(designer.selection, vec![scrim]);
        assert_eq!(
            designer.ui.kind(designer.selected().unwrap()),
            Some("checkbox")
        );
    }

    #[test]
    fn shift_adds_to_the_selection_and_takes_away_again() {
        let mut designer = designer_on("forms/reference.dform");
        let first = path_named(&designer, "notify");
        let second = path_named(&designer, "dark");

        let at_first = middle(&designer, &first);
        click_at(&mut designer, at_first, false);
        let at_second = middle(&designer, &second);
        click_at(&mut designer, at_second, true);
        assert_eq!(designer.selection.len(), 2, "{:?}", designer.selection);

        click_at(&mut designer, at_second, true);
        assert_eq!(designer.selection, vec![first]);
    }

    #[test]
    fn escape_clears_and_tab_walks_the_form_in_file_order() {
        let mut designer = designer_on("forms/reference.dform");
        let at = middle(&designer, &path_named(&designer, "notify"));
        click_at(&mut designer, at, false);
        assert!(!designer.selection.is_empty());

        press_key(&mut designer, KeyCode::Escape, false);
        assert!(designer.selection.is_empty());
        assert!(designer.selected().is_none());

        press_key(&mut designer, KeyCode::Tab, false);
        assert_eq!(designer.selection, vec![designer.placed[0].path.clone()]);
        press_key(&mut designer, KeyCode::Tab, false);
        assert_eq!(designer.selection, vec![designer.placed[1].path.clone()]);
        press_key(&mut designer, KeyCode::Tab, true);
        assert_eq!(designer.selection, vec![designer.placed[0].path.clone()]);
    }

    #[test]
    fn a_selection_draws_an_outline_eight_handles_and_a_name_tag() {
        let mut designer = designer_on("forms/reference.dform");
        assert!(
            designer.overlay.is_empty(),
            "nothing selected, nothing drawn"
        );

        let at = middle(&designer, &path_named(&designer, "notify"));
        click_at(&mut designer, at, false);
        // One outline, eight handles, one name tag.
        assert_eq!(designer.overlay.len(), 10, "{:?}", designer.overlay.len());

        press_key(&mut designer, KeyCode::Escape, false);
        assert!(
            designer.overlay.is_empty(),
            "the overlay outlived the selection"
        );
    }

    // ------------------------------------------------------------------ edits

    #[test]
    fn dragging_a_node_inside_a_panel_is_a_one_line_diff() {
        let mut designer = designer_on("forms/reference.dform");
        designer.toggle_snapping(); // off, so the arithmetic is the test's
        let slider = path_named(&designer, "volume");
        assert!(slider.len() > 1, "the slider is meant to be inside a panel");

        let before = text(&designer);
        let from = middle(&designer, &slider);
        drag_from_to(&mut designer, from, Point::new(from.x + 24, from.y + 8));

        let after = text(&designer);
        let changed = diff(&before, &after);
        assert_eq!(
            changed.len(),
            1,
            "a drag touched {} lines: {changed:#?}",
            changed.len()
        );
        // Relative to the panel it sits in, which is the only space the file
        // knows: the slider was at x=140 y=388, and the drag was +24 +8.
        assert!(changed[0].contains("x=164"), "{}", changed[0]);
        assert!(changed[0].contains("y=396"), "{}", changed[0]);
        // Everything else on the line came along.
        assert!(
            changed[0].contains("on-change=set-volume"),
            "{}",
            changed[0]
        );
        assert!(changed[0].contains("min=0 max=100"), "{}", changed[0]);
    }

    #[test]
    fn a_press_that_never_moved_leaves_the_file_exactly_as_it_was() {
        let mut designer = designer_on("forms/reference.dform");
        let before = text(&designer);
        let at = middle(&designer, &path_named(&designer, "volume"));
        click_at(&mut designer, at, false);
        assert!(!designer.selection.is_empty(), "it did select");
        assert_eq!(text(&designer), before, "a selection wrote to the file");
    }

    #[test]
    fn dragging_a_handle_resizes_and_writes_the_extent() {
        let mut designer = designer_on("forms/reference.dform");
        designer.toggle_snapping();
        let path = path_named(&designer, "volume");
        let before_rect = designer.path_bounds(&path).expect("laid out");

        // Select first, then take the south-east handle.
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);
        let corner = Point::new(
            before_rect.x + before_rect.width,
            before_rect.y + before_rect.height,
        );
        drag_from_to(
            &mut designer,
            corner,
            Point::new(corner.x + 30, corner.y + 10),
        );

        let after = text(&designer);
        assert!(after.contains("w=254"), "{after}");
        assert!(after.contains("h=34"), "{after}");
    }

    #[test]
    fn an_arrow_key_nudges_by_one_and_shift_by_more() {
        let mut designer = designer_on("forms/reference.dform");
        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);

        press_key(&mut designer, KeyCode::ArrowRight, false);
        assert!(
            text(&designer).contains("x=141"),
            "one pixel: {}",
            text(&designer)
        );

        // Shift nudges by twice the grid, so 388 becomes 396.
        press_key(&mut designer, KeyCode::ArrowDown, true);
        assert!(
            text(&designer).contains("y=396"),
            "eight: {}",
            text(&designer)
        );
    }

    #[test]
    fn delete_takes_the_node_and_its_children_out_of_the_file() {
        let mut designer = designer_on("forms/reference.dform");
        let panel = path_named(&designer, "media-section");
        let at = middle(&designer, &panel);
        click_at(&mut designer, at, false);
        // The panel itself, not something inside it.
        designer.selection = vec![panel];

        press_key(&mut designer, KeyCode::Delete, false);

        let after = text(&designer);
        assert!(!after.contains("name=media-section"), "{after}");
        assert!(!after.contains("name=shots"), "a child survived its parent");
        assert!(after.contains("name=header"), "it took the whole file");
        assert!(designer.selection.is_empty());
        // And the canvas rebuilt from what is left.
        assert!(
            !designer.outline_names().any(|n| n == "media-section"),
            "the outline still lists it"
        );
        denise_forms::Form::parse(&after).expect("still a form");
    }

    #[test]
    fn snapping_lines_a_node_up_with_its_sibling_and_says_so() {
        let mut designer = designer_on("forms/reference.dform");
        let path = path_named(&designer, "volume");
        let stars = path_named(&designer, "stars");
        let target = designer
            .ui
            .layout(designer.node_id(&stars).unwrap())
            .expect("laid out");

        // Drag the slider until its left edge is a pixel or two from the
        // rating's, and let the snap close the gap.
        let from = middle(&designer, &path);
        let here = designer
            .ui
            .layout(designer.node_id(&path).unwrap())
            .unwrap();
        let want = target.x - here.x + 2;
        drag_from_to(&mut designer, from, Point::new(from.x + want, from.y));

        let now = designer
            .ui
            .layout(designer.node_id(&path).unwrap())
            .unwrap();
        assert_eq!(
            now.x, target.x,
            "it did not line up: {now:?} against {target:?}"
        );
    }

    #[test]
    fn a_snapping_drag_draws_a_guide_and_lets_it_go_on_release() {
        let mut designer = designer_on("forms/reference.dform");
        assert!(designer.select_named("volume"));
        let settled = designer.overlay.len();

        // Far enough left that its edge lands within snapping distance of the
        // rating and the labels, all of which start at x=16.
        designer.drag_selection(-122, -6);
        assert!(
            designer.overlay.len() > settled,
            "a snap drew no guide: {} against {settled}",
            designer.overlay.len()
        );
        let now = designer
            .ui
            .layout(designer.node_id(&path_named(&designer, "volume")).unwrap())
            .unwrap();
        assert_eq!(now.x, 16, "it did not line up: {now:?}");

        // Letting go takes the guides away and leaves the selection.
        designer.release();
        assert_eq!(designer.overlay.len(), settled);
        assert!(!designer.selection.is_empty());
    }

    // ------------------------------------------------------------- placing

    /// Types into the palette's filter and lets the frame run.
    fn set_filter(designer: &mut Designer, text: &str) {
        designer
            .ui
            .widget_mut::<TextInput<Message>>(designer.chrome.filter)
            .expect("a filter field")
            .set_text(text);
        designer.poll();
    }

    /// The middle of the palette row a kind is on, filtering to reach it.
    fn palette_point(designer: &mut Designer, kind: &str) -> Point {
        set_filter(designer, kind);
        let row = (0..designer.shown.len())
            .find(|row| designer.palette_kind(*row) == Some(kind))
            .unwrap_or_else(|| panic!("`{kind}` is not in the filtered palette"));
        let view = designer
            .ui
            .bounds(designer.chrome.palette_view)
            .expect("a palette");
        Point::new(
            view.x + view.width / 2,
            view.y + row as i32 * PALETTE_ROW + PALETTE_ROW / 2,
        )
    }

    /// The middle of the form on the canvas.
    fn stage_point(designer: &Designer) -> Point {
        let stage = designer.ui.bounds(designer.chrome.stage).expect("a stage");
        Point::new(stage.x + stage.width / 2, stage.y + stage.height / 2)
    }

    #[test]
    fn the_filter_narrows_the_palette_and_giving_it_up_puts_it_back() {
        let mut designer = designer_on("forms/hello.dform");
        let all = designer.shown.len();
        assert_eq!(all, denise_ui::widgets::all().len());

        set_filter(&mut designer, "prog");
        let shown: Vec<&str> = (0..designer.shown.len())
            .filter_map(|row| designer.palette_kind(row))
            .collect();
        assert_eq!(shown, vec!["progress", "radial-progress"]);

        set_filter(&mut designer, "nothing called this");
        assert!(designer.shown.is_empty());

        set_filter(&mut designer, "");
        assert_eq!(designer.shown.len(), all);
    }

    #[test]
    fn every_widget_that_ships_can_be_dragged_out_of_the_palette() {
        // Both ways in, for all twenty-five, against the file each time: what
        // the palette offers has to be what the designer can actually place.
        for widget in denise_ui::widgets::all() {
            let kind = widget.kind;
            let mut designer = designer_on("forms/hello.dform");
            let before = text(&designer);

            let from = palette_point(&mut designer, kind);
            let to = stage_point(&designer);
            drag_from_to(&mut designer, from, to);

            let after = text(&designer);
            assert_ne!(after, before, "dragging `{kind}` wrote nothing");
            let form = denise_forms::Form::parse(&after).unwrap_or_else(|e| {
                panic!("dragging `{kind}` made a file that will not parse: {e}")
            });
            assert!(
                designer.placed.iter().any(|p| p.kind == kind),
                "`{kind}` is in the file and not on the canvas: {after}"
            );
            assert!(
                !designer.status.contains("will not load"),
                "`{kind}`: {}",
                designer.status
            );
            let _ = form;

            // Selected the moment it lands, and the inspector is describing it.
            let selected = designer.selected().expect("nothing was selected");
            assert_eq!(designer.ui.kind(selected), Some(kind));
            assert_eq!(
                designer.inspector.as_ref().expect("a pane").rows.len(),
                designer.ui.properties(selected).len() + denise_forms::NODE_PROPERTIES.len(),
                "`{kind}`'s rows"
            );

            // And it is one step, which puts the file back exactly.
            assert_eq!(designer.history.depth().0, 1, "`{kind}` was not one step");
            designer.undo();
            assert_eq!(text(&designer), before, "undoing `{kind}` was not exact");
        }
    }

    #[test]
    fn every_widget_that_ships_can_be_drawn_the_winforms_way() {
        for widget in denise_ui::widgets::all() {
            let kind = widget.kind;
            let mut designer = designer_on("forms/hello.dform");
            let before = text(&designer);

            // Click the row, then drag a rectangle where it goes.
            let row = palette_point(&mut designer, kind);
            click_at(&mut designer, row, false);
            assert_eq!(
                designer.placing,
                Placing::Armed { kind },
                "clicking `{kind}` armed nothing"
            );

            let corner = stage_point(&designer);
            drag_from_to(
                &mut designer,
                corner,
                Point::new(corner.x + 60, corner.y + 40),
            );

            let after = text(&designer);
            assert_ne!(after, before, "drawing `{kind}` wrote nothing");
            denise_forms::Form::parse(&after).unwrap_or_else(|e| {
                panic!("drawing `{kind}` made a file that will not parse: {e}")
            });
            assert!(designer.placed.iter().any(|p| p.kind == kind), "{after}");
            // The rectangle drawn is the rectangle written, snapped to the grid.
            let rect = designer
                .ui
                .layout(designer.selected().expect("selected"))
                .expect("laid out");
            assert_eq!((rect.width, rect.height), (60, 40), "`{kind}`: {rect:?}");
            // And the row is no longer armed: one click, one widget.
            assert_eq!(designer.placing, Placing::Idle);
        }
    }

    #[test]
    fn a_press_on_a_palette_row_shows_a_ghost_once_it_travels() {
        let mut designer = designer_on("forms/hello.dform");
        let from = palette_point(&mut designer, "button");
        feed(&mut designer, &[button(ElementState::Down, from)]);
        assert!(
            matches!(designer.placing, Placing::Pressed { .. }),
            "a press is a click until it travels"
        );

        // A pixel is not a drag.
        let nudged = Point::new(from.x + 1, from.y);
        feed(
            &mut designer,
            &[InputEvent::PointerMoved { position: nudged }],
        );
        assert!(matches!(designer.placing, Placing::Pressed { .. }));

        let away = Point::new(from.x + 40, from.y + 20);
        feed(
            &mut designer,
            &[InputEvent::PointerMoved { position: away }],
        );
        let Placing::Carrying { ghost, .. } = designer.placing else {
            panic!("no ghost: {:?}", designer.placing);
        };
        // Drawn where the pointer is, on the root and above every pane: the
        // drag starts in one subtree and ends in another, and a ghost inside
        // either would be cut off at its edge.
        let bounds = designer.ui.bounds(ghost).expect("the ghost is laid out");
        assert_eq!((bounds.x, bounds.y), (away.x, away.y));

        // Letting go beside the form places nothing and says so.
        let before = text(&designer);
        feed(&mut designer, &[button(ElementState::Up, away)]);
        assert_eq!(designer.placing, Placing::Idle);
        assert!(!designer.ui.contains(ghost), "the ghost outlived the drag");
        assert_eq!(text(&designer), before);
        assert!(designer.status.contains("beside it"), "{}", designer.status);
    }

    #[test]
    fn a_widget_dropped_on_a_panel_becomes_a_child_of_it() {
        let mut designer = designer_on("forms/reference.dform");
        let panel = path_named(&designer, "form-section");
        let at = middle(&designer, &panel);

        let from = palette_point(&mut designer, "badge");
        drag_from_to(&mut designer, from, at);

        // The one just placed, which is the one selected — `reference.dform`
        // already has a badge of its own, inside the header.
        let path = designer.selection.last().expect("nothing selected").clone();
        assert_eq!(
            designer.ui.kind(designer.selected().expect("selected")),
            Some("badge")
        );
        assert_eq!(
            path[..path.len() - 1],
            panel[..],
            "it went into the wrong parent"
        );
        // Written relative to the panel, which is the only space a form knows.
        let line = text(&designer)
            .lines()
            .find(|line| line.contains("badge \"badge\""))
            .expect("the badge is in the file")
            .to_string();
        assert!(
            line.starts_with("        "),
            "it landed unindented: {line:?}"
        );
    }

    #[test]
    fn a_widget_dropped_where_nothing_can_hold_it_goes_on_the_form() {
        let mut designer = designer_on("forms/hello.dform");
        // A `select` holds options, not nodes: dropping on one has missed.
        let stage = designer.ui.bounds(designer.chrome.stage).expect("a stage");
        let corner = Point::new(stage.x + 8, stage.y + stage.height - 8);

        let from = palette_point(&mut designer, "spinner");
        drag_from_to(&mut designer, from, corner);

        let placed = designer
            .placed
            .iter()
            .find(|p| p.kind == "spinner")
            .expect("placed");
        assert_eq!(placed.path.len(), 1, "it went inside something");
    }

    #[test]
    fn an_armed_row_puts_one_down_on_enter_and_the_next_one_beside_it() {
        let mut designer = designer_on("forms/hello.dform");
        let row = palette_point(&mut designer, "badge");
        click_at(&mut designer, row, false);

        press_key(&mut designer, KeyCode::Enter, false);
        let first = designer
            .ui
            .bounds(designer.selected().expect("selected"))
            .expect("laid out");

        let row = palette_point(&mut designer, "badge");
        click_at(&mut designer, row, false);
        press_key(&mut designer, KeyCode::Enter, false);
        let second = designer
            .ui
            .bounds(designer.selected().expect("selected"))
            .expect("laid out");

        assert_ne!(first, second, "the second one hid under the first");
        // Both read `x=8 y=8`, and they are not in the same place: the first
        // landed on the form and the second, stepped clear of it, landed inside
        // the card — which is what a rectangle relative to its parent means.
        assert_eq!(
            text(&designer).matches("badge \"badge\"").count(),
            2,
            "{}",
            text(&designer)
        );
    }

    #[test]
    fn escape_gives_up_an_armed_row_and_a_press_elsewhere_does_too() {
        let mut designer = designer_on("forms/hello.dform");
        let row = palette_point(&mut designer, "button");
        click_at(&mut designer, row, false);
        assert!(designer.placing.kind().is_some());

        press_key(&mut designer, KeyCode::Escape, false);
        assert_eq!(designer.placing, Placing::Idle);

        // And a press on the inspector rather than the canvas.
        click_at(&mut designer, row, false);
        assert!(designer.placing.kind().is_some());
        let pane = designer
            .ui
            .bounds(designer.chrome.inspector_view)
            .expect("a pane");
        let before = text(&designer);
        click_at(
            &mut designer,
            Point::new(pane.x + pane.width / 2, pane.y + pane.height - 20),
            false,
        );
        assert_eq!(designer.placing, Placing::Idle);
        assert_eq!(
            text(&designer),
            before,
            "a press beside the canvas placed one"
        );
    }

    #[test]
    fn a_form_whose_picture_has_moved_still_opens() {
        // Answering `None` to a missing asset fails the whole build, so a
        // designer that did would refuse a form whose picture had been renamed —
        // which is exactly the form somebody opens a designer to fix.
        let source = "form \"P\" version=1 width=200 height=200 {\n                          image x=8 y=8 w=64 h=64 src=\"gone.png\"\n}\n";
        let path = std::env::temp_dir().join("denise-designer-missing.dform");
        std::fs::write(&path, source).expect("writing");

        let mut designer = designer_on("forms/hello.dform");
        designer.open(path);
        assert!(
            designer.placed.iter().any(|p| p.kind == "image"),
            "it refused the form: {}",
            designer.status
        );
        assert!(
            designer.status.contains("could not be loaded"),
            "it said nothing about the picture: {}",
            designer.status
        );
    }

    // ---------------------------------------------------------- the inspector

    /// The pane's row for a property, by name.
    fn row(designer: &Designer, name: &str) -> usize {
        let pane = designer.inspector.as_ref().expect("a pane");
        pane.rows
            .iter()
            .position(|row| row.property.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no `{name}` row among {:?}",
                    pane.rows
                        .iter()
                        .map(|r| r.property.name)
                        .collect::<Vec<_>>()
                )
            })
    }

    /// Puts text in a row's field, as a keystroke would, and lets the frame run.
    fn write(designer: &mut Designer, name: &str, text: &str) {
        let index = row(designer, name);
        let pane = designer.inspector.as_ref().expect("a pane");
        let field = match pane.rows[index].editor {
            Editor::Field(id) | Editor::Slid { field: id, .. } => id,
            ref other => panic!("`{name}` is edited by {other:?}, not a field"),
        };
        designer
            .ui
            .widget_mut::<denise_ui::widgets::TextInput<Message>>(field)
            .expect("a field")
            .set_text(text);
        designer.poll();
    }

    /// Ticks or unticks a row's box.
    fn tick(designer: &mut Designer, name: &str, on: bool) {
        let index = row(designer, name);
        let pane = designer.inspector.as_ref().expect("a pane");
        let Editor::Flag(id) = pane.rows[index].editor else {
            panic!("`{name}` is not a box");
        };
        designer
            .ui
            .widget_mut::<denise_ui::widgets::Checkbox<Message>>(id)
            .expect("a box")
            .set_checked(on);
        designer.poll();
    }

    /// What a row is showing.
    fn showing(designer: &Designer, name: &str) -> String {
        let index = row(designer, name);
        designer.inspector.as_ref().expect("a pane").rows[index]
            .shown
            .clone()
    }

    fn select(designer: &mut Designer, name: &str) {
        assert!(designer.select_named(name), "no node called `{name}`");
    }

    #[test]
    fn every_property_the_widget_declares_gets_a_row_and_so_does_every_one_the_tree_owns() {
        // The whole point of #85 reaching an application: this file names no
        // widget and no property, so a form full of different widgets produces
        // the right rows without a table anywhere in the designer.
        let mut designer = designer_on("forms/reference.dform");
        for name in ["volume", "notify", "full-name", "stream", "records"] {
            select(&mut designer, name);
            let id = designer.selected().expect("selected");
            let pane = designer.inspector.as_ref().expect("a pane");
            let rows: Vec<&str> = pane.rows.iter().map(|r| r.property.name).collect();

            for property in designer.ui.properties(id) {
                assert!(
                    rows.contains(&property.name),
                    "`{name}` has no row for `{}`: {rows:?}",
                    property.name
                );
            }
            for property in denise_forms::NODE_PROPERTIES {
                assert!(
                    rows.contains(&property.name),
                    "`{name}` has no row for the tree's `{}`",
                    property.name
                );
            }
            assert_eq!(
                rows.len(),
                designer.ui.properties(id).len() + denise_forms::NODE_PROPERTIES.len(),
                "`{name}` grew a row from somewhere: {rows:?}"
            );
        }
    }

    #[test]
    fn each_kind_of_property_gets_the_editor_it_calls_for() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        let pane = designer.inspector.as_ref().expect("a pane");
        let editor = |name: &str| {
            let index = pane
                .rows
                .iter()
                .position(|r| r.property.name == name)
                .unwrap_or_else(|| panic!("no `{name}`"));
            &pane.rows[index].editor
        };

        // A string, a box and a dropdown.
        assert!(matches!(editor("tooltip"), Editor::Field(_)));
        assert!(matches!(editor("visible"), Editor::Flag(_)));
        assert!(matches!(editor("role"), Editor::Choice { .. }));
        // A slider's own `value` runs between its own `min` and `max`, whatever
        // those turn out to be, so it is a field like any other number. `x`,
        // between -8192 and 8192, is a field for the opposite reason: a hundred
        // pixels over sixteen thousand units cannot be aimed.
        assert!(matches!(editor("value"), Editor::Field(_)));
        assert!(matches!(editor("x"), Editor::Field(_)));

        // A rating's `value` is between nought and five, which is what a slider
        // is for.
        select(&mut designer, "stars");
        let pane = designer.inspector.as_ref().expect("a pane");
        let index = pane
            .rows
            .iter()
            .position(|r| r.property.name == "value")
            .expect("a rating has a value");
        assert!(matches!(pane.rows[index].editor, Editor::Slid { .. }));
    }

    #[test]
    fn typing_changes_the_widget_on_the_canvas_before_anything_is_pressed() {
        // The "done when" of #93, taken literally: characters go into the field
        // one at a time and the button on the canvas says the new thing, with
        // no Enter and no message in between.
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "retry");
        let button = designer.selected().expect("selected");
        let index = row(&designer, "text");
        let Editor::Field(field) = designer.inspector.as_ref().unwrap().rows[index].editor else {
            panic!("`text` is not a field");
        };

        designer.ui.focus(Some(field));
        for character in "Go".chars() {
            feed(&mut designer, &[InputEvent::Text { ch: character }]);
            designer.poll();
        }

        assert_eq!(
            designer.ui.get_property(button, "text"),
            Some(denise_ui::widgets::Value::text("⟲Go")),
            "the canvas did not follow the keystrokes"
        );
        assert!(
            text(&designer).contains(r#"button "⟲Go""#),
            "{}",
            text(&designer)
        );
    }

    #[test]
    fn a_labels_text_is_written_where_the_file_already_writes_it() {
        // `button "⟲"` carries its text as an argument. Adding `text="…"` beside
        // it would leave the file saying one thing and the screen showing
        // another, so the argument is what changes.
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "retry");
        write(&mut designer, "text", "Again");

        let after = text(&designer);
        assert!(after.contains(r#"button "Again" name=retry"#), "{after}");
        let line = after
            .lines()
            .find(|line| line.contains("name=retry"))
            .expect("the button is still there");
        assert!(
            !line.contains("text="),
            "it added a property instead: {line}"
        );
        denise_forms::Form::parse(&after).expect("still a form");
    }

    #[test]
    fn a_value_the_field_cannot_mean_never_reaches_the_file() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        let before = text(&designer);

        write(&mut designer, "x", "over there");
        assert_eq!(text(&designer), before, "nonsense reached the file");
        assert!(
            designer.status.contains("whole number"),
            "it did not say why: {}",
            designer.status
        );

        // And the next thing typed, which does mean something, goes through.
        write(&mut designer, "x", "200");
        assert!(text(&designer).contains("x=200"), "{}", text(&designer));
    }

    #[test]
    fn a_number_typed_into_a_row_moves_the_node_and_writes_one_line() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        let before = text(&designer);
        let id = designer.selected().expect("selected");

        write(&mut designer, "y", "300");
        assert_eq!(
            designer.ui.layout(id).expect("laid out").y,
            300,
            "the canvas did not follow"
        );
        let changed = diff(&before, &text(&designer));
        assert_eq!(changed.len(), 1, "{changed:#?}");
        assert!(changed[0].contains("y=300"), "{}", changed[0]);
    }

    #[test]
    fn a_run_of_keystrokes_in_one_field_is_one_undo() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        let before = text(&designer);

        for value in ["1", "12", "123"] {
            write(&mut designer, "x", value);
        }
        assert!(text(&designer).contains("x=123"), "{}", text(&designer));
        assert_eq!(
            designer.history.depth().0,
            1,
            "three keystrokes, three steps"
        );

        designer.undo();
        assert_eq!(text(&designer), before, "one undo did not put the run back");
    }

    #[test]
    fn a_box_writes_a_flag_and_a_dropdown_writes_a_bare_name() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "notify");
        tick(&mut designer, "checked", false);
        assert!(
            text(&designer).contains("checked=#false"),
            "{}",
            text(&designer)
        );

        select(&mut designer, "notify");
        let index = row(&designer, "role");
        designer.open_choice(index);
        let options = match designer.inspector.as_ref().unwrap().rows[index].editor {
            Editor::Choice { options, .. } => options,
            _ => panic!("`role` is not a dropdown"),
        };
        let accent = options.iter().position(|o| *o == "accent").expect("a role");
        designer.chose(accent);
        designer.poll();

        // Bare, as a form file writes a name — not `role="accent"`.
        assert!(
            text(&designer).contains("role=accent"),
            "{}",
            text(&designer)
        );
        denise_forms::Form::parse(&text(&designer)).expect("still a form");
    }

    #[test]
    fn a_default_is_dimmed_and_resetting_one_takes_it_out_of_the_file() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");

        // The slider writes `role=success`; it does not write `tooltip`.
        assert_eq!(showing(&designer, "role"), "success");
        let fields = {
            let paths = designer.selection.clone();
            let ids: Vec<NodeId> = paths.iter().filter_map(|p| designer.node_id(p)).collect();
            designer.fields(&paths, &ids)
        };
        let written = |name: &str| {
            fields
                .iter()
                .find(|f| f.property.name == name)
                .map(|f| f.written)
                .expect(name)
        };
        assert!(written("role"), "the file writes it");
        assert!(!written("tooltip"), "the file does not write it");

        designer.reset(row(&designer, "role"));
        let after = text(&designer);
        let line = after
            .lines()
            .find(|line| line.contains("name=volume"))
            .expect("the slider is still there")
            .to_string();
        assert!(
            !line.contains("role="),
            "the property is still written: {line}"
        );
        assert!(
            line.contains("min=0 max=100 value=70 step=5"),
            "it took the rest of the line: {line}"
        );

        // And the widget went back to what it is without one, which is only
        // knowable by building it again.
        select(&mut designer, "volume");
        assert_eq!(
            designer
                .ui
                .get_property(designer.selected().expect("selected"), "role"),
            Some(denise_ui::widgets::Value::role(Role::Primary)),
            "a slider with no role in the file is a primary one"
        );
    }

    #[test]
    fn the_four_rectangle_rows_follow_a_drag_on_the_canvas() {
        let mut designer = designer_on("forms/reference.dform");
        designer.toggle_snapping();
        select(&mut designer, "volume");
        assert_eq!(showing(&designer, "x"), "140");

        designer.drag_selection(24, 8);
        assert_eq!(showing(&designer, "x"), "164", "the field did not follow");
        assert_eq!(showing(&designer, "y"), "396");
        // And the drag is still one step when it is let go of.
        designer.release();
        assert_eq!(designer.history.depth().0, 1);
    }

    #[test]
    fn several_selected_shows_what_they_share_and_edits_all_of_them() {
        let mut designer = designer_on("forms/reference.dform");
        let (first, second) = (
            path_named(&designer, "notify"),
            path_named(&designer, "dark"),
        );
        designer.selection = vec![first, second];
        designer.selected = designer.node_id(&designer.selection[1].clone());
        designer.refresh_inspector();

        // A checkbox and a toggle: `checked` is common, and the checkbox's
        // `size` is too. Both are ticked, so the row agrees.
        assert_eq!(showing(&designer, "checked"), "#true");
        // `y` differs between them, so the row is blank rather than one of them.
        assert_eq!(showing(&designer, "y"), "");

        tick(&mut designer, "checked", false);
        let after = text(&designer);
        assert_eq!(
            after.matches("checked=#false").count(),
            2,
            "only some of them changed: {after}"
        );
        assert_eq!(designer.history.depth().0, 1, "one edit, one step");
    }

    #[test]
    fn nothing_selected_is_a_pane_that_says_so_rather_than_an_empty_one() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        assert!(!designer.inspector.as_ref().unwrap().rows.is_empty());

        press_key(&mut designer, KeyCode::Escape, false);
        assert!(designer.inspector.as_ref().unwrap().rows.is_empty());
        // And polling an empty pane is not a crash.
        designer.poll();
    }

    #[test]
    fn an_empty_field_puts_a_property_back_to_its_default() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        write(&mut designer, "step", "5");
        assert!(text(&designer).contains("step=5"), "{}", text(&designer));

        // With the caret in the field, as it is for somebody deleting what is
        // in it.
        let index = row(&designer, "step");
        let field = designer.inspector.as_ref().unwrap().rows[index]
            .editor
            .focusable();
        designer.ui.focus(Some(field));

        write(&mut designer, "step", "");
        // The file does not wait; the canvas does, because rebuilding it would
        // take the caret out of the field being typed in.
        assert!(!text(&designer).contains("step=5"), "{}", text(&designer));
        assert!(designer.stale, "nothing is waiting to be rebuilt");

        designer.ui.focus(None);
        designer.poll();
        assert!(!designer.stale, "it never caught up");
        let slider = designer.selected().expect("selected");
        assert_eq!(
            designer.ui.get_property(slider, "step"),
            None,
            "the slider kept a step the file no longer gives it"
        );
    }

    // --------------------------------------------------------------- undo

    const CTRL: denise::Modifiers = denise::Modifiers::CTRL;

    fn ctrl_z(designer: &mut Designer, shift: bool) {
        let modifiers = if shift {
            denise::Modifiers::CTRL | denise::Modifiers::SHIFT
        } else {
            CTRL
        };
        press_with(designer, KeyCode::Z, modifiers);
    }

    #[test]
    fn a_whole_drag_is_one_undo_and_puts_the_file_back_exactly() {
        let mut designer = designer_on("forms/reference.dform");
        designer.toggle_snapping();
        let before = text(&designer);
        let path = path_named(&designer, "volume");

        // A drag that moves and resizes at once is still one step.
        let from = middle(&designer, &path);
        drag_from_to(&mut designer, from, Point::new(from.x + 24, from.y + 8));
        assert_ne!(text(&designer), before);
        assert_eq!(
            designer.history.depth(),
            (1, 0),
            "a drag was more than one step"
        );

        ctrl_z(&mut designer, false);
        assert_eq!(text(&designer), before, "undo did not restore the file");
        assert_eq!(designer.history.depth(), (0, 1));
    }

    #[test]
    fn a_run_of_nudges_is_one_undo() {
        let mut designer = designer_on("forms/reference.dform");
        let before = text(&designer);
        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);

        for _ in 0..10 {
            press_key(&mut designer, KeyCode::ArrowRight, false);
        }
        assert!(text(&designer).contains("x=150"), "{}", text(&designer));
        assert_eq!(designer.history.depth().0, 1, "ten nudges were ten steps");

        ctrl_z(&mut designer, false);
        assert_eq!(text(&designer), before);
    }

    #[test]
    fn undoing_a_delete_brings_the_node_and_its_children_back() {
        let mut designer = designer_on("forms/reference.dform");
        let before = text(&designer);
        let panel = path_named(&designer, "media-section");
        let at = middle(&designer, &panel);
        click_at(&mut designer, at, false);
        designer.selection = vec![panel];

        press_key(&mut designer, KeyCode::Delete, false);
        assert!(!text(&designer).contains("name=shots"));

        ctrl_z(&mut designer, false);
        assert_eq!(
            text(&designer),
            before,
            "undoing a delete did not restore the file byte for byte"
        );
        // And the canvas came back with it.
        assert!(designer.outline_names().any(|n| n == "shots"));
    }

    #[test]
    fn redo_puts_back_what_undo_took_and_a_new_edit_discards_it() {
        let mut designer = designer_on("forms/reference.dform");
        designer.toggle_snapping();
        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);
        press_key(&mut designer, KeyCode::ArrowRight, false);
        let moved = text(&designer);

        ctrl_z(&mut designer, false);
        assert_ne!(text(&designer), moved);
        ctrl_z(&mut designer, true);
        assert_eq!(text(&designer), moved, "redo did not put it back");

        // Undo, then do something else: the redo branch is gone.
        ctrl_z(&mut designer, false);
        assert!(designer.history.can_redo());
        let other = path_named(&designer, "stars");
        let at = middle(&designer, &other);
        click_at(&mut designer, at, false);
        press_key(&mut designer, KeyCode::ArrowDown, false);
        assert!(!designer.history.can_redo());
    }

    #[test]
    fn the_title_says_when_there_is_unsaved_work_and_stops_saying_it() {
        let mut designer = designer_on("forms/reference.dform");
        assert!(!designer.document.label().contains('•'));

        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);
        press_key(&mut designer, KeyCode::ArrowRight, false);
        assert!(
            designer.document.label().contains('•'),
            "no modified marker: {}",
            designer.document.label()
        );

        // Undoing back to where it was saved makes it clean again.
        ctrl_z(&mut designer, false);
        assert!(
            !designer.document.label().contains('•'),
            "still marked: {}",
            designer.document.label()
        );
    }

    #[test]
    fn closing_with_unsaved_work_asks_before_it_goes() {
        let mut designer = designer_on("forms/reference.dform");
        designer.request_exit();
        assert!(designer.exit_requested(), "a clean form closes at once");

        let mut designer = designer_on("forms/reference.dform");
        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);
        press_key(&mut designer, KeyCode::ArrowRight, false);

        designer.request_exit();
        assert!(!designer.exit_requested(), "it left without asking");
        assert!(designer.status.contains("unsaved"), "{}", designer.status);

        designer.request_exit();
        assert!(designer.exit_requested(), "asking twice did not go through");
    }

    #[test]
    fn a_bare_z_is_not_undo() {
        let mut designer = designer_on("forms/reference.dform");
        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);
        click_at(&mut designer, at, false);
        press_key(&mut designer, KeyCode::ArrowRight, false);
        let moved = text(&designer);

        press_key(&mut designer, KeyCode::Z, false);
        assert_eq!(text(&designer), moved, "a bare Z undid something");
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
            .position(|name| name == "notify")
            .expect("the checkbox is named");
        designer.handle(Message::Outline(index));
        let box_ = designer.selected().expect("selected");
        let bounds = designer.ui.bounds(box_).expect("laid out");
        let middle = Point::new(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
        // Over the canvas, and not over a pane beside it: a press that missed
        // the form would prove nothing about the scrim.
        let canvas = designer
            .ui
            .bounds(designer.chrome.canvas)
            .expect("a canvas");
        assert!(canvas.contains(middle), "{middle:?} is not over {canvas:?}");

        // A press right on a checkbox in the form under design.
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
