//! The designer: a toolbar, three panes and a status line.

use std::path::PathBuf;

use denise::{
    ElementState, InputEvent, KeyCode, Point, PointerButton, Radius, Rect, Role, Size, theme,
};
use denise_forms::{Edit, Handler, Payload, Picture, Placed, Wiring};
use denise_ui::widgets::{Button, Divider, Label, List, ListItem, Panel};
use denise_ui::{Anchors, Dock, NodeId, Ui};

use crate::canvas::{Drag, Grip, Guide, place, topmost};
use crate::document::Document;
use crate::history::History;
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
    /// The two buttons that go grey when there is nothing to put back.
    undo_button: NodeId,
    redo_button: NodeId,
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
        undo_button,
        redo_button,
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
            outline: Vec::new(),
            selected: None,
            placed: Vec::new(),
            selection: Vec::new(),
            drag: None,
            overlay: Vec::new(),
            snapping: true,
            grid: 4,
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

    // ------------------------------------------------------------ design mode

    /// Reads the events before the tree does, and hands back what the tree
    /// should still see.
    ///
    /// Everything over the canvas is design mode's: a press there selects,
    /// drags or resizes, and the tree never learns of it. Everything else — the
    /// toolbar, the palette, the outline — passes through untouched, which is
    /// why the panes go on working while the form does not.
    pub fn input(&mut self, events: &[InputEvent]) -> Vec<InputEvent> {
        let canvas = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        let mut forward = Vec::with_capacity(events.len());

        for event in events {
            let mine = match event {
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state,
                    position,
                    modifiers,
                } if canvas.contains(*position) || self.drag.is_some() => {
                    match state {
                        ElementState::Down => {
                            self.press(*position, modifiers.contains(denise::Modifiers::SHIFT))
                        }
                        ElementState::Up => self.release(),
                    }
                    true
                }
                InputEvent::PointerMoved { position } if self.drag.is_some() => {
                    self.drag_to(*position);
                    true
                }
                InputEvent::Key {
                    code,
                    state: ElementState::Down,
                    modifiers,
                    ..
                } => self.key(*code, *modifiers),
                _ => false,
            };
            if !mine {
                forward.push(event.clone());
            }
        }
        forward
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
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            Message::Inert => {}
        }
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
