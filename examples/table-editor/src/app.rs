//! The editor's tree, and everything that happens in it.
//!
//! Platform-independent on purpose: this file never learns whether it is running
//! in a window on macOS, a child control inside a Win32 dialog, or on the scanout
//! buffer of a Raspberry Pi with no desktop at all. The backends in `main.rs` are
//! about forty lines each, and this is the application.
//!
//! # Building a widget the toolkit does not have
//!
//! There are four widgets — label, button, panel, text field — and none of them
//! is a table. A grid row here is a full-width `Button` with the cell `Label`s
//! placed on top of it: labels are not interactive, so a click falls through them
//! to the button underneath and arrives as `Select(index)`. That is the whole
//! trick, and it is worth knowing because it is how most of the widgets you will
//! miss can be assembled.
//!
//! # Why the rows are not rebuilt
//!
//! [`VISIBLE_ROWS`] nodes exist from startup and never change; scrolling moves
//! which record each one displays. Rebuilding the tree per frame would be simpler
//! to write and would throw away focus and the caret position every time anybody
//! typed — and it would allocate on a machine chosen for having very little to
//! allocate from.

use std::time::Instant;

use denise::{Rect, Role, Size, Theme};
use denise_ui::widgets::{Align, Button, Label, Panel, TextInput};
use denise_ui::{FontId, NodeId, TextStyle, Ui};

use crate::table::{Row, Table};

/// How many records are on screen at once. The tree holds exactly this many rows.
pub const VISIBLE_ROWS: usize = 9;

const ROW_HEIGHT: i32 = 26;
const GRID_TOP: i32 = 76;
const GRID_LEFT: i32 = 16;
/// Where each column starts, and how wide it is.
const COLUMNS: [(i32, i32); 4] = [(12, 190), (206, 200), (410, 60), (474, 150)];
const GRID_WIDTH: i32 = 640;

/// What the widgets send back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// A grid row was clicked. The index is into the *visible* rows.
    Select(usize),
    ScrollUp,
    ScrollDown,
    Apply,
    Add,
    Delete,
    ConfirmDelete,
    CancelDelete,
    Save,
    Reload,
    NextTheme,
    /// A key tapped on the on-screen keyboard.
    Key(denise::KeyCode),
}

/// The nodes that get written to after startup.
#[derive(Default)]
struct Nodes {
    title: Option<NodeId>,
    rows: Vec<NodeId>,
    cells: Vec<[Option<NodeId>; 4]>,
    fields: [Option<NodeId>; 4],
    /// The form panel, and where it sits with no keyboard in the way.
    form: Option<NodeId>,
    form_home: Rect,
    status: Option<NodeId>,
    position: Option<NodeId>,
    /// Every node drawn at body size, and every node drawn at heading size.
    /// Collected while building so that registering a font afterwards can restyle
    /// all of them — including the ones nothing else ever writes to again.
    body: Vec<NodeId>,
    heading: Vec<NodeId>,
}

pub struct App {
    pub ui: Ui<Message>,
    table: Table,
    path: String,
    /// Which record is selected, as an index into the whole table.
    selected: Option<usize>,
    /// The first record shown in the grid.
    scroll: usize,
    nodes: Nodes,
    /// The dimmed confirmation scene, while it is up.
    confirm: Option<NodeId>,
    /// Physical pixels per logical one. Every number in this file is logical.
    scale: f32,
    theme_index: usize,
    started: Instant,
    body: TextStyle,
    heading: TextStyle,
    pub exit: bool,
    /// The on-screen keyboard: the only one a panel has, and the whole reason
    /// the form has to get out of its way.
    keyboard: denise_keyboard::Keyboard,
}

impl App {
    /// Builds the tree once, for a surface of `size` physical pixels at `scale`.
    ///
    /// `body` and `heading` name whichever font was loaded, at sizes their caller
    /// has already scaled. Every rectangle below is written in logical pixels and
    /// passed through `s`: the application scales, once, here, which is the whole
    /// of the DPI story and the reason this layout survives a Retina display.
    pub fn new(
        size: Size,
        scale: f32,
        path: String,
        table: Table,
        body: TextStyle,
        heading: TextStyle,
    ) -> Self {
        let mut ui: Ui<Message> = Ui::new(size, Theme::BUILT_IN[0].scaled(scale));
        let s = |r: Rect| r.scaled(scale);
        // The surface back in the units the layout is written in.
        let logical_w = ((size.width as f32) / scale + 0.5) as i32;
        let mut nodes = Nodes::default();
        let root = ui.root();

        nodes.title = ui.add(
            root,
            Label::new("").with_style(heading),
            s(Rect::new(GRID_LEFT, 14, 700, 26)),
        );
        nodes.heading.extend(nodes.title);
        nodes.body.extend(
            ui.add(
                root,
                Label::new("Tab moves, Enter applies, F2 changes theme")
                    .with_style(body)
                    .with_align(Align::End, Align::Center),
                s(Rect::new(logical_w - 400, 16, 384, 22)),
            ),
        );

        // ------------------------------------------------------------ the grid

        let grid = ui
            .add(
                root,
                Panel::default(),
                s(Rect::new(
                    GRID_LEFT,
                    GRID_TOP - 34,
                    GRID_WIDTH,
                    ROW_HEIGHT * VISIBLE_ROWS as i32 + 44,
                )),
            )
            .expect("grid");

        for (column, name) in Row::COLUMNS.iter().enumerate() {
            let (x, width) = COLUMNS[column];
            nodes.body.extend(ui.add(
                grid,
                Label::new(*name).with_style(body).with_role(Role::Accent),
                s(Rect::new(x, 8, width, 20)),
            ));
        }

        for index in 0..VISIBLE_ROWS {
            let y = 34 + index as i32 * ROW_HEIGHT;
            // The row itself: a full-width button with no text, which is what
            // makes the whole row clickable and gives it hover and pressed states
            // for free.
            let row = ui
                .add(
                    grid,
                    Button::new("", Message::Select(index)).with_role(Role::Neutral),
                    s(Rect::new(6, y, GRID_WIDTH - 12, ROW_HEIGHT - 2)),
                )
                .expect("row");
            nodes.rows.push(row);

            // The cells, on top. Labels are not interactive, so the click passes
            // through them to the row.
            let mut cells = [None; 4];
            for (column, slot) in cells.iter_mut().enumerate() {
                let (x, width) = COLUMNS[column];
                *slot = ui.add(
                    grid,
                    Label::new("").with_style(body),
                    s(Rect::new(x, y + 4, width, 18)),
                );
            }
            nodes.body.extend(cells.iter().flatten().copied());
            nodes.cells.push(cells);
        }

        nodes.position = ui.add(
            root,
            Label::new("")
                .with_style(body)
                .with_align(Align::End, Align::Center),
            s(Rect::new(
                GRID_LEFT + GRID_WIDTH - 210,
                GRID_TOP + ROW_HEIGHT * VISIBLE_ROWS as i32 + 16,
                200,
                20,
            )),
        );
        nodes.body.extend(nodes.position);
        for (offset, (text, message)) in [("Up", Message::ScrollUp), ("Down", Message::ScrollDown)]
            .into_iter()
            .enumerate()
        {
            nodes.body.extend(ui.add(
                root,
                Button::new(text, message).with_style(body),
                s(Rect::new(
                    GRID_LEFT + offset as i32 * 78,
                    GRID_TOP + ROW_HEIGHT * VISIBLE_ROWS as i32 + 14,
                    70,
                    26,
                )),
            ));
        }

        // ------------------------------------------------------------ the form

        let form_x = GRID_LEFT + GRID_WIDTH + 16;
        let form_home = s(Rect::new(
            form_x,
            GRID_TOP - 34,
            logical_w - form_x - 16,
            300,
        ));
        let form = ui.add(root, Panel::default(), form_home).expect("form");
        // Scrollable so the keyboard has an answer. A form of four rows never
        // overflows on its own and this changes nothing while the keyboard is
        // down — but with the keyboard up the form is cut to the space above it
        // (see `fit_form_to_keyboard`), and a viewport that may be scrolled is
        // what lets the tree bring the focused row into what is left.
        ui.set_scrollable(form, true);
        nodes.form = Some(form);
        nodes.form_home = form_home;
        nodes.heading.extend(ui.add(
            form,
            Label::new("Record").with_style(heading),
            s(Rect::new(14, 10, 200, 24)),
        ));

        for (index, name) in Row::COLUMNS.iter().enumerate() {
            let y = 48 + index as i32 * 52;
            nodes.body.extend(ui.add(
                form,
                Label::new(*name).with_style(body),
                s(Rect::new(14, y, 160, 18)),
            ));
            nodes.fields[index] = ui.add(
                form,
                TextInput::<Message>::new()
                    .with_style(body)
                    .with_submit(Message::Apply),
                s(Rect::new(14, y + 20, 200, 28)),
            );
            nodes.body.extend(nodes.fields[index]);
        }

        // ---------------------------------------------------------- the actions

        let actions_y = GRID_TOP + ROW_HEIGHT * VISIBLE_ROWS as i32 + 58;
        for (offset, (text, message, role)) in [
            ("Apply", Message::Apply, Role::Primary),
            ("Add", Message::Add, Role::Secondary),
            ("Delete", Message::Delete, Role::Error),
            ("Save", Message::Save, Role::Success),
            ("Reload", Message::Reload, Role::Neutral),
        ]
        .into_iter()
        .enumerate()
        {
            nodes.body.extend(ui.add(
                root,
                Button::new(text, message).with_role(role).with_style(body),
                s(Rect::new(GRID_LEFT + offset as i32 * 92, actions_y, 84, 30)),
            ));
        }

        nodes.status = ui.add(
            root,
            Label::new("").with_style(body),
            s(Rect::new(GRID_LEFT, actions_y + 42, logical_w - 32, 20)),
        );
        nodes.body.extend(nodes.status);

        let mut app = Self {
            ui,
            table,
            path,
            selected: None,
            scroll: 0,
            nodes,
            confirm: None,
            scale,
            theme_index: 0,
            started: Instant::now(),
            body,
            heading,
            exit: false,
            // Whatever the machine is configured for, at the display's scale
            // and in the form's own font — a `Button` given no style falls back
            // to the built-in bitmap face, which on a panel is the one widget
            // somebody touches drawn in the one typeface that is not the rest.
            keyboard: denise_keyboard::Keyboard::from_system()
                .0
                .with_scale(scale)
                .with_style(body),
        };
        app.select(if app.table.is_empty() { None } else { Some(0) });
        app.refresh();
        app
    }

    // ------------------------------------------------------------- the updates

    /// Writes the whole visible state into the tree.
    ///
    /// Called after anything that changes what should be on screen. Every write
    /// goes through `widget_mut`, which marks that one node for repaint — so a
    /// call that changes one cell repaints one cell, and this being the only
    /// update path costs nothing.
    fn refresh(&mut self) {
        let title = format!(
            "{}{} — {} records",
            self.path,
            if self.table.is_dirty() { " *" } else { "" },
            self.table.len()
        );
        self.set_label(self.nodes.title, &title);

        for visible in 0..VISIBLE_ROWS {
            let index = self.scroll + visible;
            let row = self.table.get(index).cloned();
            for column in 0..Row::COLUMNS.len() {
                let text = row.as_ref().map(|r| r.field(column)).unwrap_or("");
                let node = self.nodes.cells[visible][column];
                self.set_label(node, text);
            }

            // The selected row is the one drawn in the accent colour. A role, not
            // a colour: it stays legible when the theme changes.
            let role = if Some(index) == self.selected && row.is_some() {
                Role::Accent
            } else {
                Role::Neutral
            };
            if let Some(id) = self.nodes.rows.get(visible).copied() {
                // A row with no record behind it is hidden rather than drawn
                // empty: an empty bar looks like a record whose fields are blank.
                self.ui.set_visible(id, row.is_some());
                if let Some(button) = self.ui.widget_mut::<Button<Message>>(id) {
                    button.set_role(role);
                }
            }
        }

        let position = match self.selected {
            Some(index) => format!("row {} of {}", index + 1, self.table.len()),
            None => "no selection".to_string(),
        };
        self.set_label(self.nodes.position, &position);

        // The status line says the most useful true thing: what is wrong with the
        // record being edited, or what is wrong anywhere, or that all is well.
        let status = match self
            .selected
            .and_then(|index| self.table.get(index))
            .and_then(Row::problem)
        {
            Some(problem) => problem,
            None => match self.table.first_problem() {
                Some((index, problem)) => format!("row {}: {problem}", index + 1),
                None if self.table.is_dirty() => "Unsaved changes".to_string(),
                None => format!("Saved to {}", self.path),
            },
        };
        self.set_label(self.nodes.status, &status);
    }

    fn set_label(&mut self, node: Option<NodeId>, text: &str) {
        if let Some(label) = node.and_then(|id| self.ui.widget_mut::<Label>(id)) {
            // `update` only writes when the text differs, so an unchanged cell
            // does not cost a repaint. `widget_mut` has already marked the node,
            // which is the conservative half of the bargain.
            label.update(text);
        }
    }

    /// Selects a record and fills the form from it.
    fn select(&mut self, index: Option<usize>) {
        self.selected = index;
        if let Some(index) = index {
            // Keep the selection on screen, whichever direction it left by.
            if index < self.scroll {
                self.scroll = index;
            } else if index >= self.scroll + VISIBLE_ROWS {
                self.scroll = index + 1 - VISIBLE_ROWS;
            }
        }

        let row = index
            .and_then(|i| self.table.get(i))
            .cloned()
            .unwrap_or_default();
        for column in 0..Row::COLUMNS.len() {
            if let Some(field) = self.nodes.fields[column]
                .and_then(|id| self.ui.widget_mut::<TextInput<Message>>(id))
            {
                field.set_text(row.field(column).to_string());
            }
        }
    }

    /// Reads the form back into the selected record.
    fn apply(&mut self) {
        let Some(index) = self.selected else {
            return;
        };
        let read = |app: &App, column: usize| -> String {
            app.nodes.fields[column]
                .and_then(|id| app.ui.widget::<TextInput<Message>>(id))
                .map(|field| field.text().to_string())
                .unwrap_or_default()
        };
        let row = Row {
            name: read(self, 0),
            role: read(self, 1),
            age: read(self, 2),
            city: read(self, 3),
        };
        self.table.replace(index, row);
    }

    // ------------------------------------------------------------ the messages

    /// Handles everything the tree emitted this frame.
    pub fn handle(&mut self, now_ms: u64) {
        // Drained until it stops rather than once: a key press is answered by
        // feeding events straight back into the tree, and whatever those produce
        // belongs to the same frame as the tap. Bounded, so a message that
        // produced itself costs a frame rather than the application.
        let mut acted = false;
        for _ in 0..8 {
            let messages: Vec<Message> = self.ui.drain_messages().collect();
            if messages.is_empty() {
                break;
            }
            acted = true;
            for message in messages {
                self.on(message);
            }
        }
        if acted {
            self.refresh();
        }
        // Every frame, not only the ones with messages: focus moves on a Tab
        // the application never sees, and the keyboard follows focus.
        self.keyboard.follow_focus(&mut self.ui, Message::Key);
        self.fit_form_to_keyboard();
        let _ = now_ms;
    }

    /// Puts the caret in the first form field, which brings the keyboard up.
    ///
    /// Through focus rather than by opening the keyboard, because that is the
    /// path a tap takes. What `--keyboard` does.
    pub fn focus_first_field(&mut self) {
        if let Some(field) = self.nodes.fields[0] {
            self.ui.focus(Some(field));
        }
    }

    /// Puts the caret in the last form field: the one the keyboard covers.
    ///
    /// What a snapshot of the keyboard should show, because the first field is
    /// the case where nothing has to happen.
    pub fn focus_last_field(&mut self) {
        if let Some(field) = self.nodes.fields.iter().rev().find_map(|f| *f) {
            self.ui.focus(Some(field));
        }
    }

    /// Whether the on-screen keyboard is up.
    pub const fn keyboard_open(&self) -> bool {
        self.keyboard.is_open()
    }

    /// Puts the keyboard away, taking the focus that summoned it.
    ///
    /// Clearing focus is what makes it stay shut: the keyboard follows focus, so
    /// closing it with the caret still in a field would bring it back.
    pub fn dismiss_keyboard(&mut self) {
        self.ui.focus(None);
        self.keyboard.close(&mut self.ui);
        self.fit_form_to_keyboard();
    }

    /// Cuts the form to the screen the keyboard has left it, and restores it.
    ///
    /// The form is a fixed panel of four rows: it never overflows by itself, so
    /// there is nothing to scroll and the tree's reveal correctly makes no move
    /// — leaving `City` under the keys being pressed. Lifting the whole panel
    /// instead does not work either, and the reason is arithmetic rather than
    /// taste: on a panel this short a 300-tall form does not fit above a
    /// 330-tall keyboard at any offset.
    ///
    /// So the form is shortened to what is above the keyboard, which makes it a
    /// viewport with more content than room — and *that* the tree knows what to
    /// do with. [`Keyboard::occluded`] is the only part the application has to
    /// supply, because only the application knows which of its panels may be cut
    /// short.
    fn fit_form_to_keyboard(&mut self) {
        let Some(form) = self.nodes.form else {
            return;
        };
        let home = self.nodes.form_home;
        let want = match self.keyboard.occluded(&self.ui) {
            // A minimum, so a keyboard on a truly tiny display leaves a form
            // that is cramped rather than one that is gone.
            Some(covered) => Rect::new(
                home.x,
                home.y,
                home.width,
                (covered.y - home.y).max(self.px(64)).min(home.height),
            ),
            None => home,
        };
        if self.ui.layout(form) == Some(want) {
            return;
        }
        self.ui.set_layout(form, want);
        // The reveal that came with the focus ran against the form's old height;
        // nothing about the focus has changed since, so nothing re-runs it.
        self.ui.reveal_focused();
    }

    /// A logical measurement in physical pixels.
    fn px(&self, v: i32) -> i32 {
        ((v as f32) * self.scale + 0.5) as i32
    }

    /// Handles one message. Public so a backend can synthesise one from a key.
    pub fn on_message(&mut self, message: Message) {
        self.on(message);
        self.refresh();
    }

    fn on(&mut self, message: Message) {
        match message {
            Message::Key(code) => {
                // Straight back into the tree, as though the events had come off
                // a keyboard plugged into the machine.
                let events = self.keyboard.press_key(&mut self.ui, code);
                self.ui.handle(&events);
            }
            Message::Select(visible) => {
                let index = self.scroll + visible;
                if index < self.table.len() {
                    // Applying on the way out means clicking another row keeps
                    // what was typed, rather than silently discarding it.
                    self.apply();
                    self.select(Some(index));
                }
            }
            Message::ScrollUp => self.scroll = self.scroll.saturating_sub(VISIBLE_ROWS),
            Message::ScrollDown => {
                let last_page = self.table.len().saturating_sub(VISIBLE_ROWS);
                self.scroll = (self.scroll + VISIBLE_ROWS).min(last_page);
            }
            Message::Apply => self.apply(),
            Message::Add => {
                self.apply();
                let index = self.table.push(Row::default());
                self.select(Some(index));
                if let Some(field) = self.nodes.fields[0] {
                    self.ui.focus(Some(field));
                }
            }
            Message::Delete => self.open_confirmation(),
            Message::CancelDelete => self.close_confirmation(),
            Message::ConfirmDelete => {
                self.close_confirmation();
                if let Some(index) = self.selected {
                    let next = self.table.remove(index);
                    self.select(next);
                }
            }
            Message::Save => self.save(),
            Message::Reload => self.load(),
            Message::NextTheme => {
                self.theme_index = (self.theme_index + 1) % Theme::BUILT_IN.len();
                self.ui
                    .set_theme(Theme::BUILT_IN[self.theme_index].scaled(self.scale));
            }
        }
    }

    fn save(&mut self) {
        match std::fs::write(&self.path, self.table.format()) {
            Ok(()) => self.table.saved(),
            Err(e) => self.set_label(self.nodes.status, &format!("Could not save: {e}")),
        }
    }

    fn load(&mut self) {
        self.table = match std::fs::read_to_string(&self.path) {
            Ok(text) => Table::parse(&text),
            Err(_) => Table::parse(crate::table::SAMPLE),
        };
        self.scroll = 0;
        self.select(if self.table.is_empty() { None } else { Some(0) });
    }

    // ------------------------------------------------------------ the modal

    /// Puts a confirmation over the top, on its own dimmed scene.
    ///
    /// A scene is the toolkit's whole answer to modality: what is underneath is
    /// still drawn and still there, and stops taking input because the scene above
    /// it takes it all. Nothing below needs disabling and nothing needs a flag.
    fn open_confirmation(&mut self) {
        if self.confirm.is_some() || self.selected.is_none() {
            return;
        }
        let name = self
            .selected
            .and_then(|index| self.table.get(index))
            .map(|row| row.name.clone())
            .unwrap_or_default();
        // `Ui::size` is physical; the dialog is placed in logical units like
        // everything else and scaled on the way in.
        let s = |r: Rect| r.scaled(self.scale);
        let size = self.ui.size();
        let (w, h) = (
            ((size.width as f32) / self.scale + 0.5) as i32,
            ((size.height as f32) / self.scale + 0.5) as i32,
        );

        let scene = self.ui.push_scene(150);
        let dialog = self
            .ui
            .add(
                scene,
                Panel::default(),
                s(Rect::new(w / 2 - 200, h / 2 - 80, 400, 160)),
            )
            .expect("dialog");
        self.ui.add(
            dialog,
            Label::new("Delete this record?").with_style(self.heading),
            s(Rect::new(20, 20, 360, 24)),
        );
        self.ui.add(
            dialog,
            Label::new(if name.is_empty() {
                "The unnamed record".to_string()
            } else {
                name
            })
            .with_style(self.body),
            s(Rect::new(20, 52, 360, 20)),
        );
        let cancel = self.ui.add(
            dialog,
            Button::new("Cancel", Message::CancelDelete),
            s(Rect::new(20, 100, 110, 32)),
        );
        self.ui.add(
            dialog,
            Button::new("Delete", Message::ConfirmDelete).with_role(Role::Error),
            s(Rect::new(270, 100, 110, 32)),
        );

        // Focus lands on the safe answer, so Enter on a dialog nobody read does
        // the harmless thing.
        self.ui.focus(cancel);
        self.confirm = Some(scene);
    }

    fn close_confirmation(&mut self) {
        if self.confirm.take().is_some() {
            self.ui.pop_scene();
        }
    }

    pub fn is_confirming(&self) -> bool {
        self.confirm.is_some()
    }

    /// Points every piece of text at a font registered after the tree was built.
    ///
    /// The tree is built with the built-in font so that it exists whether or not
    /// a file was found, and re-styled here once one has been. Two sizes, because
    /// a heading and a row of data want different ones out of the same face.
    pub fn set_font(&mut self, font: FontId) {
        self.body = TextStyle {
            font,
            size_px: self.body.size_px,
        };
        self.heading = TextStyle {
            font,
            size_px: self.heading.size_px,
        };

        for (nodes, style) in [
            (self.nodes.body.clone(), self.body),
            (self.nodes.heading.clone(), self.heading),
        ] {
            for id in nodes {
                self.restyle(id, style);
            }
        }
        // The keyboard is not in `nodes.body` because its keys do not exist
        // until it opens; it carries the style itself and hands it to each key
        // as that key is built.
        let body = self.body;
        self.keyboard.set_style(&mut self.ui, body);
        self.ui.invalidate_all();
    }

    /// Restyles one node, whichever of the three text-bearing widgets it holds.
    ///
    /// A downcast per kind, because the tree stores widgets as trait objects and
    /// "has a text style" is not something the `Widget` trait says. Three tries is
    /// the honest cost of that, and it happens once at startup.
    fn restyle(&mut self, id: NodeId, style: TextStyle) {
        if let Some(label) = self.ui.widget_mut::<Label>(id) {
            label.set_style(style);
        } else if let Some(button) = self.ui.widget_mut::<Button<Message>>(id) {
            button.set_style(style);
        } else if let Some(field) = self.ui.widget_mut::<TextInput<Message>>(id) {
            field.set_style(style);
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Moves the selection by one record, for the arrow keys.
    pub fn move_selection(&mut self, delta: i32) {
        if self.table.is_empty() {
            return;
        }
        self.apply();
        let last = self.table.len() as i32 - 1;
        let current = self.selected.unwrap_or(0) as i32;
        self.select(Some((current + delta).clamp(0, last) as usize));
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denise::KeyCode;

    /// The panel's own shape: short enough that the keyboard covers the form,
    /// which is the whole situation being tested.
    fn app() -> App {
        App::new(
            Size::new(1000, 470),
            1.0,
            "test.csv".into(),
            Table::parse(crate::table::SAMPLE),
            TextStyle::built_in(15),
            TextStyle::built_in(22),
        )
    }

    /// Tapping a field brings the keyboard, and typing on it reaches the field.
    ///
    /// The caret has to survive the tap on a key: a keyboard that types one
    /// character and then loses the field it was typing into is not a keyboard.
    #[test]
    fn a_field_summons_the_keyboard_and_the_keys_reach_it() {
        let mut app = app();
        let name = app.nodes.fields[0].expect("the name field");
        app.ui.focus(Some(name));
        app.handle(0);
        assert!(app.keyboard_open(), "focusing a field summoned nothing");

        app.ui
            .widget_mut::<TextInput<Message>>(name)
            .expect("the name field")
            .clear();
        for code in [KeyCode::A, KeyCode::D, KeyCode::A] {
            app.on_message(Message::Key(code));
        }
        app.handle(0);
        let field = app
            .ui
            .widget::<TextInput<Message>>(name)
            .expect("the name field");
        assert_eq!(field.text(), "ada");
        assert_eq!(app.ui.focused(), Some(name), "a key press stole the caret");

        // Escape puts it away, and it stays away — closing with the caret still
        // in the field would summon it again on the next frame.
        app.dismiss_keyboard();
        app.handle(0);
        assert!(!app.keyboard_open(), "it came straight back");
    }

    /// The last field ends up somewhere it can be read, on a panel where it
    /// cannot simply be moved.
    ///
    /// `City` is the bottom row of a 300-tall form, and the keyboard claims 330
    /// of a 470-tall screen: no offset puts the whole form above the keys. So
    /// the form is cut to what is left and scrolled inside it, which is the one
    /// answer that exists — and the assertion is the one that matters either
    /// way: the field somebody is typing into is not under the keyboard.
    #[test]
    fn the_last_field_is_readable_with_the_keyboard_up() {
        let mut app = app();
        let form = app.nodes.form.expect("the form");
        let city = app.nodes.fields[3].expect("the city field");
        let home = app.ui.layout(form).expect("laid out");

        app.ui.focus(Some(city));
        app.handle(0);
        let covered = app.keyboard.occluded(&app.ui).expect("the keyboard is up");
        let cut = app.ui.layout(form).expect("laid out");
        assert!(
            cut.bottom() <= covered.y,
            "the form still runs under the keyboard: {cut:?} against {covered:?}"
        );

        let bounds = app.ui.bounds(city).expect("the field is placed");
        assert!(
            bounds.bottom() <= covered.y && bounds.y >= cut.y,
            "the field is not in what is left of the form: {bounds:?} in {cut:?}"
        );

        // And the form gets its full height back when the keyboard leaves.
        app.dismiss_keyboard();
        app.handle(0);
        assert_eq!(
            app.ui.layout(form),
            Some(home),
            "the form stayed cut short with nothing covering it"
        );
    }

    /// The first field is already clear, so nothing scrolls it.
    ///
    /// A form that jumped whenever the keyboard appeared would be worse than one
    /// that never moved: the movement is only justified where it buys something.
    #[test]
    fn a_field_already_clear_of_the_keyboard_is_left_where_it_is() {
        let mut app = app();
        let form = app.nodes.form.expect("the form");
        let name = app.nodes.fields[0].expect("the name field");
        let before = app.ui.bounds(name).expect("the field is placed");

        app.ui.focus(Some(name));
        app.handle(0);
        assert!(app.keyboard_open());
        assert_eq!(
            app.ui.scroll(form),
            denise::Point::ZERO,
            "the form scrolled for a field that was already visible"
        );
        assert_eq!(app.ui.bounds(name), Some(before));
    }
}
