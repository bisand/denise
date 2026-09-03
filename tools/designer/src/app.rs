//! The designer: a toolbar, three panes and a status line.

use std::path::{Path, PathBuf};
use std::time::Duration;

use denise::{
    ElementState, InputEvent, KeyCode, Point, PointerButton, Radius, Rect, Role, Size, theme,
};
use denise_forms::{Edit, Form, FormKind, Handler, Literal, Payload, Picture, Placed, Wiring};
use denise_ui::widgets::{
    Align, Button, Divider, Group, Label, List, ListItem, Panel, Property, PropertyKind, Tabs,
    TextInput, Value, WidgetInfo, open_select,
};
use denise_ui::{Anchors, Dock, NodeId, TextStyle, Ui};

use crate::arrange::{self, Command, Needs};
use crate::canvas::{self, Band, Drag, Grip, Guide, between, place, resting, snap, topmost};
use crate::clipboard::Clipboard;
use crate::code;
use crate::document::Document;
use crate::history::History;
use crate::inspector::{Editor, Field, Inspector, is_event, show_value};
use crate::outline::{self, Outline};
use crate::scale::Scale;
use crate::settings::{PaletteMode, Settings};
use crate::text::Text;
use crate::watch::{self, differences};
use crate::zoom::Zoom;

/// How many message names a form can have before the log stops naming them.
///
/// A widget that carries a value holds a **function pointer** — `fn(bool) -> M`
/// — which cannot capture which name it belongs to. So there is a table of
/// function pointers, one per name, generated below; sixty-four is more names
/// than a screen has and the log says so plainly if a form has more.
const NAMES: usize = 64;

/// What the designer's own widgets emit.
#[derive(Clone, Copy, PartialEq, Debug)]
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
    /// A rename in the outline was finished with Enter.
    Renamed,
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
    /// Appends an item to the collection a row edits.
    ItemAdd(usize),
    /// Takes one out: the row, and which item.
    ItemRemove(usize, usize),
    /// Moves one earlier in the file, and so earlier in the widget.
    ItemUp(usize, usize),
    /// And later.
    ItemDown(usize, usize),
    /// One of the arrange commands: align, size, space, group.
    Arrange(Command),
    /// A kind picked in the new-form sheet.
    NewKind(usize),
    /// A size preset picked in the new-form sheet.
    NewSize(usize),
    /// Make the form the new-form sheet describes.
    Create,
    /// Put the new-form sheet away and change nothing.
    Never,
    /// Take the file on disk and lose what is unsaved here.
    Reload,
    /// Keep what is unsaved here, and overwrite the file on the next save.
    KeepMine,
    /// Turns preview mode on, or off again.
    Preview,
    /// Turns the tab-order overlay on, or off again.
    TabOrder,
    /// The next theme along.
    Theme,
    /// The zoom control: the next step round, and back to fit after the widest.
    Zoom,
    /// The palette's display mode: the next of glyph-and-name, name, glyphs.
    PaletteMode,
    /// Open an inspector row's event handler in the editor.
    OpenCode(usize),
    /// A key tapped on the on-screen keyboard.
    Key(KeyCode),
    /// A message the **form under design** emitted, by the index of its name.
    ///
    /// A designer cannot know an application's message type, so every name in an
    /// open form resolves to one of these four. Nothing fires them while the
    /// canvas is in design mode — the scrim over the form absorbs the press — and
    /// in preview mode they are what the log is showing.
    Fired(usize),
    /// One carrying a flag: a checkbox, a toggle, a collapse.
    FiredBool(usize, bool),
    /// One carrying a choice: anything that selects one of several.
    FiredIndex(usize, usize),
    /// One carrying a number: a slider, a rating.
    FiredNumber(usize, f32),
    /// A name the form used that this build ran out of table for.
    Inert,
}

/// Fixed extents. Everything else is docked, so the window resizes for free.
const TOOLBAR: i32 = 44;
const STATUS: i32 = 26;
const PALETTE_ROW: i32 = 24;
/// Eleven rows exactly, so the viewport never cuts one in half.
const PALETTE_ROWS: i32 = PALETTE_ROW * 11;
/// A glyph tile in the palette's glyphs-only mode.
const TILE: i32 = 28;
/// How many tiles sit abreast in that mode.
const TILE_COLUMNS: i32 = 4;
/// The field that filters the palette.
const FILTER: i32 = 26;
/// How far a press has to travel before it is a drag rather than a click.
const THRESHOLD: i32 = 4;
/// The strip of arrange commands, under the toolbar.
const ARRANGE: i32 = 30;
const HEADER: i32 = 22;
const GAP: i32 = 8;
/// The message log, while previewing. Nothing at all while designing.
const LOG: i32 = 84;
/// How many fired messages the log keeps.
const LOGGED: usize = 6;

/// The new-form sheet, while somebody is answering it.
///
/// A form has a kind before it has anything else — Delphi asked *Form / Data
/// module / Frame* first, and for the same reason: the kind decides what the
/// rest of the questions even are.
struct Making {
    /// What has been picked so far.
    kind: FormKind,
    /// The kind buttons, so the picked one can be the one that looks picked.
    kinds: Vec<NodeId>,
    /// The line under them, which says what the picked kind is for.
    note: NodeId,
    /// The two fields, which a preset writes into and a person may overwrite.
    width: NodeId,
    height: NodeId,
}

/// Numbering the form's tab stops, while somebody is re-sequencing them.
///
/// Delphi had this and WinForms kept it: turn it on, and every place Tab can
/// land is numbered on the canvas in the order it will be reached. Click them in
/// the order you want instead.
///
/// **Only siblings can be re-sequenced**, and that falls out of the format
/// rather than being a shortcut. Tab order is file order read depth first, and a
/// file can only say that one node comes before another *within one parent* — so
/// moving a field from inside one panel to a place in the sequence inside
/// another would mean moving it into that panel, which is a change to the design
/// and not to the order. Clicking across a parent starts a new run there, and the
/// status line says so.
#[derive(Clone, Debug, Default)]
struct Ordering {
    /// The paths clicked so far in this run, in the order they were clicked.
    picked: Vec<Vec<usize>>,
}

/// The other editor's version of the file, while somebody decides about it.
///
/// Only ever up when there is unsaved work to lose. With nothing to lose there
/// is no question worth asking, and the file is simply read again.
struct Clash {
    /// What was on disk when the question was asked.
    ///
    /// Held rather than re-read on the answer, so that *Reload* takes the
    /// version that was described in the list — not whatever the other editor
    /// has done in the seconds since.
    text: String,
}

/// How to find a node again after the file has been read afresh.
///
/// By the name the file gave it, because that is the one piece of identity a
/// form node carries and the other editor may have moved it; by where it sat if
/// it has no name, because that is all there is to go on.
#[derive(Clone, Debug)]
struct Keepsake {
    name: Option<String>,
    path: Vec<usize>,
}

/// How many changed nodes the conflict sheet names before it stops naming them.
const NAMED: usize = 8;

/// The sizes offered, which are the panels this toolkit is aimed at.
const PRESETS: &[(&str, u32, u32)] = &[
    ("800x480", 800, 480),
    ("1024x600", 1024, 600),
    ("1280x720", 1280, 720),
    ("1920x1080", 1920, 1080),
];

/// What the canvas is standing in for.
///
/// Preview mode is **hiding the scrim** and letting the events through, which is
/// what the whole canvas design was for; the rest of it is simulating the machine
/// the form will run on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Simulated {
    /// Whatever the file's own `theme=` says.
    Own,
    Dark,
    Light,
    HighContrast,
}

impl Simulated {
    const ALL: [Self; 4] = [Self::Own, Self::Dark, Self::Light, Self::HighContrast];

    const fn name(self) -> &'static str {
        match self {
            Self::Own => "theme: the form's",
            Self::Dark => "theme: dark",
            Self::Light => "theme: light",
            Self::HighContrast => "theme: high contrast",
        }
    }

    fn next(self) -> Self {
        let at = Self::ALL.iter().position(|held| *held == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

/// One row of the palette: a shelf's heading, or a widget standing on it.
///
/// The palette is one `List`, so a heading is a row like any other — a disabled
/// one, which the widget already knows to skip with the keyboard and ignore
/// under the pointer. Design mode takes the palette's presses for itself anyway,
/// so what a heading really costs is this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shelf {
    /// A group heading.
    Heading(Group),
    /// A widget, by its index into [`Designer::palette`].
    Widget(usize),
}

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
    /// The button beside the palette's heading that cycles [`PaletteMode`].
    mode_button: NodeId,
    /// The outline's scrolling viewport.
    outline_view: NodeId,

    /// The inspector's scrolling viewport. The pane inside it is replaced
    /// whenever the selection changes; this outlives every form.
    inspector_view: NodeId,
    /// The strip that lists the messages the form has fired. Nothing but a
    /// docked node of no height until preview mode gives it some.
    log: NodeId,
    /// The lines inside it, replaced whenever one arrives.
    log_lines: NodeId,
    /// The arrange commands, in the order [`Command::ALL`] gives them. Greyed
    /// out one at a time, because they do not all need the same selection.
    arrange_buttons: Vec<(Command, NodeId)>,
    /// The button that says which mode it is.
    preview_button: NodeId,
    /// The one that turns the tab-order overlay on.
    tab_order_button: NodeId,
    /// The button that says which theme is being simulated.
    theme_button: NodeId,
    /// The one that says what magnification the canvas is at.
    zoom_button: NodeId,
    /// The invisible sheet over the form. Hiding it *is* preview mode.
    scrim: Option<NodeId>,
    /// What the form is shown *over*, for the kinds that are shown over
    /// something: the dimmed backdrop behind a dialog, the screen a drawer
    /// comes in across. `None` for a form that is the whole surface.
    surface: Option<NodeId>,
    /// The two columns, greyed out while the form is running: a palette that
    /// looked live and was not would be worse than one that says so.
    columns: [NodeId; 2],
    /// The node the form is built under, replaced whenever a form is opened.
    stage: NodeId,
    /// The canvas's scrolling viewport, which the stage is centred in.
    canvas: NodeId,
}

impl Chrome {
    /// Every node that belongs to the **chrome**, named, for the test that
    /// checks the whole of it scales together.
    ///
    /// Deliberately not the stage, the scrim or the surface: those three are the
    /// canvas, which is drawn at 1:1 whatever the display does, because a form
    /// is authored in the panel's own device pixels. See [`Scale`].
    #[cfg(test)]
    fn every(&self) -> Vec<(&'static str, NodeId)> {
        let mut all = vec![
            ("title", self.title),
            ("status", self.status),
            ("undo", self.undo_button),
            ("redo", self.redo_button),
            ("preview", self.preview_button),
            ("tab order", self.tab_order_button),
            ("theme", self.theme_button),
            ("zoom", self.zoom_button),
            ("palette viewport", self.palette_view),
            ("filter", self.filter),
            ("palette", self.palette),
            ("palette mode", self.mode_button),
            ("outline viewport", self.outline_view),
            ("inspector viewport", self.inspector_view),
            ("log", self.log),
            ("log lines", self.log_lines),
            ("left column", self.columns[0]),
            ("right column", self.columns[1]),
            ("canvas", self.canvas),
        ];
        all.extend(
            self.arrange_buttons
                .iter()
                .map(|(command, id)| (command.label(), *id)),
        );
        all
    }
}

/// Builds every pane, and docks them.
///
/// The whole layout is `Dock`: a toolbar along the top, a status line along the
/// bottom, two columns against the sides, and the canvas taking what is left. So
/// the window resizes correctly without this file doing arithmetic on a resize,
/// which is the thing anchoring and docking were added for.
fn build_chrome(ui: &mut Ui<Message>, settings: &Settings, scale: Scale) -> Chrome {
    let root = ui.root();
    // Every rectangle and every text size below is written in logical units and
    // multiplied here, on the way in — the pattern `docs/design.md` settles on,
    // and the reason none of the constants above mention DPI. See [`Scale`].
    let s = |rect: Rect| scale.r(rect);
    let px = |text: Text| scale.text(text);

    let toolbar = ui
        .add(
            root,
            Panel::filled(Role::Base200),
            s(Rect::new(0, 0, 0, TOOLBAR)),
        )
        .expect("root");
    ui.set_dock(toolbar, Some(Dock::Top));

    let mut x = GAP;
    let mut kept: Vec<NodeId> = Vec::new();
    for (text, message) in [
        ("New", Message::New),
        ("Open…", Message::Open),
        ("Save", Message::Save),
        ("Save as…", Message::SaveAs),
        ("Undo", Message::Undo),
        ("Redo", Message::Redo),
        ("Design", Message::Preview),
        ("Tab order", Message::TabOrder),
        (Simulated::Own.name(), Message::Theme),
        // Built at its widest and relabelled on the first frame: a toolbar
        // button takes its width from the text it is built with, and `fit
        // (100%)` is longer than the `100%` it opens saying.
        (Zoom::WIDEST_LABEL, Message::Zoom),
    ] {
        let width = 8 * text.chars().count() as i32 + 24;
        let id = ui.add(
            toolbar,
            Button::new(text, message)
                .with_role(if matches!(message, Message::Preview) {
                    Role::Primary
                } else {
                    Role::Neutral
                })
                .with_size(px(Text::Body)),
            s(Rect::new(x, GAP, width, TOOLBAR - GAP * 2)),
        );
        if matches!(
            message,
            Message::Undo
                | Message::Redo
                | Message::Preview
                | Message::TabOrder
                | Message::Theme
                | Message::Zoom
        ) && let Some(id) = id
        {
            kept.push(id);
        }
        x += width + 6;
    }
    let (undo_button, redo_button) = (kept[0], kept[1]);
    let (preview_button, tab_order_button) = (kept[2], kept[3]);
    let (theme_button, zoom_button) = (kept[4], kept[5]);
    let title = ui
        .add(
            toolbar,
            Label::new("Untitled")
                .with_size(px(Text::Body))
                .with_role(Role::BaseContent),
            s(Rect::new(x + GAP, GAP, 420, TOOLBAR - GAP * 2)),
        )
        .expect("toolbar");

    // A second strip under the toolbar: what to do with more than one node
    // selected. Always there rather than appearing when it applies — a command
    // nobody can see is a command nobody knows about — and greyed out one
    // button at a time, each saying in its tooltip what it wants instead.
    let arrange_bar = ui
        .add(
            root,
            Panel::filled(Role::Base200),
            s(Rect::new(0, 0, 0, ARRANGE)),
        )
        .expect("root");
    ui.set_dock(arrange_bar, Some(Dock::Top));

    let mut x = GAP;
    let caption = |ui: &mut Ui<Message>, x: &mut i32, text: &str| {
        let width = 7 * text.chars().count() as i32 + 6;
        ui.add(
            arrange_bar,
            Label::new(text)
                .with_size(px(Text::Caption))
                .with_role(Role::Base300),
            s(Rect::new(*x, 0, width, ARRANGE)),
        );
        *x += width;
    };
    let mut arrange_buttons: Vec<(Command, NodeId)> = Vec::new();
    for command in Command::ALL {
        match command {
            Command::Left => caption(ui, &mut x, "align"),
            Command::SameWidth => caption(ui, &mut x, "size"),
            Command::SpaceAcross => caption(ui, &mut x, "space"),
            Command::Group => caption(ui, &mut x, "group"),
            _ => {}
        }
        let label = command.label();
        let width = (8 * label.chars().count() as i32 + 16).max(24);
        if let Some(id) = ui.add(
            arrange_bar,
            Button::new(label, Message::Arrange(command))
                .with_role(Role::Neutral)
                .with_size(px(Text::Body)),
            s(Rect::new(x, 4, width, ARRANGE - 8)),
        ) {
            ui.set_tooltip(id, command.what());
            arrange_buttons.push((command, id));
        }
        x += width + 4;
    }

    let status_bar = ui
        .add(
            root,
            Panel::filled(Role::Base200),
            s(Rect::new(0, 0, 0, STATUS)),
        )
        .expect("root");
    ui.set_dock(status_bar, Some(Dock::Bottom));
    let status = ui
        .add(
            status_bar,
            Label::new("")
                .with_size(px(Text::Caption))
                .with_role(Role::Base300),
            s(Rect::new(GAP, 0, 4000, STATUS)),
        )
        .expect("status bar");

    // Above the status line and across the whole width, and of no height at all
    // until preview mode gives it some: a strip that was there while designing
    // would be a strip with nothing in it.
    let log = ui
        .add(root, Panel::filled(Role::Base200), s(Rect::new(0, 0, 0, 0)))
        .expect("root");
    ui.set_dock(log, Some(Dock::Bottom));
    let log_lines = ui
        .add(log, Panel::default(), s(Rect::new(0, 0, 1, 1)))
        .expect("the log strip is there");

    let left = ui
        .add(
            root,
            Panel::filled(Role::Base100),
            s(Rect::new(0, 0, settings.left, 0)),
        )
        .expect("root");
    ui.set_dock(left, Some(Dock::Left));

    let right = ui
        .add(
            root,
            Panel::filled(Role::Base100),
            s(Rect::new(0, 0, settings.right, 0)),
        )
        .expect("root");
    ui.set_dock(right, Some(Dock::Right));

    let canvas = ui
        .add(root, Panel::filled(Role::Base300), s(Rect::new(0, 0, 0, 0)))
        .expect("root");
    ui.set_dock(canvas, Some(Dock::Fill));
    // Larger than the window is reachable rather than cropped.
    ui.set_scrollable(canvas, true);

    // The left column: a palette above a divider above an outline.
    let width = settings.left - GAP * 2;
    ui.add(
        left,
        Label::new("Palette")
            .with_size(px(Text::Heading))
            .with_role(Role::Primary),
        s(Rect::new(GAP, GAP, width - 64, HEADER)),
    );
    // The heading shares its row with the button that says how the palette is
    // listing things — and, pressed, lists them the next way instead.
    let mode_button = ui
        .add(
            left,
            Button::new(settings.palette.name(), Message::PaletteMode)
                .with_role(Role::Neutral)
                .with_size(px(Text::Caption)),
            s(Rect::new(GAP + width - 60, GAP, 60, HEADER)),
        )
        .expect("left");
    ui.set_tooltip(
        mode_button,
        "How the palette lists widgets: glyph and name, name alone, or glyphs alone",
    );
    let split = GAP + HEADER + FILTER + PALETTE_ROWS + GAP;
    ui.add(left, Divider::new(), s(Rect::new(GAP, split, width, 8)));
    ui.add(
        left,
        Label::new("Outline")
            .with_size(px(Text::Heading))
            .with_role(Role::Primary),
        s(Rect::new(GAP, split + 12, width, HEADER)),
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
                .with_size(px(Text::Body))
                .with_max_chars(32),
            s(Rect::new(GAP, GAP + HEADER, width, FILTER - 2)),
        )
        .expect("left");
    let palette_view = ui
        .add(
            left,
            Panel::filled(Role::Base100),
            s(Rect::new(GAP, GAP + HEADER + FILTER, width, PALETTE_ROWS)),
        )
        .expect("left");
    ui.set_scrollable(palette_view, true);
    // Replaced by the first `fill_palette`; a node has to exist for it to
    // remove.
    let palette = ui
        .add(palette_view, Panel::default(), s(Rect::new(0, 0, 1, 1)))
        .expect("palette viewport");

    let outline_top = split + 12 + HEADER;
    let outline_view = ui
        .add(
            left,
            Panel::filled(Role::Base100),
            s(Rect::new(GAP, outline_top, width, 240)),
        )
        .expect("left");
    ui.set_scrollable(outline_view, true);
    // Held top and bottom, so the outline takes whatever height the window has.
    ui.set_anchors(outline_view, Anchors::new(true, true, true, true));

    // A form node with twenty properties of its own and fourteen the tree owns
    // is taller than any pane, so the rows go in a viewport that scrolls.
    let inspector_view = ui
        .add(
            right,
            Panel::default(),
            s(Rect::new(0, 0, settings.right, 0)),
        )
        .expect("right");
    ui.set_dock(inspector_view, Some(Dock::Fill));
    ui.set_scrollable(inspector_view, true);

    // Replaced by the first `show_form`; a node has to exist for it to remove.
    let stage = ui
        .add(canvas, Panel::default(), s(Rect::new(0, 0, 1, 1)))
        .expect("canvas");

    Chrome {
        title,
        status,
        undo_button,
        redo_button,
        palette_view,
        filter,
        palette,
        mode_button,
        outline_view,
        inspector_view,
        columns: [left, right],
        log,
        log_lines,
        arrange_buttons,
        preview_button,
        tab_order_button,
        theme_button,
        zoom_button,
        scrim: None,
        surface: None,
        stage,
        canvas,
    }
}

/// The designer.
pub struct Designer {
    pub ui: Ui<Message>,
    /// What the display multiplies logical units by.
    ///
    /// The chrome is written in logical units and scaled on the way into the
    /// tree. See [`Scale`], and [`Designer::zoom`] for the canvas, which is a
    /// different multiplication for a different reason.
    scale: Scale,
    /// How many screen pixels the canvas draws one form pixel as.
    ///
    /// Everything under the stage is built at this — `Form::build_scaled` puts
    /// the whole subtree in screen units — so hit testing, the handles and the
    /// tree's own painting need no conversion at all. What does need one is
    /// every number on its way to the **file** or the **inspector**, and every
    /// number arriving from them. See [`Zoom`].
    zoom: Zoom,
    chrome: Chrome,
    document: Document,
    settings: Settings,
    /// Every widget the toolkit ships, in the order the palette lists them.
    palette: Vec<&'static str>,
    /// What the palette is showing: a heading per shelf with anything the
    /// filter let through under it, in `Group::ALL` order.
    shown: Vec<Shelf>,
    /// Where each entry of [`Designer::shown`] sits, in the palette's own
    /// scaled coordinates: stacked rows in the list modes, tiles under
    /// headings in glyphs mode. One arithmetic for drawing and hit-testing,
    /// whichever mode built it.
    slots: Vec<Rect>,
    /// What the filter field last held.
    filter: String,
    /// A widget on its way onto the canvas.
    placing: Placing,
    /// The frame clock, as of the last `keyboard_turn`: what pairs two presses
    /// on an event's name into the double-click that opens its handler.
    now_ms: u64,
    /// The last press on an event's name in the inspector, and when.
    last_name_press: Option<(usize, u64)>,
    /// The lower-left pane, rebuilt whenever the tree or the selection changes.
    outline: Option<Outline>,
    /// The subtrees drawn shut, by path.
    folded: Vec<Vec<usize>>,
    /// The nodes hidden **in the designer only**, by path. The file never learns
    /// about these: they are how something behind something else is reached.
    hidden: Vec<Vec<usize>>,
    /// Every `tab` page the open form built, from the last build.
    pages: Vec<denise_forms::Page>,
    /// Which tab the designer is *looking at*, per `tabs` node, when that is
    /// not the one the file opens on.
    ///
    /// Designer state, like [`Designer::hidden`], and for the same reason: a
    /// form that remembered which tab somebody had open while working on it
    /// would be carrying that to the panel. `selected` in the file stays the
    /// tab an application starts on.
    looking_at: Vec<(Vec<usize>, usize)>,
    /// A row being dragged in the outline.
    outline_drag: Option<outline::Drag>,
    selected: Option<NodeId>,
    /// Every node of the open form, and where in the file it came from.
    placed: Vec<Placed>,
    /// What is selected, by file path rather than by `NodeId`: a path survives a
    /// rebuild, and every edit rebuilds.
    selection: Vec<Vec<usize>>,
    drag: Option<Drag>,
    /// A rubber band being drawn over the canvas.
    band: Option<Band>,
    /// Where copied nodes go, as `.dform` source.
    clipboard: Clipboard,
    /// The new-form sheet, while it is up.
    making: Option<Making>,
    /// The file-changed-underneath sheet, while it is up.
    clash: Option<Clash>,
    /// The rest of the selection while one of them is being dragged, with the
    /// rectangle each had when the drag began. A drag moves all of them by the
    /// same amount, and writes all of them as one edit.
    ///
    /// **Form** rectangles, like [`Designer::dragged_to`] and `Drag::origin`:
    /// the file is what a drag is really editing, and the tree is given a copy.
    carrying: Vec<(Vec<usize>, Rect)>,
    /// Where the drag in flight has put each node, in **form** coordinates.
    ///
    /// The rectangle a drag commits, rather than one read back out of the tree.
    /// Below 100% zoom a form pixel is less than a screen pixel, so what the
    /// tree holds cannot say what the file should: `in_form(on_screen(11))` is
    /// 12 at 50%. Keeping the form rectangle means a node lands exactly where
    /// the arithmetic put it at every magnification. See [`Zoom`].
    dragged_to: Vec<(Vec<usize>, Rect)>,
    /// The container a drag would drop into, while one is in flight. Drawn on
    /// the canvas so a reparent is never a surprise.
    dropping: Option<Vec<usize>>,
    /// The selection outline, its handles and any alignment guides. Rebuilt
    /// whenever any of them moves, and removed before the form is.
    overlay: Vec<NodeId>,
    snapping: bool,
    grid: i32,
    /// The right pane, rebuilt whenever the selection changes.
    inspector: Option<Inspector>,
    /// The row whose dropdown is open, and the popup it opened.
    choosing: Option<(usize, NodeId)>,
    /// Whether the form is being run rather than drawn.
    preview: bool,
    /// The tab-order overlay, while it is up.
    ordering: Option<Ordering>,
    /// Which theme the canvas is standing in for.
    simulated: Simulated,
    /// Every message name the open form used, for the log to name them by.
    names: Vec<String>,
    /// The last few messages the form fired, newest last.
    fired: Vec<String>,
    /// The on-screen keyboard, up over the form while previewing.
    keyboard: denise_keyboard::Keyboard,
    /// Whether it was up last time anybody looked.
    keyboard_up: bool,
    /// Whether the form is behind the file.
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
    /// Set by `remember_size` and cleared by `settle_resize`.
    resized: bool,
    status: String,
    /// What the window's title bar should say. Kept rather than built on demand:
    /// `DeniseApp::title` is asked once a frame and `Document::label` allocates.
    window_title: String,
    exit: bool,
}

impl Designer {
    /// Builds the designer's own tree.
    pub fn new(size: Size, scale: f32, settings: Settings, document: Document) -> Self {
        // The one multiplication, at construction, as `docs/design.md` requires:
        // the theme's furniture here, and every chrome rectangle and text size
        // through the `Scale` this keeps. `size` is already physical.
        let scale = Scale::new(scale);
        let mut ui: Ui<Message> = Ui::new(size, theme::DARK.scaled(scale.factor()));
        // The tree draws tooltips itself, so this is the only way to say how big
        // they are — and the palette's whole answer to #126 is a tooltip.
        ui.set_tooltip_size(scale.text(Text::Body));
        let chrome = build_chrome(&mut ui, &settings, scale);
        let palette: Vec<&'static str> = denise_ui::widgets::all().iter().map(|w| w.kind).collect();

        let mut designer = Self {
            ui,
            scale,
            zoom: Zoom::default().on_device(scale.factor()),
            chrome,
            document,
            settings,
            palette,
            shown: Vec::new(),
            slots: Vec::new(),
            filter: String::new(),
            placing: Placing::Idle,
            now_ms: 0,
            last_name_press: None,
            outline: None,
            folded: Vec::new(),
            hidden: Vec::new(),
            pages: Vec::new(),
            looking_at: Vec::new(),
            outline_drag: None,
            selected: None,
            placed: Vec::new(),
            selection: Vec::new(),
            drag: None,
            band: None,
            clipboard: Clipboard::new(),
            making: None,
            clash: None,
            carrying: Vec::new(),
            dragged_to: Vec::new(),
            dropping: None,
            overlay: Vec::new(),
            snapping: true,
            grid: 4,
            inspector: None,
            choosing: None,
            preview: false,
            ordering: None,
            simulated: Simulated::Own,
            names: Vec::new(),
            fired: Vec::new(),
            keyboard: denise_keyboard::Keyboard::from_system().0,
            keyboard_up: false,
            stale: false,
            history: History::new(),
            warned: false,
            resized: false,
            status: String::new(),
            // Filled by `refresh_labels`, which `show_form` below reaches.
            window_title: String::new(),
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
        self.settings.clone()
    }

    /// Records a new window size, to be written out on the way to exiting.
    pub fn remember_size(&mut self, size: Size) {
        // The stage is centred in the canvas viewport where the tree is *built*,
        // so a viewport that changed size leaves the form where the old one put
        // it -- off to one side, and clipped if the window shrank. Acted on in
        // `settle_resize`, because the tree has not seen the resize yet and the
        // new viewport is what the placement needs.
        self.resized = true;
        // The event carries the surface, which is physical; `Settings` holds a
        // window, which is logical, and hands it straight back to
        // `WindowConfig`. Without the division the remembered size grows by the
        // scale factor on every run, until `Settings::sane` stops it at 16,384.
        self.settings.width = self.scale.logical(size.width);
        self.settings.height = self.scale.logical(size.height);
    }

    /// Re-places the form after the window changed size.
    ///
    /// Called once the tree has seen the resize, so `show_form` reads the
    /// viewport the form is now to be centred in rather than the one it was.
    /// Rebuilding is what moves it: a node's layout is fixed when it is added,
    /// and the canvas scrolling cannot centre a stage that is placed off to one
    /// side.
    pub fn settle_resize(&mut self) {
        // The flag is not cleared until the form is actually placed, so a window
        // resized mid-drag settles when the pointer comes up rather than never.
        if !self.resized || self.mid_gesture() {
            return;
        }
        self.resized = false;
        self.show_form();
    }

    /// What the window's title bar should say, as of the last `refresh_labels`.
    #[must_use]
    pub fn window_title(&self) -> &str {
        &self.window_title
    }

    /// Asks the loop to stop after this frame.
    ///
    /// Unsaved work stops it the first time and says so. A modal would be the
    /// better question and needs a second window; asking twice is the honest
    /// version of it until then, and it is at least impossible to lose a form to
    /// one keystroke.
    pub fn request_exit(&mut self) {
        // Escape out of a running form goes back to designing it, which is what
        // the key does in every tool that has a preview.
        if self.preview {
            self.toggle_preview();
            return;
        }
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

    /// Every name the open form gave a node, in file order.
    pub fn outline_names(&self) -> impl Iterator<Item = &str> {
        self.placed.iter().filter_map(|node| node.name.as_deref())
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
        // The filter reads what a widget *is* as well as what it is called, so
        // "dropdown" finds `select` and "switch" finds `toggle`. Somebody
        // searching a palette is searching for a thing, not for a spelling.
        let matches = |info: &WidgetInfo| {
            needle.is_empty()
                || info.kind.contains(&needle)
                || info.doc.to_lowercase().contains(&needle)
        };

        self.shown.clear();
        let mut counts: Vec<usize> = Vec::new();
        for group in Group::ALL {
            let members: Vec<usize> = denise_ui::widgets::all()
                .iter()
                .enumerate()
                .filter(|(_, info)| info.group == group && matches(info))
                .map(|(index, _)| index)
                .collect();
            // A shelf the filter emptied is a heading over nothing.
            if members.is_empty() {
                continue;
            }
            counts.push(members.len());
            self.shown.push(Shelf::Heading(group));
            self.shown.extend(members.into_iter().map(Shelf::Widget));
        }

        match self.settings.palette {
            PaletteMode::Glyphs => self.fill_palette_glyphs(counts),
            mode => self.fill_palette_list(counts, mode == PaletteMode::Both),
        }
    }

    /// The palette as rows: a name per widget, its glyph leading when the mode
    /// says both.
    fn fill_palette_list(&mut self, counts: Vec<usize>, with_glyphs: bool) {
        let mut headings = counts.into_iter();
        let items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|row| match row {
                // Upper case and a count, because a heading that only differs
                // from a widget by being a shade dimmer is not a heading — it
                // is a row somebody will try to drag. Disabled as well, so the
                // keyboard steps over it and the pointer leaves it alone.
                Shelf::Heading(group) => ListItem::new(group.name().to_uppercase())
                    .with_trailing(headings.next().unwrap_or(0).to_string())
                    .disabled(),
                Shelf::Widget(index) => {
                    let item = ListItem::new(self.palette[*index]);
                    match denise_ui::widgets::all().get(*index) {
                        Some(info) if with_glyphs => item.with_leading_icon(info.icon),
                        _ => item,
                    }
                }
            })
            .collect();
        let rows = items.len().max(1) as i32;
        let armed = self.placing.kind().and_then(|kind| {
            self.shown
                .iter()
                .position(|row| matches!(row, Shelf::Widget(i) if self.palette[*i] == kind))
        });
        // The list's height is `rows` times the *scaled* row rather than the
        // scaled height of `rows` logical ones: at a fractional factor those two
        // differ by a pixel, and the one that matters is the one the rows add up
        // to, or the viewport scrolls a sliver past the last of them.
        let row_height = self.scale.n(PALETTE_ROW);
        // The size has to be said out loud. A widget defaults its text to 16 px
        // and knows nothing about the display, so the one list here that never
        // named a size was the one thing in the chrome that stayed 16 px while
        // everything around it doubled. `docs/design.md` calls this out as the
        // rough edge the DPI decision leaves to the application.
        let list = List::new(items, Message::Palette)
            .with_row_height(row_height)
            .with_style(TextStyle::built_in(self.scale.text(Text::Body)))
            .with_selected(armed);

        let width = self.scale.n(self.settings.left - GAP * 2);
        // Rows stacked from the top, remembered for hit-testing: the same
        // arithmetic the list draws with.
        self.slots = (0..self.shown.len().max(1))
            .map(|row| Rect::new(0, row as i32 * row_height, width, row_height))
            .collect();
        self.ui.remove(self.chrome.palette);
        self.chrome.palette = self
            .ui
            .add(
                self.chrome.palette_view,
                list,
                Rect::new(0, 0, width, rows * row_height),
            )
            .expect("the palette viewport is there");
    }

    /// The palette as tiles: every glyph in sight at once, names in tooltips.
    ///
    /// Presses never reach these buttons — design mode takes the palette's
    /// presses so a tile can start a drag, exactly as it does for rows — so
    /// the tiles are inert and the armed one is marked by its role instead.
    /// What the keyboard loses with the list, the mode gives back in height:
    /// the whole catalogue lands in a third of the rows.
    fn fill_palette_glyphs(&mut self, counts: Vec<usize>) {
        let width = self.scale.n(self.settings.left - GAP * 2);
        let row_height = self.scale.n(PALETTE_ROW);
        let tile_height = self.scale.n(TILE);
        let tile_width = width / TILE_COLUMNS;
        let armed = self.placing.kind();

        // First pass: where everything goes, in the palette's own coordinates.
        // A heading takes a full row of its own; tiles flow under it, wrapping
        // at the column count.
        self.slots.clear();
        let mut y = 0;
        let mut column = 0;
        for row in &self.shown {
            match row {
                Shelf::Heading(_) => {
                    if column > 0 {
                        y += tile_height;
                        column = 0;
                    }
                    self.slots.push(Rect::new(0, y, width, row_height));
                    y += row_height;
                }
                Shelf::Widget(_) => {
                    self.slots
                        .push(Rect::new(column * tile_width, y, tile_width, tile_height));
                    column += 1;
                    if column == TILE_COLUMNS {
                        column = 0;
                        y += tile_height;
                    }
                }
            }
        }
        if column > 0 {
            y += tile_height;
        }

        self.ui.remove(self.chrome.palette);
        let panel = self
            .ui
            .add(
                self.chrome.palette_view,
                Panel::default(),
                Rect::new(0, 0, width, y.max(1)),
            )
            .expect("the palette viewport is there");

        let mut headings = counts.into_iter();
        let inset = self.scale.n(2);
        let indent = self.scale.n(GAP);
        for (row, slot) in self.shown.clone().into_iter().zip(self.slots.clone()) {
            match row {
                Shelf::Heading(group) => {
                    // The same words the list's headings use, in the same
                    // clothes: upper case, dimmed, a count at the far end.
                    let heading = Rect::new(
                        slot.x + indent,
                        slot.y,
                        (slot.width - indent * 2).max(1),
                        slot.height,
                    );
                    self.ui.add(
                        panel,
                        Label::new(group.name().to_uppercase())
                            .with_size(self.scale.text(Text::Caption))
                            .with_role(Role::Base300),
                        heading,
                    );
                    self.ui.add(
                        panel,
                        Label::new(headings.next().unwrap_or(0).to_string())
                            .with_align(Align::End, Align::Center)
                            .with_size(self.scale.text(Text::Caption))
                            .with_role(Role::Base300),
                        heading,
                    );
                }
                Shelf::Widget(index) => {
                    let Some(info) = denise_ui::widgets::all().get(index) else {
                        continue;
                    };
                    let tile = Rect::new(
                        slot.x + inset,
                        slot.y + inset,
                        (slot.width - inset * 2).max(1),
                        (slot.height - inset * 2).max(1),
                    );
                    let button = Button::<Message>::inert("").with_icon(info.icon).with_role(
                        if armed == Some(info.kind) {
                            Role::Primary
                        } else {
                            Role::Neutral
                        },
                    );
                    if let Some(id) = self.ui.add(panel, button, tile) {
                        // The name the other modes print, and the line they
                        // put in the shared tooltip, together on the tile.
                        self.ui
                            .set_tooltip(id, format!("{} — {}", info.kind, info.doc));
                    }
                }
            }
        }
        self.chrome.palette = panel;
    }

    /// The kind a palette row stands for, or `None` for a heading.
    fn palette_kind(&self, row: usize) -> Option<&'static str> {
        match self.shown.get(row)? {
            Shelf::Heading(_) => None,
            Shelf::Widget(index) => self.palette.get(*index).copied(),
        }
    }

    /// The palette entry under a point: a row in the list modes, a tile or a
    /// heading in glyphs mode. Indexes [`Designer::shown`], whatever the mode —
    /// the slots were laid down by whichever `fill_palette` branch built the
    /// pane, so hitting and drawing cannot disagree.
    fn palette_slot(&self, at: Point) -> Option<usize> {
        let view = self.ui.bounds(self.chrome.palette_view)?;
        let scroll = self.ui.scroll(self.chrome.palette_view);
        let local = Point::new(at.x - view.x + scroll.x, at.y - view.y + scroll.y);
        self.slots.iter().position(|slot| slot.contains(local))
    }

    /// Hovers a palette row by the widget's name, and waits out the tooltip.
    ///
    /// For `--snapshot`, which has no pointer: the palette's whole answer to
    /// #126 is what appears when one rests on a row, and a picture of the
    /// palette without it is a picture of the thing that was already there.
    pub fn hover_palette(&mut self, kind: &str) -> bool {
        let Some(row) = (0..self.shown.len()).find(|row| self.palette_kind(*row) == Some(kind))
        else {
            return false;
        };
        let Some(slot) = self.slots.get(row).copied() else {
            return false;
        };
        let Some(view) = self.ui.bounds(self.chrome.palette_view) else {
            return false;
        };
        // Scrolled to, because the viewport holds eleven rows and there are
        // thirty-one: a `--hover video` that quietly hovered nothing would be a
        // picture of the palette with the pointer in the wrong place.
        let scroll = self.ui.scroll(self.chrome.palette_view);
        let wanted = scroll.y.clamp(slot.bottom() - view.height, slot.y);
        self.ui
            .set_scroll(self.chrome.palette_view, Point::new(scroll.x, wanted));
        let scroll = self.ui.scroll(self.chrome.palette_view);
        let at = Point::new(
            view.x + slot.x + slot.width / 2,
            view.y + slot.y - scroll.y + slot.height / 2,
        );
        let moved = [InputEvent::PointerMoved { position: at }];
        self.input(&moved);
        self.ui.handle(&moved);
        // Past the hover delay, which is what a person resting a pointer does.
        self.ui.tick(2_000);
        true
    }

    /// Puts what a widget *is* on the palette, for the row under the pointer.
    ///
    /// The tooltip belongs to the whole list — one node, one tooltip — so it is
    /// rewritten as the pointer moves down it. That is what the palette is for:
    /// twenty-five names tell somebody who already knows which widget they want,
    /// and this is for everybody else. See `Describe::DOC`.
    fn palette_hover(&mut self, at: Point) {
        // The tiles of glyphs mode carry their own tooltips — a glyph without
        // its name would be a riddle — so the shared one stays out of the way.
        if self.settings.palette == PaletteMode::Glyphs {
            return;
        }
        let doc = self
            .palette_slot(at)
            .and_then(|row| self.shown.get(row).copied())
            .and_then(|row| match row {
                Shelf::Heading(_) => None,
                Shelf::Widget(index) => denise_ui::widgets::all().get(index),
            })
            .map(|info| info.doc);
        match doc {
            Some(doc) => self.ui.set_tooltip(self.chrome.palette, doc),
            None => self.ui.clear_tooltip(self.chrome.palette),
        }
    }

    // ---------------------------------------------------------------- zoom

    /// A form rectangle as the tree holds it.
    ///
    /// The direction a number travels on its way *out* of the file. See
    /// [`Zoom`] for why the two directions are named rather than multiplied
    /// inline.
    fn on_screen(&self, form: Rect) -> Rect {
        self.zoom.on_screen(form)
    }

    /// A tree rectangle as the file writes it, and as the inspector shows it.
    ///
    /// The direction every number bound for the document travels. Below 100%
    /// this loses precision — see [`Zoom`] — which is why a drag keeps its own
    /// form rectangle rather than reading back what it wrote.
    fn in_form(&self, screen: Rect) -> Rect {
        self.zoom.in_form(screen)
    }

    /// A node's rectangle as the **file** has it.
    ///
    /// The tree holds it on screen, because that is where the form was built.
    /// Every command that reads a rectangle in order to write one — nudging,
    /// aligning, grouping — goes through here, and none of them mention zoom.
    fn form_layout(&self, id: NodeId) -> Option<Rect> {
        self.ui.layout(id).map(|rect| self.in_form(rect))
    }

    /// A pointer position in form coordinates, relative to the stage's corner.
    ///
    /// The stage is where the form's origin is on screen, so this is the one
    /// conversion that needs to know where the canvas has scrolled to — and it
    /// is why every caller takes the stage from `Ui::bounds` rather than
    /// remembering it.
    fn in_form_point(&self, screen: Point) -> Point {
        let stage = self.ui.bounds(self.chrome.stage).unwrap_or(Rect::ZERO);
        Point::new(
            self.zoom.in_form_n(screen.x - stage.x),
            self.zoom.in_form_n(screen.y - stage.y),
        )
    }

    /// Draws the form at another magnification, keeping everything else.
    ///
    /// The form is rebuilt because the whole subtree is in screen units, and
    /// `Form::build_scaled` is what puts it there. Nothing about the *document*
    /// changes, so this is not an edit and never touches the history.
    pub fn set_zoom(&mut self, zoom: Zoom) {
        // The display's scale is folded in here and nowhere else: every `Zoom`
        // constructor starts at 100 and would otherwise drop it, and this is
        // the one door they all come through.
        let zoom = zoom.on_device(self.scale.factor());
        if zoom == self.zoom {
            return;
        }
        self.zoom = zoom;
        self.show_form();
        self.status = format!("zoom {}", self.zoom.label());
        self.refresh_labels();
    }

    /// One step in, one step out, and back to actual size.
    pub fn zoom_in(&mut self) {
        self.set_zoom(self.zoom.wider());
    }

    /// See [`Designer::zoom_in`].
    pub fn zoom_out(&mut self) {
        self.set_zoom(self.zoom.narrower());
    }

    /// Back to one screen pixel per form pixel.
    pub fn zoom_actual(&mut self) {
        self.set_zoom(Zoom::ACTUAL);
    }

    /// As large as the canvas can show the whole form.
    pub fn zoom_to_fit(&mut self) {
        self.set_zoom(self.fitted());
    }

    /// The next step round, and back to *fit* after the widest.
    ///
    /// One control rather than three, for the same reason the theme is one: the
    /// toolbar says what the state **is**, and pressing it changes it. The
    /// keyboard has the separate ones, which is where somebody who wants a
    /// particular magnification reaches anyway.
    /// The next palette mode along: glyph-and-name, name alone, glyphs alone.
    ///
    /// Remembered in the settings, because it is a taste rather than a
    /// property of any form. Whatever was armed stays armed — the palette is
    /// rebuilt, and each mode marks the armed widget its own way.
    pub fn cycle_palette_mode(&mut self) {
        self.settings.palette = self.settings.palette.next();
        self.fill_palette();
        self.status = format!("palette: {}", self.settings.palette.name());
        self.refresh_labels();
    }

    pub fn cycle_zoom(&mut self) {
        let next = if self.zoom.is_fit() {
            Zoom::at(Zoom::STEPS[0])
        } else if self.zoom.percent() >= Zoom::STEPS[Zoom::STEPS.len() - 1] {
            self.fitted()
        } else {
            self.zoom.wider()
        };
        self.set_zoom(next);
    }

    /// What "fit" works out to for the form and canvas as they are now.
    fn fitted(&self) -> Zoom {
        let view = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        let room = Size::new(view.width.max(0) as u32, view.height.max(0) as u32);
        Zoom::to_fit(
            self.document.form().size(),
            room,
            self.scale.n(GAP),
            self.scale.factor(),
        )
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
        if let Some(surface) = self.chrome.surface.take() {
            self.ui.remove(surface);
        }
        self.selected = None;
        self.placed.clear();
        self.drag = None;
        self.band = None;
        self.carrying.clear();
        self.dragged_to.clear();
        self.dropping = None;
        self.outline_drag = None;

        let size = self.document.form().size();
        let kind = self.document.form().kind();
        // A zoom that follows the viewport is worked out again here, because
        // this is the one place that runs both when the form changes size and
        // when the window does.
        if self.zoom.is_fit() {
            self.zoom = self.fitted();
        }
        // Centred in the viewport if it fits, and at the margin if it does not —
        // the canvas scrolls, so a form larger than the window is reachable
        // rather than cropped.
        // Bounds, not layout: the canvas is docked, so its `layout` is the
        // placeholder it was added with and its `bounds` is where it ended up.
        let view = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        // On screen from here down: the stage is how big the form *looks*, and
        // everything placed against it is in the same units the form's own
        // subtree will be built in.
        let shown = self.zoom.on_screen_size(size);
        let margin = self.scale.n(GAP);
        let x = ((view.width - shown.width as i32) / 2).max(margin);
        let y = ((view.height - shown.height as i32) / 2).max(margin);
        let designed = Rect::new(x, y, shown.width as i32, shown.height as i32);

        // A form is not always the whole surface, and the kinds that are not say
        // so on the canvas rather than in a property nobody looks at. What the
        // form is shown *over* goes down first, so the form is drawn on it.
        let (backdrop, stage_rect) = match kind {
            // The surface is the form's own size and the form is the strip that
            // comes in across it — which is exactly what `width`, `height` and
            // `extent` mean for these two.
            FormKind::Drawer | FormKind::Shelf => {
                let side = self.document.form().side();
                // `extent` is a form length — how far the drawer comes in — so
                // it is drawn at the zoom the rest of the form is.
                let extent = self.zoom.on_screen_n(self.document.form().extent());
                (Some(designed), resting(designed, side, extent))
            }
            // Whatever is behind a dialog is the application's, so the canvas
            // stands in for it: dimmed, the whole way out, at the depth the
            // file asks for.
            FormKind::Dialog => (
                Some(Rect::new(
                    0,
                    0,
                    view.width.max(designed.right() + margin),
                    view.height.max(designed.bottom() + margin),
                )),
                designed,
            ),
            FormKind::Screen | FormKind::Window | FormKind::Fragment => (None, designed),
        };

        if let Some(rect) = backdrop {
            let panel = if kind == FormKind::Dialog {
                // A stand-in, and knowingly so: the toolkit dims a scene with
                // black at an alpha, and a `Panel` names a theme *role* rather
                // than a colour, so there is no role that means "whatever is
                // behind, darker". What this draws is that there **is** a
                // backdrop and where the dialog sits on it; `dim` in the
                // inspector is what says how dark it will really be.
                Panel::filled(Role::Base300)
            } else {
                Panel {
                    fill: Some(Role::Base200),
                    border: Some(Role::Base300),
                    border_width: 1,
                    radius: Radius::Box,
                    backdrop: false,
                }
            };
            self.chrome.surface = self.ui.add(self.chrome.canvas, panel, rect);
        }

        let stage = self
            .ui
            .add(
                self.chrome.canvas,
                Panel::filled(self.document.form().background()),
                stage_rect,
            )
            .expect("the canvas is there");
        self.chrome.stage = stage;

        let mut wiring = Design {
            base: self.document.base(),
            missing: Vec::new(),
            names: Vec::new(),
        };
        // The whole subtree in screen units, which is what makes the tree's own
        // hit testing, painting and handle placement need no conversion at all.
        // What needs one is every number crossing to or from the file.
        let outcome = self
            .document
            .form()
            // With the `design` blocks, which is the one caller that wants
            // them: a table drawn with no rows is not a table anybody can lay
            // out against, and the application supplies the real ones. See #160.
            .build_with_design(&mut self.ui, stage, self.zoom.factor(), &mut wiring)
            .map(|built| {
                self.placed = built.placed().to_vec();
                self.pages = built.pages().to_vec();
            });
        self.names = std::mem::take(&mut wiring.names);

        match outcome {
            Ok(()) => {
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
        self.chrome.scrim = self.ui.add(self.chrome.canvas, scrim, stage_rect);
        if let Some(id) = self.chrome.scrim {
            self.ui.set_z(id, 100);
            // Hiding it is the whole of preview mode. Nothing else about the
            // form changes: the same tree, the same widgets, the same paint.
            self.ui.set_visible(id, !self.preview);
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
        self.apply_looking_at();
        self.apply_hidden();
        self.refresh_outline();
        self.refresh_inspector();
        self.refresh_overlay();
        self.refresh_labels();
    }

    /// Redraws the outline for the tree as it now stands.
    ///
    /// Every node, not only the named ones: the canvas cannot show a node behind
    /// another, clipped out of its parent or sized to nothing, and this is where
    /// those are reached.
    fn refresh_outline(&mut self) {
        if let Some(pane) = self.outline.take() {
            self.ui.remove(pane.content);
        }
        let hides = |path: &[usize]| {
            self.document
                .form()
                .property(path, "visible")
                .is_some_and(|value| value == "#false")
        };
        let rows = outline::rows(&self.placed, &self.folded, &self.hidden, hides);
        let width = self.scale.n(self.settings.left - GAP * 2);
        let view = outline::View {
            rows: &rows,
            selection: &self.selection,
            drag: self.outline_drag.as_ref(),
            width,
            scale: self.scale,
        };
        self.outline = Some(Outline::build(&mut self.ui, self.chrome.outline_view, view));
    }

    /// Applies the designer's own hiding to the tree it has just built.
    ///
    /// Nothing here is written to the file. Design mode's hit test already skips
    /// what is not drawn, so hiding a sheet that covers the form is how whatever
    /// is under it gets clicked on.
    fn apply_hidden(&mut self) {
        let hidden = self.hidden.clone();
        for path in hidden {
            if let Some(id) = self.node_id(&path) {
                self.ui.set_visible(id, false);
            }
        }
    }

    /// Shows the tab page the designer is looking at, where that is not the one
    /// the file opens on.
    ///
    /// Nothing here is written to the file. A page that is not showing is not
    /// in the tree's order at all — nothing in it paints, answers a press or
    /// takes the caret — so this is what makes the second tab's contents
    /// reachable while the file still says the first one opens.
    fn apply_looking_at(&mut self) {
        let choices = self.looking_at.clone();
        for (strip, ordinal) in choices {
            let siblings: Vec<_> = self
                .pages
                .iter()
                .filter(|page| page.path.starts_with(&strip) && page.path.len() == strip.len() + 1)
                .map(|page| (page.id, page.ordinal))
                .collect();
            for (id, at) in siblings {
                self.ui.set_visible(id, at == ordinal);
            }
            // And the strip says the same thing. A rebuild makes it from the
            // file's `selected`, which is the tab an *application* opens on --
            // so without this a rebuilt strip highlights one tab while the page
            // below it belongs to another. `set_selected` emits nothing, so
            // agreeing is not an edit.
            let strip_id = self.node_id(&strip);
            if let Some(id) = strip_id
                && let Some(tabs) = self.ui.widget_mut::<Tabs<Message>>(id)
            {
                tabs.set_selected(ordinal);
            }
        }
    }

    /// Looks at the tab page holding `path`, if it is on a tab that is not
    /// showing.
    ///
    /// Called when something is selected, so that reaching a widget on another
    /// tab is *selecting* it — from the outline, or by name — rather than a
    /// separate gesture nobody would find.
    fn look_at_page_of(&mut self, path: &[usize]) {
        let Some(page) = self
            .pages
            .iter()
            .filter(|page| path.starts_with(&page.path))
            .max_by_key(|page| page.path.len())
        else {
            return;
        };
        let strip = page.path[..page.path.len() - 1].to_vec();
        let ordinal = page.ordinal;
        match self.looking_at.iter_mut().find(|(at, _)| *at == strip) {
            Some(held) => held.1 = ordinal,
            None => self.looking_at.push((strip, ordinal)),
        }
        self.apply_looking_at();
    }

    /// Brings up the page of a tab picked on a *running* form.
    ///
    /// [`Tabs`] owns which tab is selected and nothing else: showing the page
    /// is the host application's job, and while previewing the designer is the
    /// host. Design mode answers the same question through the selection —
    /// see [`Designer::look_at_page_of`] — and preview has no selection, so
    /// without this the strip moves and the page underneath does not.
    ///
    /// Read from the strip rather than from the message, because a `tabs` node
    /// with pages need not carry `on-change` at all, and a message says which
    /// name fired rather than which node did.
    pub fn follow_previewed_tabs(&mut self) {
        if !self.previewing() {
            return;
        }
        // Every strip that hosts pages, once each.
        let mut strips: Vec<Vec<usize>> = Vec::new();
        for page in &self.pages {
            let Some((_, strip)) = page.path.split_last() else {
                continue;
            };
            if !strips.iter().any(|held| held == strip) {
                strips.push(strip.to_vec());
            }
        }

        let mut moved = false;
        for strip in strips {
            let Some(id) = self.node_id(&strip) else {
                continue;
            };
            let Some(tabs) = self.ui.widget::<Tabs<Message>>(id) else {
                continue;
            };
            let selected = tabs.selected();
            match self.looking_at.iter_mut().find(|(at, _)| *at == strip) {
                Some(held) if held.1 == selected => continue,
                Some(held) => held.1 = selected,
                None => self.looking_at.push((strip, selected)),
            }
            moved = true;
        }
        if moved {
            self.apply_looking_at();
        }
    }

    /// Redraws the inspector for whatever is selected.
    ///
    /// Every row here comes from a descriptor — the widget's own for what the
    /// widget holds, and [`denise_forms::NODE_PROPERTIES`] for what the tree
    /// holds. This file names no widget and no property, which is what keeps a
    /// twenty-seventh widget from needing a line of it.
    fn refresh_inspector(&mut self) {
        // Rebuilt rather than reconciled: a selection change replaces every row,
        // and a row holds no state worth carrying across one.
        if let Some(inspector) = self.inspector.take() {
            self.ui.remove(inspector.content);
        }
        self.close_choice();
        let width = self.scale.n(self.settings.right);
        let paths = self.selection.clone();
        let ids: Vec<NodeId> = paths.iter().filter_map(|path| self.node_id(path)).collect();

        // Nothing selected is not nothing to edit: it is the **form**, whose own
        // properties have no node to be reached through. It is also the only
        // way to reach them — the form node is not on the canvas and not in the
        // outline, so a pane that said "nothing selected" was a pane saying the
        // form's size could not be changed.
        if ids.is_empty() {
            let form = self.document.form();
            let header = [
                (
                    String::from(denise_forms::FormKind::NAMES[form.kind() as usize]),
                    Role::Primary,
                    Text::Heading,
                ),
                (form.title().to_string(), Role::BaseContent, Text::Caption),
            ];
            let fields = self.form_fields();
            self.inspector = Some(Inspector::build(
                &mut self.ui,
                self.chrome.inspector_view,
                width,
                &header,
                &fields,
                self.scale,
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
                (kind.to_string(), Role::Primary, Text::Heading),
                // The node's own name, which is worth reading: it is what the
                // application will ask the form for.
                (name, Role::BaseContent, Text::Caption),
            ]
        } else {
            [
                (
                    format!("{} selected", ids.len()),
                    Role::Primary,
                    Text::Heading,
                ),
                (
                    String::from("What they have in common; an edit goes to all of them."),
                    Role::Base300,
                    Text::Caption,
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
            self.scale,
        ));
    }

    /// A row per property the selection has in common.
    ///
    /// With several selected that is the *intersection*, so an edit is never
    /// offered that only some of them could take.
    /// The form's own properties: the ones every form has, then the ones only
    /// this kind of form has.
    ///
    /// From the descriptors, like everything else in this pane — so changing the
    /// kind changes the rows, and adding a property to the format adds a row
    /// here without anybody editing this file.
    fn form_fields(&self) -> Vec<Field> {
        let kind = self.document.form().kind();
        std::iter::once(&TITLE)
            .chain(denise_forms::FORM_PROPERTIES)
            .chain(denise_forms::kind_properties(kind))
            .map(|property| {
                // The title is the node's argument, and a form always has one.
                let written = property.name == "title"
                    || self.document.form().property(&[], property.name).is_some();
                Field {
                    property,
                    node: false,
                    value: Some(self.form_value(property)),
                    written,
                    // The title cannot be taken away: a form with no title is
                    // not a form this format can write.
                    resettable: property.name != "title"
                        && self.document.form().property(&[], property.name).is_some(),
                    hint: None,
                    answered: None,
                    // The form node has no collection of its own.
                    items: Vec::new(),
                }
            })
            .collect()
    }

    /// What a form property is showing: what the file says, or what the form
    /// resolves it to when the file says nothing.
    ///
    /// The one place descriptor names are matched against accessors. It is a
    /// mapping that has to live somewhere, and
    /// `every_form_property_the_schema_declares_has_a_value_here` is what stops
    /// it going quietly out of date.
    fn form_value(&self, property: &Property) -> String {
        let form = self.document.form();
        let written = || form.property(&[], property.name).unwrap_or_default();
        match property.name {
            "title" => form.title().to_string(),
            "name" => form.name().unwrap_or_default().to_string(),
            "kind" => String::from(denise_forms::FormKind::NAMES[form.kind() as usize]),
            "width" => form.size().width.to_string(),
            "height" => form.size().height.to_string(),
            "theme" => form.theme_name().to_string(),
            "background" => {
                String::from(denise_ui::widgets::describe::role_name(form.background()))
            }
            "resizable" => String::from(if form.resizable() { "#true" } else { "#false" }),
            "min-width" => form
                .min_size()
                .map(|it| it.width.to_string())
                .unwrap_or_default(),
            "min-height" => form
                .min_size()
                .map(|it| it.height.to_string())
                .unwrap_or_default(),
            "dim" => form.dim().to_string(),
            "scaling" => String::from(denise_forms::Scaling::NAMES[form.scaling() as usize]),
            "side" => String::from(denise_ui::widgets::describe::side_name(form.side())),
            "extent" => form.extent().to_string(),
            _ => written(),
        }
    }

    /// Writes one of the form's own properties.
    ///
    /// Never live: the size, the kind and the theme are what the canvas is
    /// *built from*, so each of them is a rebuild rather than a value set on a
    /// widget. What is deferred is only the caret — a field being typed in must
    /// not have the form pulled out from under it mid-keystroke.
    fn commit_form(&mut self, property: &'static Property, text: &str, deferred: bool) {
        let Ok(written) = (!text.trim().is_empty())
            .then(|| self.interpret(property, text))
            .transpose()
        else {
            return;
        };
        self.complain("");

        let edit = match (property.name, written) {
            // The title is the form node's argument, and a form must have one:
            // an empty field leaves it alone rather than writing `form ""`.
            ("title", None) => return,
            ("title", Some((literal, _))) => Edit::Argument {
                path: Vec::new(),
                value: literal,
            },
            (name, None) => Edit::property(&[], name, None),
            (name, Some((literal, _))) => Edit::property(&[], name, Some(literal)),
        };
        self.edit(edit);

        if deferred {
            self.stale = true;
        } else {
            self.reload_from_document();
        }
    }

    /// The names the code behind the form answers, and which file said so —
    /// or `None` when there is no code to ask. See [`code::answered`].
    fn vocabulary(&self) -> Option<(String, Vec<String>)> {
        let link = code::read_link(self.document.path()?)?;
        let source = std::fs::read_to_string(&link.code).ok()?;
        Some((
            code::display_name(&link.code),
            code::answered(&source, link.handlers.as_deref()),
        ))
    }

    fn fields(&self, paths: &[Vec<usize>], ids: &[NodeId]) -> Vec<Field> {
        let used = self.message_names();
        let vocabulary = self.vocabulary();
        // What an event's tooltip adds under the property's own line: the
        // names this form already uses, and the names the code answers — the
        // second being what a redesign of a built application has to stay
        // inside.
        let mut lines: Vec<String> = Vec::new();
        if !used.is_empty() {
            lines.push(format!("Used in this form: {}", used.join(", ")));
        }
        if let Some((file, names)) = &vocabulary {
            lines.push(if names.is_empty() {
                format!("{file} answers no event names yet")
            } else {
                format!("Answered by {file}: {}", names.join(", "))
            });
        }
        let hint = (!lines.is_empty()).then(|| lines.join("\n"));

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
            let hint = is_event(property).then(|| hint.clone()).flatten();
            let mut field = self.field(paths, ids, property, false, hint);
            if is_event(property)
                && let Some((_, names)) = &vocabulary
            {
                // A name the code does not answer is the load error, early. No
                // name, or several nodes disagreeing, is nothing to judge.
                field.answered = field
                    .value
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| names.iter().any(|known| known == name));
            }
            fields.push(field);
        }
        // The events go last, under their own heading, whatever order each
        // widget listed them in: geometry, then look, then data, then what it
        // fires. The pane draws the heading where the first one starts.
        let (mut fields, events): (Vec<Field>, Vec<Field>) = fields
            .into_iter()
            .partition(|field| !is_event(field.property));
        fields.extend(events);
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
            // A collection is written as child nodes, so there is no entry to
            // find and none to take away: it counts as written when it holds
            // something, and it is emptied one item at a time.
            let list = property.kind.is_collection();
            resettable &= entry && !list;
            written &= if list {
                !self.items_of(path, property.name).is_empty()
            } else {
                entry || self.argument_for(path, *id, property, node).is_some()
            };
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
            // A list's items come from the file, like a message's name and an
            // asset's path: they are child nodes, not a value the widget holds.
            // With several selected they are the first one's, and editing goes
            // to that one — a list is the node's content, and merging two
            // nodes' content is not something an inspector can mean.
            items: match (property.kind, paths.first()) {
                (kind, Some(path)) if kind.is_collection() => self.items_of(path, property.name),
                _ => Vec::new(),
            },
            answered: None,
        }
    }

    /// What one node's property currently is, as a field shows it.
    fn value_of(&self, path: &[usize], id: NodeId, property: &Property, node: bool) -> String {
        if node {
            // The rectangle comes from the tree rather than the file, so the
            // four fields follow a drag on the canvas as it happens — in the
            // file's units, because that is what the row is for.
            if let Some(axis) = axis_of(property.name) {
                let rect = self.form_layout(id).unwrap_or(Rect::ZERO);
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
            // A list's row shows how many there are; the items themselves are
            // the editor, one field each.
            PropertyKind::List | PropertyKind::Placeholder => {
                let items = self.items_of(path, property.name);
                match items.len() {
                    0 => String::new(),
                    1 => format!("1 {}", property.name),
                    many => format!("{many} {}s", property.name),
                }
            }
            _ => {
                // A length the **file** writes is read from the file. The
                // builder multiplied it into the widget and the rounding is not
                // reversible: `thickness=3` at 50% is held as 2, which divides
                // back out as 4. The file still has the 3, so it answers.
                //
                // A length the file does *not* write is the widget's own
                // default, which the file cannot supply — dividing that back
                // out is exact enough for a number nobody typed, and it is
                // dimmed in the pane anyway.
                if property.pixels
                    && !self.zoom.is_unit()
                    && let Some(written) = self.document.form().property(path, property.name)
                {
                    return written;
                }
                self.ui
                    .get_property(id, property.name)
                    .map(|value| show_value(&self.unlengthened(property, value)))
                    .unwrap_or_default()
            }
        }
    }

    // --------------------------------------------------------- collections

    /// Writes back whatever item's text was changed.
    ///
    /// Compared item by item and written one at a time, so an option nobody
    /// touched keeps the comment above it and the spelling it was written with.
    /// The counts always agree here: the pane is rebuilt whenever the length
    /// changes, and only a field can have been typed into since.
    fn commit_items(&mut self, row: usize, kind: &'static str, joined: &str) {
        let Some((path, _)) = self.list_row(row) else {
            return;
        };
        let was = self.document.form().items(&path, kind);
        let now: Vec<&str> = joined.split('\n').collect();
        if now.len() != was.len() {
            return;
        }

        let mut edits = Vec::new();
        for (nth, (before, after)) in was.iter().zip(&now).enumerate() {
            if before == after {
                continue;
            }
            let Some(item) = self.document.form().item_path(&path, kind, nth) else {
                continue;
            };
            edits.push(Edit::Argument {
                path: item,
                value: Literal::text(*after),
            });
        }
        match edits.len() {
            0 => return,
            1 => self.edit(edits.remove(0)),
            _ => self.edit(Edit::Many(edits)),
        }
        // The widget is built from its children, so the canvas follows only
        // once the form is read again — and that cannot happen under the caret.
        self.stale = true;
    }

    /// The node a list row edits, and which child kind it holds.
    ///
    /// One node, never several: a list is the node's **content**, and merging
    /// two nodes' content is not something an inspector can mean. The pane
    /// already shows the first selected node's items for the same reason.
    fn list_row(&self, row: usize) -> Option<(Vec<usize>, &'static str)> {
        let pane = self.inspector.as_ref()?;
        let property = pane.rows.get(row)?.property;
        if !property.kind.is_collection() {
            return None;
        }
        Some((self.selection.first()?.clone(), property.name))
    }

    /// Appends one to a collection.
    ///
    /// Seeded with the property's own name, so the new row says what it is and
    /// can be found again — the same reason `seed` gives a dropped `label` the
    /// word "label".
    fn add_item(&mut self, row: usize) {
        let Some((path, kind)) = self.list_row(row) else {
            return;
        };
        let at = self.document.form().items(&path, kind).len();
        // Placeholder content goes in the node's `design` block rather than on
        // the node, so that no build but this one loads it. The first one
        // brings the block with it, which keeps this a single edit and so a
        // single undo.
        let (parent, text) = match self.document.form().collection_parent(&path, kind) {
            Some(parent) => (parent, format!("{kind} {:?}", kind)),
            None => (
                path,
                format!("{} {{\n    {kind} {:?}\n}}", denise_forms::DESIGN, kind),
            ),
        };
        // Past every child, not past every *item*: a node may hold more than one
        // collection — a `table` holds columns and rows — and the index an edit
        // takes is the one among children.
        let index = self.document.form().child_count(&parent);
        self.history.separate();
        self.edit(Edit::Insert {
            parent,
            index,
            text,
        });
        self.status = format!("added {kind} {}", at + 1);
        self.reload_from_document();
    }

    /// Takes one out.
    fn remove_item(&mut self, row: usize, nth: usize) {
        let Some((path, kind)) = self.list_row(row) else {
            return;
        };
        let Some(item) = self.document.form().item_path(&path, kind, nth) else {
            return;
        };
        self.history.separate();
        self.edit(Edit::remove(&item));
        self.status = format!("removed {kind} {}", nth + 1);
        self.reload_from_document();
    }

    /// Moves one earlier or later among its siblings.
    ///
    /// `Edit::Move` reaches a collection's children like any other node, and
    /// carries a comment written above one along with it — which is the whole
    /// reason the items are edited where they live rather than rewritten as a
    /// block.
    fn move_item(&mut self, row: usize, nth: usize, to: usize) {
        let Some((path, kind)) = self.list_row(row) else {
            return;
        };
        let items = self.document.form().items(&path, kind).len();
        if nth >= items || to >= items {
            return;
        }
        let (Some(from), Some(landing)) = (
            self.document.form().item_path(&path, kind, nth),
            self.document.form().item_path(&path, kind, to),
        ) else {
            return;
        };
        // Among children, which is the index `Move` takes.
        let index = *landing.last().expect("an item is a child of its node");
        self.history.separate();
        self.edit(Edit::Move {
            from,
            to: path,
            index,
        });
        self.status = format!("moved {kind} {} to {}", nth + 1, to + 1);
        self.reload_from_document();
    }

    /// The items of a node's collection, in file order.
    ///
    /// A [`PropertyKind::List`] property is named after the child nodes that
    /// *are* it — a `select`'s `option`s, a `tabs`'s `tab`s — so this reads them
    /// straight out of the document. Each item is the node's argument, which is
    /// how every collection in this format writes its text.
    fn items_of(&self, path: &[usize], name: &str) -> Vec<String> {
        self.document.form().items(path, name)
    }

    /// A property's value as the **file** writes it.
    ///
    /// `Form::build_scaled` multiplies every property measured in pixels into
    /// the widget — that is what `Property::in_pixels` is for, and it is how a
    /// magnified form gets thicker borders and larger text rather than the same
    /// ones in a bigger box. So a spinner whose file says `thickness=3` really
    /// does hold 12 at 400%, and an inspector that showed what it holds would be
    /// showing a number nobody wrote.
    ///
    /// The rectangle is not one of these: it is not a *property* of the widget
    /// but the tree's own geometry, and it goes through [`Designer::form_layout`].
    fn unlengthened(&self, property: &Property, value: Value) -> Value {
        if !property.pixels || self.zoom.is_unit() {
            return value;
        }
        match value {
            Value::Int(n) => Value::Int(self.zoom.in_form_n(n)),
            Value::Float(f) => Value::Float(f / self.zoom.factor()),
            other => other,
        }
    }

    /// The same value on its way back in, for the live preview.
    ///
    /// The row edits the file's number and the widget wants the magnified one.
    /// Without this, typing `3` into `thickness` at 400% would write 3 to the
    /// file — correctly — and then draw a hairline until the next rebuild.
    fn lengthened(&self, property: &Property, value: Value) -> Value {
        if !property.pixels || self.zoom.is_unit() {
            return value;
        }
        match value {
            // At least one, as the builder does: a border that rounded away at
            // 25% has been deleted rather than scaled. Zero stays zero, because
            // zero was somebody saying "none".
            Value::Int(n) if n != 0 => {
                let scaled = self.zoom.on_screen_n(n);
                Value::Int(if n > 0 { scaled.max(1) } else { scaled.min(-1) })
            }
            Value::Float(f) => Value::Float(f * self.zoom.factor()),
            other => other,
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

    /// Redraws everything that follows the selection.
    ///
    /// Three panes show it — the outline highlights it, the inspector describes
    /// it, the canvas draws handles round it — so changing it is one call rather
    /// than three that can be forgotten one at a time.
    fn reselected(&mut self) {
        // Selecting something on a tab that is not showing brings that page
        // into view: every selection path comes through here, so reaching a
        // widget on another tab is selecting it rather than a gesture of its
        // own. See `Designer::look_at_page_of`.
        if let Some(path) = self.selection.last().cloned() {
            self.look_at_page_of(&path);
        }
        self.refresh_outline();
        self.refresh_inspector();
        self.refresh_overlay();
    }

    fn refresh_labels(&mut self) {
        // A button that cannot do anything says so, rather than doing nothing.
        let (can_undo, can_redo) = (self.history.can_undo(), self.history.can_redo());
        self.ui.set_enabled(self.chrome.undo_button, can_undo);
        self.ui.set_enabled(self.chrome.redo_button, can_redo);

        // The button says what pressing it does, which is the other mode.
        let mode = if self.preview { "Design" } else { "Preview" };
        if let Some(button) = self
            .ui
            .widget_mut::<Button<Message>>(self.chrome.preview_button)
        {
            button.set_label(mode);
        }
        // The tab-order button says whether it is on, the way the preview
        // button does: a mode you cannot see the state of is a mode you press
        // twice.
        let ordering = self.ordering();
        if let Some(button) = self
            .ui
            .widget_mut::<Button<Message>>(self.chrome.tab_order_button)
        {
            button.set_role(if ordering {
                Role::Primary
            } else {
                Role::Neutral
            });
        }

        let theme = self.simulated.name();
        if let Some(button) = self
            .ui
            .widget_mut::<Button<Message>>(self.chrome.theme_button)
        {
            button.set_label(theme);
        }

        let palette_mode = self.settings.palette.name();
        if let Some(button) = self
            .ui
            .widget_mut::<Button<Message>>(self.chrome.mode_button)
        {
            button.set_label(palette_mode);
        }

        let zoom = self.zoom.label();
        let fitting = self.zoom.is_fit();
        if let Some(button) = self
            .ui
            .widget_mut::<Button<Message>>(self.chrome.zoom_button)
        {
            button.set_label(zoom);
            // Marked when it is following the window rather than sitting on a
            // step, because that is the state that will change by itself.
            button.set_role(if fitting {
                Role::Primary
            } else {
                Role::Neutral
            });
        }

        // Each arrange command wants a different selection, so they go grey one
        // at a time — and each says in its tooltip what it wants instead of
        // what it does.
        for (command, id) in self.chrome.arrange_buttons.clone() {
            let on = self.can_arrange(command);
            self.ui.set_enabled(id, on);
            self.ui.set_tooltip(
                id,
                if on {
                    command.what()
                } else {
                    command.needs().why()
                },
            );
        }

        let title = self.document.label();
        // The same words in the two places that name the open file. `label` is
        // documented as what goes in the title bar and had never reached one:
        // `WindowConfig::title` is read at start-up, so opening a second form
        // left the bar naming the first.
        self.window_title = format!("Denise designer — {title}");
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

    /// Whether the caret is in one of the inspector's own fields.
    ///
    /// Two things wait on this: rebuilding the canvas after an edit that changed
    /// the shape of the tree, and reading the file again after somebody else
    /// wrote it. Both replace the inspector, and replacing it under a caret
    /// throws away what was being typed.
    fn typing(&self) -> bool {
        self.ui.focused().is_some_and(|id| {
            self.inspector.as_ref().is_some_and(|pane| {
                pane.rows.iter().any(|row| {
                    matches!(row.editor, Editor::Field(_) | Editor::Slid { .. })
                        && row.editor.focusable() == id
                })
            })
        })
    }

    /// Rebuilds the canvas from the file, once the caret is out of the way.
    ///
    /// See [`Designer::stale`] for what is waiting and why it has to.
    fn settle(&mut self) {
        if !self.stale {
            return;
        }
        if self.typing() {
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

        // A list is a run of child nodes, so what came back is the items joined
        // by newlines rather than one value. Only the text of an item can arrive
        // this way — adding, removing and reordering are buttons, which carry an
        // index a polled string cannot.
        if property.kind.is_collection() {
            self.commit_items(row, property.name, &text);
            return;
        }

        let paths = self.selection.clone();
        if paths.is_empty() {
            self.commit_form(property, &text, deferred);
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
                            let value = self.lengthened(property, value.clone());
                            if let Some(Err(refused)) =
                                self.ui.set_property(id, property.name, value)
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
        // `value` was typed into the inspector, so it is a form number; the
        // tree's rectangle is on screen. The other three axes go back out
        // unchanged, which they only do if the whole rectangle makes the round
        // trip together.
        let was = self.in_form(was);
        let mut axes = [was.x, was.y, was.width, was.height];
        axes[axis] = value;
        let form = Rect::new(axes[0], axes[1], axes[2], axes[3]);
        self.ui.set_layout(id, self.on_screen(form));
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
        if let Some(rect) = self.ui.layout(id).map(|rect| self.in_form(rect)) {
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
    ///
    /// While previewing it takes almost nothing: the form is running, and every
    /// press and keystroke is its own. Only the way out is still the designer's.
    fn claim(&mut self, event: &InputEvent) -> bool {
        // The file-changed sheet, for the same reason as the new-form one below
        // — and Escape means *Keep mine*, because the safe answer to a question
        // somebody dismissed is the one that loses nothing.
        if self.clashing() {
            return matches!(
                event,
                InputEvent::Key {
                    code: KeyCode::Escape,
                    state: ElementState::Down,
                    ..
                }
            ) && {
                self.keep_mine();
                true
            };
        }
        // While the new-form sheet is up, every press belongs to it. It is a
        // modal scene *over* the canvas, and design mode reading the events
        // first would read a press on one of its buttons as a press on the form
        // behind it. Escape is the way out, as it is everywhere else here.
        if self.making() {
            return matches!(
                event,
                InputEvent::Key {
                    code: KeyCode::Escape,
                    state: ElementState::Down,
                    ..
                }
            ) && {
                self.close_new();
                self.status = String::from("no new form, then");
                self.refresh_labels();
                true
            };
        }
        if self.preview {
            return matches!(
                event,
                InputEvent::Key {
                    code: KeyCode::F5,
                    state: ElementState::Down,
                    ..
                }
            ) && {
                self.toggle_preview();
                true
            };
        }
        self.claim_designing(event)
    }

    /// Whether design mode took this event, while designing.
    fn claim_designing(&mut self, event: &InputEvent) -> bool {
        let canvas = self.ui.bounds(self.chrome.canvas).unwrap_or(Rect::ZERO);
        let palette = self
            .ui
            .bounds(self.chrome.palette_view)
            .unwrap_or(Rect::ZERO);
        let outline = self
            .ui
            .bounds(self.chrome.outline_view)
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
                if outline.contains(*position) {
                    // The outline's presses are design mode's for the same
                    // reason the palette's are: a row has three things to press,
                    // and a widget that saw the press would decide for itself
                    // which one it was.
                    self.cancel_placing();
                    self.press_outline(*position, modifiers.contains(denise::Modifiers::SHIFT));
                    return true;
                }
                if self.press_event_name(*position) {
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
                if self.outline_drag.is_some() {
                    self.drop_row();
                    return true;
                }
                if canvas.contains(*position) || self.drag.is_some() || self.band.is_some() {
                    self.release();
                    return true;
                }
                false
            }
            InputEvent::PointerMoved { position } => {
                // Before anything claims the move: the tree shows the tooltip
                // for whatever is under the pointer *after* this returns, so the
                // row's line has to be on the node by then.
                if palette.contains(*position) {
                    self.palette_hover(*position);
                }
                if self.placing.moving() {
                    self.carry_to(*position);
                    return true;
                }
                if self.outline_drag.is_some() {
                    self.drag_row_to(*position);
                    return true;
                }
                if self.drag.is_some() {
                    self.drag_to(*position);
                    return true;
                }
                if self.band.is_some() {
                    self.band_to(*position);
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

    // --------------------------------------------------------------- preview

    /// Whether the form is being run rather than drawn.
    pub const fn previewing(&self) -> bool {
        self.preview
    }

    /// Runs the form, or stops running it.
    ///
    /// The whole of the first half is **hiding the scrim** — the invisible sheet
    /// that has been absorbing every press over the form — and letting design
    /// mode stop claiming the canvas's events. The same tree, the same widgets,
    /// the same paint; what changes is who the events belong to.
    ///
    /// Going back rebuilds the form from the file, which is what puts the typed
    /// text and the toggled toggles back to what the file says. There is no
    /// snapshot of runtime state to restore, because the file is the state.
    pub fn toggle_preview(&mut self) {
        self.preview = !self.preview;

        if self.preview {
            // Nothing that belongs to designing survives into running: no
            // selection, no handles, no half-finished placement, no rename.
            self.cancel_placing();
            self.cancel_rename();
            self.close_choice();
            self.selection.clear();
            self.selected = None;
            self.fired.clear();
            self.reselected();
            if let Some(scrim) = self.chrome.scrim {
                self.ui.set_visible(scrim, false);
            }
            self.status = String::from("running the form — F5 or Escape goes back");
        } else {
            self.keyboard.close(&mut self.ui);
            self.ui.focus(None);
            // The file is the state, so this is the reset.
            self.reload_from_document();
            self.status = String::from("designing");
        }

        // The panes are not the form's, so they go grey rather than pretending
        // to work: a press on a palette row while the form was running would
        // otherwise arm a widget nobody could ever place.
        for column in self.chrome.columns {
            self.ui.set_enabled(column, !self.preview);
        }
        self.resize_log();
        self.refresh_labels();
    }

    /// Gives the log strip its height, or takes it away again.
    fn resize_log(&mut self) {
        let height = if self.preview { self.scale.n(LOG) } else { 0 };
        self.ui
            .set_layout(self.chrome.log, Rect::new(0, 0, 0, height));
        self.refresh_log();
    }

    /// Redraws the log for the messages the form has fired.
    fn refresh_log(&mut self) {
        self.ui.remove(self.chrome.log_lines);
        // `width` comes back out of the tree, so it is already physical while
        // every constant beside it is logical — the asymmetry `Scale` documents,
        // and why these scale a length at a time rather than a whole rectangle.
        let width = self.ui.bounds(self.chrome.log).unwrap_or(Rect::ZERO).width;
        let (gap, line_height) = (self.scale.n(GAP), self.scale.n(13));
        let lines = self
            .ui
            .add(
                self.chrome.log,
                Panel::default(),
                Rect::new(0, 0, width.max(1), self.scale.n(LOG).max(1)),
            )
            .expect("the log strip is there");
        self.chrome.log_lines = lines;
        if !self.preview {
            return;
        }

        if self.fired.is_empty() {
            self.ui.add(
                lines,
                Label::new("no messages yet — press something on the form")
                    .with_size(self.scale.text(Text::Caption))
                    .with_role(Role::Base300),
                Rect::new(gap, self.scale.n(4), width - gap * 2, self.scale.n(14)),
            );
            return;
        }
        // Newest last, so the eye stays at the bottom where the next one lands.
        for (index, line) in self.fired.iter().enumerate() {
            let last = index + 1 == self.fired.len();
            self.ui.add(
                lines,
                Label::new(line.clone())
                    .with_size(self.scale.text(Text::Caption))
                    .with_role(if last {
                        Role::Accent
                    } else {
                        Role::BaseContent
                    }),
                Rect::new(
                    gap,
                    self.scale.n(4) + index as i32 * line_height,
                    width - gap * 2,
                    line_height,
                ),
            );
        }
    }

    /// Notes a message the form fired.
    fn log_fired(&mut self, index: usize, value: Option<String>) {
        let name = self
            .names
            .get(index)
            .cloned()
            .unwrap_or_else(|| String::from("(more names than this build can hold)"));
        let line = match value {
            Some(value) => format!("{name}({value})"),
            None => name,
        };
        self.fired.push(line);
        // A strip, not a transcript: the oldest goes when the newest arrives.
        while self.fired.len() > LOGGED {
            self.fired.remove(0);
        }
        self.refresh_log();
    }

    /// The next theme along, applied to the whole window.
    ///
    /// The whole window, and not only the canvas, because there is one tree and
    /// one theme — which is the same reason the canvas is pixel-exact about what
    /// the panel will draw. Going back to designing is what puts the designer's
    /// own theme back.
    pub fn cycle_theme(&mut self) {
        self.simulated = self.simulated.next();
        self.apply_theme();
        self.status = String::from(self.simulated.name());
        self.refresh_labels();
    }

    fn apply_theme(&mut self) {
        let theme = match self.simulated {
            Simulated::Own => self.document.form().theme(),
            Simulated::Dark => theme::DARK,
            Simulated::Light => theme::LIGHT,
            Simulated::HighContrast => theme::HIGH_CONTRAST,
        };
        // Scaled on the way in, like the one `Designer::new` starts with. Only
        // the colours are being simulated here; the furniture is this display's
        // and stays this display's, or every theme but the first would put the
        // chrome back at half size.
        self.ui.set_theme(theme.scaled(self.scale.factor()));
    }

    /// Lets the on-screen keyboard see the events before the tree does.
    ///
    /// Its alternates gesture is answered by its own hit test — the press that
    /// opened the popup is still down on the key — so it needs them first and
    /// unedited. Only while previewing: the keyboard is part of the machine being
    /// simulated, and a designer whose own fields summoned one would be a
    /// designer you could not type a property into.
    pub fn keyboard_input(&mut self, events: &[InputEvent]) {
        if !self.preview {
            return;
        }
        let typed = self.keyboard.handle(&mut self.ui, events);
        if !typed.is_empty() {
            self.ui.handle(&typed);
        }
    }

    /// A held key, and the caret deciding whether a keyboard is wanted.
    pub fn keyboard_turn(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        if !self.preview {
            return;
        }
        let repeats = self.keyboard.tick(&mut self.ui, now_ms);
        if !repeats.is_empty() {
            self.ui.handle(&repeats);
        }
        // Focus lands on a field and the keyboard comes up; it leaves and the
        // keyboard goes. Which is the behaviour worth being able to check on the
        // form being designed, because it is the one that moves the form.
        self.keyboard.follow_focus(&mut self.ui, Message::Key);

        // Said out loud when it changes, because *how much of the form the
        // keyboard is covering* is the thing somebody is previewing to find out.
        let up = self.keyboard_open();
        if up != self.keyboard_up {
            self.keyboard_up = up;
            self.status = match self.keyboard.occluded(&self.ui) {
                Some(covered) => format!(
                    "the keyboard is up, over the bottom {} pixels of the surface",
                    covered.height
                ),
                None => String::from("the keyboard is away"),
            };
            self.refresh_labels();
        }
    }

    /// Whether the on-screen keyboard is up.
    pub const fn keyboard_open(&self) -> bool {
        self.keyboard.is_open()
    }

    // --------------------------------------------------------------- outline

    /// Which row of the outline a point is on, and where in it.
    fn outline_hit(&self, at: Point) -> Option<(usize, outline::Hit)> {
        let view = self.ui.bounds(self.chrome.outline_view)?;
        let scroll = self.ui.scroll(self.chrome.outline_view);
        let row = usize::try_from((at.y - view.y + scroll.y) / self.scale.n(outline::ROW)).ok()?;
        let pane = self.outline.as_ref()?;
        let held = pane.rows.get(row)?;
        let width = self.scale.n(self.settings.left - GAP * 2);
        Some((
            row,
            outline::hit(at.x - view.x + scroll.x, width, held, self.scale),
        ))
    }

    /// A press in the outline.
    fn press_outline(&mut self, at: Point, add: bool) {
        self.cancel_rename();
        let Some((row, hit)) = self.outline_hit(at) else {
            return;
        };
        let Some(path) = self
            .outline
            .as_ref()
            .and_then(|pane| pane.rows.get(row))
            .map(|held| held.path.clone())
        else {
            return;
        };

        match hit {
            outline::Hit::Fold => {
                if let Some(index) = self.folded.iter().position(|shut| *shut == path) {
                    self.folded.remove(index);
                } else {
                    self.folded.push(path);
                }
                self.refresh_outline();
            }
            outline::Hit::Eye => self.toggle_hidden(&path),
            outline::Hit::Body => {
                self.history.separate();
                if add {
                    if let Some(already) = self.selection.iter().position(|held| *held == path) {
                        self.selection.remove(already);
                    } else {
                        self.selection.push(path.clone());
                    }
                } else {
                    self.selection = vec![path.clone()];
                }
                self.selected = self.node_id(&path);
                self.outline_drag = Some(outline::Drag {
                    path,
                    from: at,
                    moved: false,
                    onto: None,
                    into: false,
                });
                self.reselected();
            }
        }
    }

    /// Hides a node in the designer, or shows it again.
    ///
    /// Not written to the file — the eye is the designer's own, and a form that
    /// remembered what somebody had folded away while working on it would be
    /// carrying the designer's state to the panel. Design mode's hit test
    /// already skips what is not drawn, so this is how a sheet covering the
    /// whole form stops eating every click.
    pub fn toggle_hidden(&mut self, path: &[usize]) {
        if let Some(index) = self.hidden.iter().position(|held| held == path) {
            self.hidden.remove(index);
            if let Some(id) = self.node_id(path) {
                // Back to whatever the *file* says, which may still be hidden.
                let by_file = self
                    .document
                    .form()
                    .property(path, "visible")
                    .is_some_and(|value| value == "#false");
                self.ui.set_visible(id, !by_file);
            }
        } else {
            self.hidden.push(path.to_vec());
            if let Some(id) = self.node_id(path) {
                self.ui.set_visible(id, false);
            }
        }
        self.refresh_outline();
        self.refresh_overlay();
    }

    /// The pointer moved while a row was being dragged.
    fn drag_row_to(&mut self, at: Point) {
        let Some(drag) = self.outline_drag.as_mut() else {
            return;
        };
        // A chrome threshold, so it scales; the canvas ones do not, because a
        // pointer pixel there *is* a form pixel.
        if !drag.moved
            && (at.y - drag.from.y).abs() + (at.x - drag.from.x).abs() < self.scale.n(THRESHOLD)
        {
            return;
        }
        drag.moved = true;
        let from = drag.path.clone();
        let (onto, into) = self.landing(at, &from);
        let Some(drag) = self.outline_drag.as_mut() else {
            return;
        };
        drag.onto = onto;
        drag.into = into;
        self.refresh_outline();
    }

    /// Where a drop at `at` would put the node now at `from`.
    ///
    /// Beside the row under the pointer, or *inside* it when the pointer is over
    /// the middle of something that can hold children — the same distinction the
    /// canvas draws, and with the same [`denise_forms::owns_children`].
    fn landing(&self, at: Point, from: &[usize]) -> (Option<(Vec<usize>, usize)>, bool) {
        let Some(view) = self.ui.bounds(self.chrome.outline_view) else {
            return (None, false);
        };
        let scroll = self.ui.scroll(self.chrome.outline_view);
        let y = at.y - view.y + scroll.y;
        let Some(pane) = self.outline.as_ref() else {
            return (None, false);
        };
        if pane.rows.is_empty() {
            return (None, false);
        }

        let row = (y / outline::ROW).clamp(0, pane.rows.len() as i32 - 1) as usize;
        let within = y - row as i32 * outline::ROW;
        let target = &pane.rows[row];

        // A node cannot go inside itself, so there is nothing to draw.
        if target.path.starts_with(from) {
            return (None, false);
        }

        // The middle half of a container means *into* it; the edges mean beside.
        let inside = denise_forms::owns_children(target.kind)
            && within > outline::ROW / 4
            && within < outline::ROW * 3 / 4;
        if inside {
            return (
                Some((target.path.clone(), self.child_count(&target.path))),
                true,
            );
        }

        let Some((&index, parent)) = target.path.split_last() else {
            return (None, false);
        };
        let after = within >= outline::ROW / 2;
        (Some((parent.to_vec(), index + usize::from(after))), false)
    }

    /// The pointer came up on a row being dragged.
    fn drop_row(&mut self) {
        let Some(drag) = self.outline_drag.take() else {
            return;
        };
        let Some((parent, index)) = drag.onto.clone().filter(|_| drag.moved) else {
            // A press that never travelled was a selection.
            self.refresh_outline();
            return;
        };
        // Already there: nothing to do, and nothing to write.
        if let Some((&was, above)) = drag.path.split_last()
            && above == parent.as_slice()
            && (index == was || index == was + 1)
        {
            self.refresh_outline();
            return;
        }

        // Where it will be: taking it out shifts everything after it, the
        // destination included.
        let mut landed =
            denise_forms::after_removing(&parent, &drag.path).unwrap_or(parent.clone());
        landed.push(index);

        self.history.separate();
        self.edit(Edit::Move {
            from: drag.path,
            to: parent,
            index,
        });
        // The node is somewhere else now, so the selection follows it there.
        self.selection = vec![landed];
        self.reload_from_document();
    }

    /// Renames the selected node, in the outline, in place.
    pub fn begin_rename(&mut self) {
        self.cancel_rename();
        let Some(path) = self.selection.last().cloned() else {
            return;
        };
        let Some((row, depth, content)) = self.outline.as_ref().and_then(|pane| {
            let row = pane.row_of(&path)?;
            Some((row, pane.rows[row].depth as i32, pane.content))
        }) else {
            return;
        };

        let width = self.settings.left - GAP * 2;
        let x = depth * 10 + 13;
        // Logical throughout, so the whole rectangle scales at once — and the
        // row height has to be the scaled one the pane laid the rows out with.
        let row_height = self.scale.n(outline::ROW);
        let Some(id) = self.ui.add(
            content,
            TextInput::<Message>::new()
                .with_size(self.scale.text(Text::Body))
                .with_max_chars(64)
                .with_placeholder("name")
                .with_submit(Message::Renamed),
            Rect::new(
                self.scale.n(x),
                row as i32 * row_height,
                self.scale.n(width - x),
                row_height,
            ),
        ) else {
            return;
        };
        let current = self.document.form().property(&path, "name");
        if let Some(input) = self.ui.widget_mut::<TextInput<Message>>(id) {
            input.set_text(current.unwrap_or_default());
        }
        self.ui.focus(Some(id));
        if let Some(pane) = self.outline.as_mut() {
            pane.renaming = Some((row, id));
        }
        self.status = String::from("type a name; Enter keeps it, Escape does not");
        self.refresh_labels();
    }

    /// Writes what was typed into the rename field.
    fn finish_rename(&mut self) {
        let Some((row, field)) = self.outline.as_mut().and_then(|pane| pane.renaming.take()) else {
            return;
        };
        let text = self
            .ui
            .widget::<TextInput<Message>>(field)
            .map(|input| input.text().trim().to_string())
            .unwrap_or_default();
        let path = self
            .outline
            .as_ref()
            .and_then(|pane| pane.rows.get(row))
            .map(|held| held.path.clone());
        self.ui.remove(field);
        self.ui.focus(None);
        let Some(path) = path else {
            return;
        };

        let was = self.document.form().property(&path, "name");
        let now = (!text.is_empty()).then_some(text);
        if was == now {
            self.refresh_outline();
            return;
        }
        self.history.separate();
        self.edit(Edit::property(
            &path,
            "name",
            now.map(denise_forms::Literal::name),
        ));
        // A name is what the tree knows a node by, so it has to be built again.
        self.reload_from_document();
    }

    /// Gives up a rename without writing it.
    fn cancel_rename(&mut self) {
        if let Some((_, field)) = self.outline.as_mut().and_then(|pane| pane.renaming.take()) {
            self.ui.remove(field);
            self.ui.focus(None);
        }
    }

    /// Whether a rename is being typed.
    fn renaming(&self) -> bool {
        self.outline
            .as_ref()
            .is_some_and(|pane| pane.renaming.is_some())
    }

    // --------------------------------------------------------------- placing

    /// A press on a palette row: a click until the pointer travels.
    fn press_palette(&mut self, at: Point) {
        // A heading has no kind, so pressing one arms nothing — which is also
        // what `ListItem::disabled` would have decided if the list saw presses.
        let Some(kind) = self.palette_slot(at).and_then(|row| self.palette_kind(row)) else {
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
                // The ghost is how big the widget will *look* once it lands, so
                // it is the default size at the canvas's magnification.
                let size = self.zoom.on_screen_size(denise_forms::default_size(kind));
                let rect = Rect::new(at.x, at.y, size.width as i32, size.height as i32);
                let Some(ghost) = self.add_ghost(kind, rect) else {
                    return;
                };
                self.placing = Placing::Carrying { kind, ghost };
            }
            Placing::Carrying { kind, ghost } => {
                let size = self.zoom.on_screen_size(denise_forms::default_size(kind));
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
        // Caption, and the display's size rather than the form's: like the
        // grab handles, this decorates the canvas rather than living on it.
        self.ui.add(
            ghost,
            Label::new(kind)
                .with_size(self.scale.text(Text::Caption))
                .with_role(Role::Accent),
            self.scale.r(Rect::new(2, 2, 200, Text::Caption.line())),
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

        let parent = self.container_at(corner, &[]);
        let origin = parent
            .as_ref()
            .and_then(|path| self.path_bounds(path))
            .unwrap_or(stage);
        // Screen on the way in, because that is where the pointer left it, and
        // form on the way out, because that is what the file takes. Snapping
        // happens after the conversion: the grid is the file's, not the
        // screen's, so a widget dropped at 200% still lands on a multiple of
        // four.
        let mut rect = self.in_form(Rect::new(
            screen.x - origin.x,
            screen.y - origin.y,
            screen.width,
            screen.height,
        ));
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
    /// missed. A `collapse` does hold children, so a drop into one lands in it
    /// — closed or open, since what is drawn is not what the file says.
    ///
    /// `skip` names subtrees to look straight through: a node being dragged is
    /// under the pointer by definition, and a node cannot be dropped into
    /// itself.
    fn container_at(&self, at: Point, skip: &[Vec<usize>]) -> Option<Vec<usize>> {
        let containers: Vec<Placed> = self
            .placed
            .iter()
            .filter(|node| denise_forms::owns_children(node.kind))
            .filter(|node| !skip.iter().any(|path| node.path.starts_with(path)))
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
            && let Some(grip) = Grip::at(bounds, at, self.scale.n(canvas::HANDLE))
            && matches!(grip, Grip::Resize { .. })
        {
            self.begin(grip, at, path);
            return;
        }

        // While the tab order is showing, a press on the canvas is a place in
        // the sequence rather than a selection or a drag.
        if self.ordering.is_some() {
            self.pick_tab_stop(at);
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

        // A press with nothing to take hold of draws a band instead: the bare
        // canvas, or the background of a container that is *already* held. A
        // panel is a thing before it is a surface — the first press takes hold
        // of it, and once it is held its background is somewhere to band over.
        let banding = match &hit {
            None => Some(Vec::new()),
            Some(path) if self.is_container(path) && self.selection.contains(path) => {
                Some(path.clone())
            }
            _ => None,
        };
        if let Some(scope) = banding {
            // An empty canvas gives up the selection at once, as it always has.
            // A band inside a container waits until it has travelled, so a press
            // that goes nowhere leaves the container held.
            if hit.is_none() && !add {
                self.selection.clear();
                self.selected = None;
                self.reselected();
            }
            self.begin_band(scope, at, add);
            return;
        }
        let path = hit.expect("anything else drew a band");

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
        self.reselected();

        if let Some(bounds) = self.path_bounds(&path)
            && let Some(grip) = Grip::at(bounds, at, self.scale.n(canvas::HANDLE))
        {
            self.begin(grip, at, path);
        }
        self.refresh_overlay();
    }

    fn begin(&mut self, grip: Grip, at: Point, path: Vec<usize>) {
        // A drag is its own step, whatever was being nudged before it.
        self.history.separate();
        // In form coordinates from here: what a drag edits is the file, and
        // the tree gets shown the result.
        let Some(origin) = self
            .node_id(&path)
            .and_then(|id| self.ui.layout(id))
            .map(|rect| self.in_form(rect))
        else {
            return;
        };
        // Everything else selected comes along, at the offset it already has.
        // Only for a move: a resize takes hold of one node's edge, and there is
        // no sense in which several nodes share one.
        self.carrying = if matches!(grip, Grip::Move) {
            self.selection
                .iter()
                .filter(|other| **other != path)
                .filter_map(|other| {
                    let was = self.node_id(other).and_then(|id| self.ui.layout(id))?;
                    Some((other.clone(), self.in_form(was)))
                })
                .collect()
        } else {
            Vec::new()
        };
        self.drag = Some(Drag {
            grip,
            from: at,
            to: at,
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
        drag.to = to;
        let drag = drag.clone();
        let Some(id) = self.node_id(&drag.path) else {
            return;
        };

        let siblings = self.siblings_of(&drag.path);
        // `place` works entirely in form coordinates — the grid it snaps to is
        // the grid the file records, and the edges it lines up with are the ones
        // the file gives. So the pointer is converted on the way in and the
        // answer on the way out, and zoom never reaches the arithmetic.
        let in_form = Drag {
            from: self.in_form_point(drag.from),
            to: self.in_form_point(drag.to),
            ..drag.clone()
        };
        let landed = self.in_form_point(to);
        let placement = place(&in_form, landed, &siblings, self.grid, self.snapping);
        // Where it would land if the button came up here. Worked out on every
        // step so the canvas can draw it: a reparent is never a surprise.
        // `to` and not `landed`, because this asks what is under the *pointer*.
        self.dropping = matches!(drag.grip, Grip::Move)
            .then(|| self.reparent_target(to, &drag.path))
            .flatten();

        // The tree moves so the person can see it. The *file* does not, until
        // the button comes up: one drag is one edit, which is what keeps a move
        // to a one-line diff and will make it one undo step.
        self.ui.set_layout(id, self.on_screen(placement.rect));
        let (dx, dy) = (
            placement.rect.x - drag.origin.x,
            placement.rect.y - drag.origin.y,
        );
        self.dragged_to = vec![(drag.path.clone(), placement.rect)];
        for (path, origin) in self.carrying.clone() {
            let moved = Rect::new(origin.x + dx, origin.y + dy, origin.width, origin.height);
            if let Some(id) = self.node_id(&path) {
                self.ui.set_layout(id, self.on_screen(moved));
            }
            self.dragged_to.push((path, moved));
        }
        self.refresh_overlay_with(&placement.guides, &drag.path);
        self.sync_rect();
        let landing = match self.dropping.clone() {
            None => String::new(),
            Some(parent) if parent.is_empty() => String::from(" — onto the form"),
            Some(parent) => format!(" — into {}", self.describe(&parent)),
        };
        self.status = format!(
            "{} {},{} {}x{}{landing}",
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
        if self.band.is_some() {
            self.drop_band();
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.dropping = None;
        if !drag.moved {
            // A press that never moved was a selection, and a selection must not
            // touch the file.
            self.refresh_overlay();
            return;
        }
        // A move that ended over a different container is a reparent, which is
        // a different edit: the node changes place in the *tree*, not only its
        // numbers.
        if matches!(drag.grip, Grip::Move)
            && let Some(parent) = self.reparent_target(drag.to, &drag.path)
        {
            self.reparent(parent);
            return;
        }
        // The primary and everything it carried, as one edit: one gesture is
        // one step to put back.
        let mut moved: Vec<(Vec<usize>, Rect, Rect)> = Vec::new();
        // What the drag decided, not what the tree was shown: below 100% zoom
        // those differ, and the file must have the former. See `dragged_to`.
        let landed = std::mem::take(&mut self.dragged_to);
        for (path, was) in std::iter::once((drag.path.clone(), drag.origin))
            .chain(std::mem::take(&mut self.carrying))
        {
            if let Some(rect) = landed
                .iter()
                .find(|(other, _)| *other == path)
                .map(|(_, rect)| *rect)
            {
                moved.push((path, rect, was));
            }
        }
        self.write_rects(&moved);
        self.refresh_overlay();
    }

    // ------------------------------------------------------------- the band

    /// Starts a rubber band over `scope`'s children.
    fn begin_band(&mut self, scope: Vec<usize>, at: Point, add: bool) {
        self.history.separate();
        self.band = Some(Band {
            from: at,
            to: at,
            scope,
            kept: if add {
                self.selection.clone()
            } else {
                Vec::new()
            },
            moved: false,
        });
    }

    /// The band followed the pointer: whatever it now encloses is selected.
    fn band_to(&mut self, to: Point) {
        let Some(band) = self.band.as_mut() else {
            return;
        };
        if !band.moved && (to.x - band.from.x).abs() + (to.y - band.from.y).abs() < THRESHOLD {
            return;
        }
        band.moved = true;
        band.to = to;
        let band = band.clone();

        let mut selection = band.kept.clone();
        for path in self.enclosed(&band) {
            if !selection.contains(&path) {
                selection.push(path);
            }
        }
        self.selection = selection;
        self.selected = self.selection.last().and_then(|path| self.node_id(path));
        // Every pane that follows the selection follows it here too, on every
        // step: a band is a selection being made, and watching the inspector
        // fill in is most of how somebody knows what they have caught.
        self.reselected();
        self.say_selection();
    }

    /// The scope's direct children that the band has taken.
    ///
    /// Direct children only, and never past one: see [`Band`]. Drawn ones only,
    /// for the same reason a click does not find a node that is not drawn.
    fn enclosed(&self, band: &Band) -> Vec<Vec<usize>> {
        let depth = band.scope.len() + 1;
        self.placed
            .iter()
            .filter(|node| node.path.len() == depth && node.path.starts_with(&band.scope))
            .filter(|node| {
                self.ui.visible(node.id) && self.ui.bounds(node.id).is_some_and(|it| band.takes(it))
            })
            .map(|node| node.path.clone())
            .collect()
    }

    /// The pointer came up on a band.
    fn drop_band(&mut self) {
        let Some(band) = self.band.take() else {
            return;
        };
        if !band.moved {
            // A band that never travelled is not a band, and leaves the
            // selection exactly as it found it.
            self.refresh_overlay();
            return;
        }
        // A band that took nothing inside a container leaves the container
        // held: it is still the thing being worked on, and giving it up would
        // make banding inside it a one-way door.
        if self.selection.is_empty() && !band.scope.is_empty() {
            self.selection = vec![band.scope];
            self.selected = self.selection.last().and_then(|path| self.node_id(path));
        }
        self.reselected();
        self.say_selection();
    }

    /// Gives up a band without changing anything.
    fn cancel_band(&mut self) {
        let Some(band) = self.band.take() else {
            return;
        };
        self.selection = band.kept;
        self.selected = self.selection.last().and_then(|path| self.node_id(path));
        self.reselected();
    }

    /// Says how much is selected, in the status line.
    fn say_selection(&mut self) {
        self.status = match self.selection.as_slice() {
            [] => String::from("nothing selected"),
            [one] => format!("selected {}", self.describe(one)),
            many => format!("{} selected", many.len()),
        };
        self.refresh_labels();
    }

    /// What to call a node in a sentence.
    fn describe(&self, path: &[usize]) -> String {
        self.placed
            .iter()
            .find(|node| node.path == path)
            .map_or_else(
                || String::from("the form"),
                |node| {
                    node.name.as_deref().map_or_else(
                        || format!("a `{}`", node.kind),
                        |name| format!("`{}` {name}", node.kind),
                    )
                },
            )
    }

    // -------------------------------------------------------- reparenting

    /// Whether a node can hold children of its own.
    fn is_container(&self, path: &[usize]) -> bool {
        self.placed
            .iter()
            .any(|node| node.path == path && denise_forms::owns_children(node.kind))
    }

    /// Which container a drag ending at `at` would drop the node into, when that
    /// is not the one it is already in.
    ///
    /// A drop on something that cannot hold children targets whatever holds
    /// *it*, which is what [`Self::container_at`] answers — so dropping a button
    /// on a button inside a panel puts it in the panel, and not in a button.
    fn reparent_target(&self, at: Point, from: &[usize]) -> Option<Vec<usize>> {
        let stage = self.ui.bounds(self.chrome.stage)?;
        if !stage.contains(at) {
            return None;
        }
        // With several selected they all go, which only has an answer when they
        // all came from the same place: see [`Self::move_into`].
        let going = self.siblings_selected()?;
        // A node dragged out over nothing lands on the form itself.
        let to = self.container_at(at, &going).unwrap_or_default();
        let (_, parent) = from.split_last()?;
        (parent != to.as_slice()).then_some(to)
    }

    /// Moves the selection into another container, without letting it appear
    /// to move.
    ///
    /// The rectangle in the file is relative to the parent, so a node that keeps
    /// its place on screen has to be given different numbers — which is why this
    /// is two edits per node and not one, and why they all go in as a single
    /// [`Edit::Many`] so that undo puts the whole drop back at once.
    fn reparent(&mut self, to: Vec<usize>) {
        let Some(paths) = self.siblings_selected() else {
            return;
        };
        let stage = self.ui.bounds(self.chrome.stage).unwrap_or(Rect::ZERO);
        let origin = if to.is_empty() {
            stage
        } else {
            self.path_bounds(&to).unwrap_or(stage)
        };

        let mut moves: Vec<(Vec<usize>, Rect)> = Vec::new();
        for path in &paths {
            let Some(bounds) = self.path_bounds(path) else {
                return;
            };
            // As in `insert_widget`: the tree's rectangles are on screen and
            // the file's are not, and the grid belongs to the file.
            let mut rect = self.in_form(Rect::new(
                bounds.x - origin.x,
                bounds.y - origin.y,
                bounds.width,
                bounds.height,
            ));
            if self.snapping {
                rect = snap(rect, self.grid);
            }
            moves.push((path.clone(), rect));
        }

        // Last of their new siblings, which is the front: a node dropped onto a
        // panel is put on top of it and not under it.
        let held = self.child_count(&to);
        let (edits, landing) = self.move_into(&moves, &to);

        self.history.separate();
        self.edit(Edit::Many(edits));
        self.selection = (0..moves.len())
            .map(|step| {
                let mut path = landing.clone();
                path.push(held + step);
                path
            })
            .collect();
        self.dropping = None;
        self.reload_from_document();
        self.say_selection();
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

    /// Writes rectangles back to the document, all of them as **one** edit.
    ///
    /// One edit for the whole rectangle, so a drag that moved *and* resized is
    /// one step; and one edit for the whole selection, because a group move and
    /// a nudge with several selected are one gesture. Per-node edits would be
    /// one step each, and putting a move back would take as many presses as
    /// there were nodes. A single property on its own stays a single `Number`,
    /// which is what lets a run of nudges coalesce.
    fn write_rects(&mut self, rects: &[(Vec<usize>, Rect, Rect)]) {
        let edits: Vec<Edit> = rects
            .iter()
            .flat_map(|(path, rect, was)| rect_edits(path, *rect, *was))
            .collect();
        match edits.len() {
            0 => {}
            1 => self.edit(edits.into_iter().next().expect("one")),
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
            if self.selection.is_empty() || self.renaming() {
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
        // The clipboard is the *nodes'*, not the caret's: while something in the
        // designer's own chrome is being typed into, these belong to the field
        // and this build has nothing to give it. Better to do nothing than to
        // copy a node somebody was not looking at.
        if command && !typing {
            match code {
                KeyCode::C => {
                    self.copy();
                    return true;
                }
                KeyCode::X => {
                    self.cut();
                    return true;
                }
                KeyCode::V => {
                    self.paste();
                    return true;
                }
                KeyCode::D => {
                    self.duplicate();
                    return true;
                }
                _ => {}
            }
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter
                if matches!(self.placing, Placing::Armed { .. }) =>
            {
                self.place_armed();
                true
            }
            KeyCode::F5 => {
                self.toggle_preview();
                true
            }
            KeyCode::F2 if !self.selection.is_empty() => {
                self.begin_rename();
                true
            }
            KeyCode::Escape if self.renaming() => {
                self.cancel_rename();
                self.refresh_outline();
                true
            }
            KeyCode::Escape if self.band.is_some() => {
                self.cancel_band();
                true
            }
            KeyCode::PageUp if !self.selection.is_empty() && !typing => {
                self.restack(true);
                true
            }
            KeyCode::PageDown if !self.selection.is_empty() && !typing => {
                self.restack(false);
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
            // The magnification shortcuts every drawing program has. Guarded on
            // `command` so that typing a `0` or a `-` into a property field is
            // still typing one.
            KeyCode::Equal | KeyCode::NumpadAdd if command => {
                self.zoom_in();
                true
            }
            KeyCode::Minus | KeyCode::NumpadSubtract if command => {
                self.zoom_out();
                true
            }
            KeyCode::Digit0 | KeyCode::Numpad0 if command => {
                self.zoom_actual();
                true
            }
            KeyCode::Digit9 | KeyCode::Numpad9 if command => {
                self.zoom_to_fit();
                true
            }
            KeyCode::Escape if !self.selection.is_empty() => {
                self.selection.clear();
                self.selected = None;
                self.reselected();
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
        let mut moved: Vec<(Vec<usize>, Rect, Rect)> = Vec::new();
        for path in self.selection.clone() {
            let Some(id) = self.node_id(&path) else {
                continue;
            };
            let Some(was) = self.form_layout(id) else {
                continue;
            };
            // Form pixels: an arrow key moves a node one pixel in the *file*,
            // whatever the canvas is showing.
            let rect = Rect::new(was.x + dx, was.y + dy, was.width, was.height);
            self.ui.set_layout(id, self.on_screen(rect));
            moved.push((path, rect, was));
        }
        self.write_rects(&moved);
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

    // ------------------------------------------------- making a new form

    /// Puts up the sheet that asks what the new form is.
    ///
    /// A modal scene of the designer's own, which is why design mode stops
    /// claiming input while it is up: the canvas is behind it, and a press on
    /// the sheet must not be read as a press on the form.
    pub fn begin_new(&mut self) {
        if self.making.is_some() {
            return;
        }
        const WIDE: i32 = 560;
        const TALL: i32 = 280;
        const ROW: i32 = 30;

        // The scene is the top one from here until it goes, which is what
        // `close_new` pops: a modal over a modal is not a thing this designer
        // does.
        let scene = self.ui.push_scene(160);
        // The card's own children are logical and scale together; the sheet is
        // placed against the window, which is already physical.
        let scale = self.scale;
        let s = |rect: Rect| scale.r(rect);
        let px = |text: Text| scale.text(text);
        let size = self.settings_size();
        let (wide, tall) = (scale.n(WIDE), scale.n(TALL));
        let sheet = Rect::new(
            (size.width as i32 - wide) / 2,
            (size.height as i32 - tall) / 3,
            wide,
            tall,
        );
        let card = self
            .ui
            .add(
                scene,
                Panel {
                    fill: Some(Role::Base100),
                    border: Some(Role::Base300),
                    border_width: 1,
                    radius: Radius::Box,
                    backdrop: false,
                },
                sheet,
            )
            .expect("a scene root takes children");

        let label = |ui: &mut Ui<Message>, text: &str, rect: Rect, role, size| {
            ui.add(
                card,
                Label::new(text).with_size(px(size)).with_role(role),
                rect,
            )
        };
        label(
            &mut self.ui,
            "New form",
            s(Rect::new(GAP * 2, GAP * 2, 300, 22)),
            Role::Primary,
            Text::Title,
        );
        label(
            &mut self.ui,
            "What is it?",
            s(Rect::new(GAP * 2, 48, 300, 16)),
            Role::Base300,
            Text::Caption,
        );

        // A button each rather than a dropdown: there are six, they all fit,
        // and a person choosing what to make should be able to see what the
        // choices are.
        let mut kinds = Vec::new();
        let mut x = GAP * 2;
        for (index, name) in denise_forms::FormKind::NAMES.iter().enumerate() {
            let width = 8 * name.len() as i32 + 18;
            if let Some(id) = self.ui.add(
                card,
                Button::new(*name, Message::NewKind(index))
                    .with_role(if index == 0 {
                        Role::Primary
                    } else {
                        Role::Neutral
                    })
                    .with_size(px(Text::Body)),
                s(Rect::new(x, 68, width, 26)),
            ) {
                kinds.push(id);
            }
            x += width + 5;
        }
        let note = label(
            &mut self.ui,
            FormKind::Screen.what(),
            s(Rect::new(GAP * 2, 100, WIDE - GAP * 4, 16)),
            Role::Base300,
            Text::Caption,
        )
        .expect("the card is there");

        label(
            &mut self.ui,
            "How big?",
            s(Rect::new(GAP * 2, 130, 300, 16)),
            Role::Base300,
            Text::Caption,
        );
        let mut x = GAP * 2;
        for (index, (name, _, _)) in PRESETS.iter().enumerate() {
            let width = 8 * name.len() as i32 + 18;
            self.ui.add(
                card,
                Button::new(*name, Message::NewSize(index))
                    .with_role(Role::Neutral)
                    .with_size(px(Text::Body)),
                s(Rect::new(x, 150, width, 26)),
            );
            x += width + 5;
        }

        let field = |ui: &mut Ui<Message>, x: i32, text: &str| {
            let id = ui.add(
                card,
                TextInput::<Message>::new().with_size(px(Text::Body)),
                s(Rect::new(x, 186, 90, ROW)),
            )?;
            if let Some(input) = ui.widget_mut::<TextInput<Message>>(id) {
                input.set_text(text.to_string());
            }
            Some(id)
        };
        label(
            &mut self.ui,
            "width",
            s(Rect::new(GAP * 2, 192, 44, 16)),
            Role::BaseContent,
            Text::Caption,
        );
        let width = field(&mut self.ui, GAP * 2 + 48, "800").expect("the card is there");
        label(
            &mut self.ui,
            "height",
            s(Rect::new(GAP * 2 + 150, 192, 50, 16)),
            Role::BaseContent,
            Text::Caption,
        );
        let height = field(&mut self.ui, GAP * 2 + 204, "480").expect("the card is there");

        self.ui.add(
            card,
            Button::new("Cancel", Message::Never)
                .with_role(Role::Neutral)
                .with_size(px(Text::Body)),
            s(Rect::new(WIDE - 200, TALL - 46, 88, 32)),
        );
        self.ui.add(
            card,
            Button::new("Create", Message::Create)
                .with_role(Role::Primary)
                .with_size(px(Text::Body)),
            s(Rect::new(WIDE - 104, TALL - 46, 88, 32)),
        );

        self.making = Some(Making {
            kind: FormKind::Screen,
            kinds,
            note,
            width,
            height,
        });
        self.status = String::from("what kind of form is it?");
        self.refresh_labels();
    }

    /// The window's size, which the sheet is centred in.
    fn settings_size(&self) -> Size {
        self.ui
            .bounds(self.ui.root())
            .map_or(Size::new(1280, 800), |rect| {
                Size::new(rect.width.max(1) as u32, rect.height.max(1) as u32)
            })
    }

    /// A kind was picked: the button that was pressed becomes the one that
    /// looks pressed, and the line under them says what it is for.
    pub fn choose_kind(&mut self, index: usize) {
        let Some(kind) = [
            FormKind::Screen,
            FormKind::Window,
            FormKind::Dialog,
            FormKind::Drawer,
            FormKind::Shelf,
            FormKind::Fragment,
        ]
        .get(index)
        .copied() else {
            return;
        };
        let Some(making) = self.making.as_mut() else {
            return;
        };
        making.kind = kind;
        let (kinds, note) = (making.kinds.clone(), making.note);
        for (nth, id) in kinds.iter().enumerate() {
            if let Some(button) = self.ui.widget_mut::<Button<Message>>(*id) {
                button.set_role(if nth == index {
                    Role::Primary
                } else {
                    Role::Neutral
                });
            }
        }
        let what = kind.what();
        if let Some(label) = self.ui.widget_mut::<Label>(note) {
            label.set_text(what);
        }
    }

    /// A size preset was picked: it goes into the two fields, which somebody
    /// may still type over.
    pub fn choose_size(&mut self, index: usize) {
        let Some(&(_, width, height)) = PRESETS.get(index) else {
            return;
        };
        let Some(making) = self.making.as_ref() else {
            return;
        };
        let (into_width, into_height) = (making.width, making.height);
        for (id, value) in [(into_width, width), (into_height, height)] {
            if let Some(field) = self.ui.widget_mut::<TextInput<Message>>(id) {
                field.set_text(value.to_string());
            }
        }
    }

    /// Makes the form the sheet describes.
    pub fn create(&mut self) {
        let Some(making) = self.making.as_ref() else {
            return;
        };
        let read = |ui: &Ui<Message>, id: NodeId, fallback: u32| {
            ui.widget::<TextInput<Message>>(id)
                .and_then(|field| field.text().trim().parse::<u32>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(fallback)
        };
        let size = Size::new(
            read(&self.ui, making.width, 800).clamp(16, 8192),
            read(&self.ui, making.height, 480).clamp(16, 8192),
        );
        let kind = making.kind;

        self.close_new();
        self.document = Document::of(kind, size);
        self.history = History::new();
        self.warned = false;
        self.selection.clear();
        self.show_form();
        self.status = format!(
            "a new {} — {}x{}",
            denise_forms::FormKind::NAMES[kind as usize],
            size.width,
            size.height
        );
        self.refresh_labels();
    }

    /// Puts the sheet away, whether it was answered or not.
    pub fn close_new(&mut self) {
        if self.making.take().is_some() {
            self.ui.pop_scene();
        }
    }

    /// Whether the new-form sheet is up.
    pub const fn making(&self) -> bool {
        self.making.is_some()
    }

    // ---------------------------------------------------- the tab order

    /// Turns the tab-order overlay on, or off again.
    ///
    /// See [`Ordering`] for what it is and why re-sequencing is per parent.
    pub fn toggle_tab_order(&mut self) {
        if self.ordering.take().is_some() {
            self.status = String::from("designing");
            self.reselected();
            self.refresh_labels();
            return;
        }
        // Not while the form is running: the numbers are about the file, and
        // preview mode is about the form. F5 is the way out of that one.
        if self.preview {
            self.toggle_preview();
        }
        self.cancel_placing();
        self.selection.clear();
        self.selected = None;
        self.ordering = Some(Ordering::default());
        self.status = String::from(
            "tab order — click the stops in the order you want them, within one parent",
        );
        self.reselected();
        self.refresh_labels();
    }

    /// Whether the tab-order overlay is up.
    pub const fn ordering(&self) -> bool {
        self.ordering.is_some()
    }

    /// Every place Tab can land, in the order it lands there, as file paths.
    ///
    /// Asked of the tree rather than worked out here, so the numbers on the
    /// canvas are the order the form will really have — including the parts
    /// this crate has no opinion about, like a list with no enabled rows not
    /// being a stop at all.
    pub fn tab_stops(&mut self) -> Vec<Vec<usize>> {
        let stops = self.ui.tab_stops();
        // Filtered to what the *form* built. The designer's own panes are in
        // the same tree and are emphatically not the form's tab order, and
        // `placed` is exactly the form's nodes — so the filter is the check.
        stops
            .into_iter()
            .filter_map(|id| {
                self.placed
                    .iter()
                    .find(|node| node.id == id)
                    .map(|node| node.path.clone())
            })
            .collect()
    }

    /// A click on the canvas while the tab order is showing.
    ///
    /// Each click makes that node the next stop after the one before it. The
    /// first click of a run is where the run starts; a click on a node with a
    /// different parent starts a new run there, because the file cannot say
    /// anything else. See [`Ordering`].
    fn pick_tab_stop(&mut self, at: Point) {
        let Some(path) = topmost(
            &self.placed,
            |p| {
                self.ui
                    .bounds(p.id)
                    .map(|r| (r, self.ui.visible(p.id), self.ui.z(p.id)))
            },
            at,
        )
        .map(|p| p.path.clone()) else {
            return;
        };
        if !self.tab_stops().contains(&path) {
            self.status = format!("{} is not a tab stop", self.describe(&path));
            self.refresh_labels();
            return;
        }
        let Some(ordering) = self.ordering.as_mut() else {
            return;
        };

        let previous = ordering.picked.last().cloned();
        let same_parent = previous.as_ref().is_some_and(|last| {
            last.split_last().map(|it| it.1) == path.split_last().map(|it| it.1)
        });
        if !same_parent {
            ordering.picked.clear();
        }
        if ordering.picked.contains(&path) {
            self.status = String::from("already placed in this run");
            self.refresh_labels();
            return;
        }

        match previous.filter(|_| same_parent) {
            // The first of a run: nothing to move it after yet.
            None => {
                if let Some(ordering) = self.ordering.as_mut() {
                    ordering.picked.push(path.clone());
                }
                self.status = format!("{} is first — click the next one", self.describe(&path));
            }
            Some(after) => {
                let landed = self.move_after(&path, &after);
                if let Some(ordering) = self.ordering.as_mut() {
                    ordering.picked.push(landed);
                }
                self.status = format!("{} placed", self.describe(&path));
                self.reload_from_document();
            }
        }
        self.refresh_labels();
        self.refresh_overlay();
    }

    /// Moves a node so it is the next sibling after `after`, returning where it
    /// landed.
    ///
    /// One `Edit::Move` among siblings, which is the same edit *bring to front*
    /// writes — so this is a reordering of the file and nothing on the canvas
    /// moves, every rectangle being its own.
    fn move_after(&mut self, path: &[usize], after: &[usize]) -> Vec<usize> {
        let (Some((&from, parent)), Some((&anchor, _))) = (path.split_last(), after.split_last())
        else {
            return path.to_vec();
        };
        // Landing *after* the anchor: one past it, and one less again when the
        // node being moved was already before it and vacates a slot on the way.
        let target = if from < anchor { anchor } else { anchor + 1 };
        if from == target {
            return path.to_vec();
        }
        let mut landed = parent.to_vec();
        landed.push(target);
        self.history.separate();
        self.edit(Edit::Move {
            from: path.to_vec(),
            to: parent.to_vec(),
            index: target,
        });
        landed
    }

    // ------------------------------------------------- the file underneath

    /// Reads the file again if something else has written it.
    ///
    /// Called once a frame; see [`watch::EVERY`] for how often a frame is when
    /// nothing else is going on, and [`crate::watch`] for why the file is asked
    /// rather than subscribed to, and read rather than stat-ed. With
    /// nothing unsaved the reload is silent, because there is no question worth
    /// asking: the designer is showing the file, the file changed, so the
    /// designer shows the new one. With unsaved work there is exactly one
    /// question, and it gets asked rather than answered by whoever wrote last.
    /// Whether rebuilding the tree now would pull it out from under something.
    ///
    /// A drag holds `NodeId`s from the tree it started in, and a rebuild hands
    /// out new ones. A caret is the same argument about text: the inspector is
    /// replaced wholesale, and what is half typed into it has not reached the
    /// document to be kept. Both callers answer the same way -- wait, and ask
    /// again next frame.
    fn mid_gesture(&self) -> bool {
        self.clash.is_some()
            || self.making.is_some()
            || self.drag.is_some()
            || self.band.is_some()
            || self.outline_drag.is_some()
            || self.choosing.is_some()
            || self.placing != Placing::Idle
            || self.typing()
    }

    pub fn check_file(&mut self) {
        // The file will still have changed when the pointer comes up.
        if self.mid_gesture() {
            return;
        }
        let Some(text) = self.document.changed_on_disk() else {
            return;
        };
        // Under a deadline: this is a file two editors have open, and the other
        // one is a text editor that can write anything at all into it. See
        // `Form::parse_within`.
        let fresh = match Form::parse_within(&text, denise_forms::PATIENCE) {
            Ok(form) => form,
            Err(error) => {
                // A file halfway through being written is not a conflict, and a
                // file somebody has broken is theirs to fix. Either way there is
                // nothing to reload, and saying so is the whole response. The
                // next write is noticed like any other.
                self.status = format!("the file on disk does not parse: {error}");
                self.refresh_labels();
                return;
            }
        };
        if !self.history.is_dirty() {
            let changed = differences(self.document.form(), &fresh).len();
            self.reload(text);
            self.status = match changed {
                0 => String::from("the file changed on disk — reloaded"),
                1 => String::from("the file changed on disk — reloaded, one node differs"),
                many => format!("the file changed on disk — reloaded, {many} nodes differ"),
            };
            self.refresh_labels();
            return;
        }
        self.begin_clash(text, &fresh);
    }

    /// Takes the file's own version, keeping hold of what the file can name.
    ///
    /// The selection, the folded subtrees and the nodes hidden in the designer
    /// are all remembered **by name** across the reload, because a name is the
    /// one piece of identity a form node carries and the other editor may well
    /// have moved things about. A node with no name is remembered by where it
    /// sat, which is all there is to go on.
    fn reload(&mut self, text: String) {
        let selection = self.keepsakes(&self.selection.clone());
        let folded = self.keepsakes(&self.folded.clone());
        let hidden = self.keepsakes(&self.hidden.clone());
        // Which tab is being looked at is remembered the same way, and has to
        // be: an edit that inserts or moves a node above a `tabs` shifts its
        // path, and a stale one would show the wrong strip's page -- or none.
        let looking = self.looking_at.clone();
        let looking_at = self.keepsakes(
            &looking
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>(),
        );

        if let Err(error) = self.document.adopt(text) {
            self.status = error;
            self.refresh_labels();
            return;
        }
        self.history = History::new();
        self.warned = false;
        self.stale = false;
        self.selection.clear();
        self.selected = None;
        self.folded.clear();
        self.hidden.clear();
        self.show_form();

        self.selection = self.found_again(&selection);
        self.selected = self.selection.last().and_then(|path| self.node_id(path));
        self.folded = self.found_again(&folded);
        self.hidden = self.found_again(&hidden);
        // Pair by pair, not zipped: `found_again` drops what it cannot find,
        // so zipping two lists of different lengths would hand a strip's page
        // number to a different strip.
        self.looking_at = looking
            .into_iter()
            .zip(looking_at)
            .filter_map(|((_, ordinal), keepsake)| {
                self.found_again(std::slice::from_ref(&keepsake))
                    .pop()
                    .map(|path| (path, ordinal))
            })
            .collect();
        self.apply_looking_at();
        self.apply_hidden();
        self.reselected();
        self.refresh_labels();
    }

    /// How to find these nodes again in a tree that has been read afresh.
    fn keepsakes(&self, paths: &[Vec<usize>]) -> Vec<Keepsake> {
        paths
            .iter()
            .map(|path| Keepsake {
                name: self
                    .placed
                    .iter()
                    .find(|node| node.path == *path)
                    .and_then(|node| node.name.clone()),
                path: path.clone(),
            })
            .collect()
    }

    /// Where those nodes are now, dropping the ones that have gone.
    fn found_again(&self, kept: &[Keepsake]) -> Vec<Vec<usize>> {
        kept.iter()
            .filter_map(|keepsake| match &keepsake.name {
                Some(name) => self
                    .placed
                    .iter()
                    .find(|node| node.name.as_deref() == Some(name.as_str()))
                    .map(|node| node.path.clone()),
                None => self
                    .placed
                    .iter()
                    .any(|node| node.path == keepsake.path)
                    .then(|| keepsake.path.clone()),
            })
            .collect()
    }

    /// Puts up the one question a file changing underneath can raise.
    fn begin_clash(&mut self, text: String, fresh: &Form) {
        let found = differences(self.document.form(), fresh);
        const WIDE: i32 = 520;
        const ROW: i32 = 18;
        const HEAD: i32 = 104;
        const FOOT: i32 = 58;
        let listed = found.len().min(NAMED) as i32 + i32::from(found.len() > NAMED);
        let tall = HEAD + listed * ROW + FOOT;

        let scene = self.ui.push_scene(160);
        // As in `begin_new`: `tall` stays logical, because the card's children
        // are placed against it and scale with them; the sheet is placed against
        // the window, which is physical already.
        let scale = self.scale;
        let s = |rect: Rect| scale.r(rect);
        let px = |text: Text| scale.text(text);
        let size = self.settings_size();
        let (wide_px, tall_px) = (scale.n(WIDE), scale.n(tall));
        let sheet = Rect::new(
            (size.width as i32 - wide_px) / 2,
            (size.height as i32 - tall_px) / 3,
            wide_px,
            tall_px,
        );
        let card = self
            .ui
            .add(
                scene,
                Panel {
                    fill: Some(Role::Base100),
                    border: Some(Role::Warning),
                    border_width: 1,
                    radius: Radius::Box,
                    backdrop: false,
                },
                sheet,
            )
            .expect("a scene root takes children");

        let label = |ui: &mut Ui<Message>, text: &str, rect: Rect, role, size| {
            ui.add(
                card,
                Label::new(text).with_size(px(size)).with_role(role),
                rect,
            );
        };
        label(
            &mut self.ui,
            "The file changed on disk",
            s(Rect::new(GAP * 2, GAP * 2, WIDE - GAP * 4, 22)),
            Role::Warning,
            Text::Title,
        );
        let name = self.document.label().replace(" •", "");
        label(
            &mut self.ui,
            &format!("{name} was written by something else, and this form has unsaved changes."),
            s(Rect::new(GAP * 2, 46, WIDE - GAP * 4, 16)),
            Role::BaseContent,
            Text::Caption,
        );
        label(
            &mut self.ui,
            &match found.len() {
                0 => String::from("Nothing in it reads differently, but the bytes moved."),
                1 => String::from("One node reads differently:"),
                many => format!("{many} nodes read differently:"),
            },
            s(Rect::new(GAP * 2, 70, WIDE - GAP * 4, 16)),
            Role::Base300,
            Text::Caption,
        );

        let mut y = HEAD - 12;
        for difference in found.iter().take(NAMED) {
            label(
                &mut self.ui,
                &difference.line(),
                s(Rect::new(GAP * 3, y, WIDE - GAP * 5, ROW)),
                Role::BaseContent,
                Text::Caption,
            );
            y += ROW;
        }
        if found.len() > NAMED {
            label(
                &mut self.ui,
                &format!("…and {} more", found.len() - NAMED),
                s(Rect::new(GAP * 3, y, WIDE - GAP * 5, ROW)),
                Role::Base300,
                Text::Caption,
            );
        }

        // *Keep mine* is the primary because it is the one that loses nothing
        // now: the file on disk stays where it is until somebody saves over it,
        // and the work in the designer is the only copy of itself.
        self.ui.add(
            card,
            Button::new("Reload", Message::Reload)
                .with_role(Role::Neutral)
                .with_size(px(Text::Body)),
            s(Rect::new(WIDE - 216, tall - 44, 96, 32)),
        );
        self.ui.add(
            card,
            Button::new("Keep mine", Message::KeepMine)
                .with_role(Role::Primary)
                .with_size(px(Text::Body)),
            s(Rect::new(WIDE - 112, tall - 44, 96, 32)),
        );

        self.clash = Some(Clash { text });
        self.status = String::from("the file changed on disk — reload it, or keep what is here?");
        self.refresh_labels();
    }

    /// *Reload*: the file wins, and the unsaved edits go.
    pub fn take_theirs(&mut self) {
        let Some(clash) = self.clash.take() else {
            return;
        };
        self.ui.pop_scene();
        self.reload(clash.text);
        self.status = String::from("reloaded from disk");
        self.refresh_labels();
    }

    /// *Keep mine*: the designer wins, and the next save writes over the file.
    ///
    /// Nothing has to be marked as answered. [`crate::watch::Watch::changed`]
    /// took note of that version of the file when it reported it, so the same
    /// change is not raised again on the next frame; the next one will be.
    pub fn keep_mine(&mut self) {
        if self.clash.take().is_none() {
            return;
        }
        self.ui.pop_scene();
        self.status = String::from("kept what is here — saving will overwrite the file on disk");
        self.refresh_labels();
    }

    /// Puts the file-changed sheet up against another version of the file.
    ///
    /// For `--snapshot`, which cannot arrange the three things this sheet needs
    /// — a file open, unsaved work in hand, and another editor having saved one
    /// of them. Everything it draws is real: the list is [`differences`] between
    /// the form in hand and the text given here.
    pub fn clash_over(&mut self, theirs: &str) -> bool {
        let Ok(fresh) = Form::parse(theirs) else {
            return false;
        };
        self.begin_clash(theirs.to_string(), &fresh);
        true
    }

    /// Whether the file-changed sheet is up.
    pub const fn clashing(&self) -> bool {
        self.clash.is_some()
    }

    /// How long the event loop may sleep.
    ///
    /// A designer is idle almost all the time — but a designer with a file open
    /// is idle *and watching*, which is the whole of #100. Whichever is sooner.
    pub fn next_frame_in(&self) -> Option<Duration> {
        let animating = self.ui.next_wake_ms().map(|_| Duration::from_millis(16));
        let watching = self.document.path().map(|_| watch::EVERY);
        match (animating, watching) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (only, None) | (None, only) => only,
        }
    }

    // ------------------------------------------------- the clipboard

    /// Puts the selection on the clipboard, as `.dform` source.
    pub fn copy(&mut self) {
        let Some(text) = self.selection_text() else {
            self.status = String::from("nothing selected to copy");
            self.refresh_labels();
            return;
        };
        let lines = self.selection.len();
        self.clipboard.put(&text);
        self.status = format!("copied {lines} node(s) as form source");
        self.refresh_labels();
    }

    /// Copies the selection and takes it out of the file.
    ///
    /// One step, because the delete is one step and the copy writes nothing.
    pub fn cut(&mut self) {
        if self.selection.is_empty() {
            self.status = String::from("nothing selected to cut");
            self.refresh_labels();
            return;
        }
        let taken = self.selection.len();
        self.copy();
        self.delete_selection();
        self.status = format!("cut {taken} node(s)");
        self.refresh_labels();
    }

    /// Puts whatever is on the clipboard into the form.
    ///
    /// Under the selected container, or beside the selected node, or on the
    /// form — and offset, so that a copy of something is not hidden exactly
    /// behind it.
    pub fn paste(&mut self) {
        let Some(text) = self.clipboard.take() else {
            self.status = String::from("there is nothing on the clipboard");
            self.refresh_labels();
            return;
        };
        let parent = match self.selection.last() {
            Some(path) if self.is_container(path) => path.clone(),
            Some(path) => path
                .split_last()
                .map_or_else(Vec::new, |(_, up)| up.to_vec()),
            None => Vec::new(),
        };
        self.put_fragment(&text, parent, "pasted");
    }

    /// Another one of whatever is selected, beside it.
    ///
    /// Beside and not inside, even for a panel: duplicating a container means
    /// wanting a second one, not wanting one nested in the first.
    pub fn duplicate(&mut self) {
        let Some(text) = self.selection_text() else {
            self.status = String::from("nothing selected to duplicate");
            self.refresh_labels();
            return;
        };
        let parent = self
            .selection
            .last()
            .and_then(|path| path.split_last())
            .map_or_else(Vec::new, |(_, up)| up.to_vec());
        self.put_fragment(&text, parent, "duplicated");
    }

    /// The selection as `.dform` source, in file order.
    fn selection_text(&self) -> Option<String> {
        if self.selection.is_empty() {
            return None;
        }
        let mut paths = self.selection.clone();
        paths.sort();
        let mut out = String::new();
        for path in &paths {
            out.push_str(&self.document.form().node_text(path)?);
        }
        (!out.trim().is_empty()).then_some(out)
    }

    /// Whether a fragment would build, and the fragment as a form if it would.
    ///
    /// The schema lives with the builder — which widgets exist, which properties
    /// each has — so the only honest way to ask "is this form source?" is to
    /// build it. Into a tree nobody will ever draw, thrown away with the answer.
    /// What comes back is the fragment as a [`Form`], which is where the
    /// rectangles it wants are read from.
    fn check_fragment(&self, text: &str) -> Result<Form, denise_forms::Error> {
        let source = format!(
            "form \"\" version={} width=1 height=1 {{\n{text}\n}}\n",
            denise_forms::VERSION
        );
        // The wrapper puts one line above the fragment, so every complaint would
        // otherwise point one line too far down.
        let lower = |mut error: denise_forms::Error| {
            error.at.line = error.at.line.saturating_sub(1).max(1);
            error
        };
        // Under a deadline, because this is where the clipboard comes in and
        // the clipboard holds whatever the last program to touch it put there.
        let form = Form::parse_within(&source, denise_forms::PATIENCE).map_err(lower)?;

        let mut scratch: Ui<Message> = Ui::new(Size::new(1, 1), theme::DARK);
        let root = scratch.root();
        let mut wiring = Design {
            base: self.document.base(),
            missing: Vec::new(),
            names: Vec::new(),
        };
        form.build(&mut scratch, root, &mut wiring).map_err(lower)?;
        Ok(form)
    }

    /// Puts a fragment of form source into the document, as one edit.
    fn put_fragment(&mut self, text: &str, parent: Vec<usize>, what: &str) {
        let checked = match self.check_fragment(text) {
            Ok(form) => form,
            Err(error) => {
                self.status = format!("that is not form source: {error}");
                self.refresh_labels();
                return;
            }
        };

        // Every name the document already uses, so the arrivals get their own.
        let mut taken: Vec<String> = self
            .placed
            .iter()
            .filter_map(|node| node.name.clone())
            .collect();
        let nodes = match denise_forms::fragment(text, &mut taken) {
            Ok(nodes) => nodes,
            Err(error) => {
                self.status = format!("that is not form source: {error}");
                self.refresh_labels();
                return;
            }
        };
        if nodes.is_empty() {
            self.status = String::from("there was nothing in that to paste");
            self.refresh_labels();
            return;
        }

        // Far enough to be seen behind what it came from, and on the grid so it
        // still lines up with everything else.
        let step = self.grid.max(1) * 2;
        let held = self.child_count(&parent);
        let mut edits = Vec::new();
        let mut landed = Vec::new();
        for (nth, node) in nodes.iter().enumerate() {
            let index = held + nth;
            let mut path = parent.clone();
            path.push(index);
            edits.push(Edit::Insert {
                parent: parent.clone(),
                index,
                text: node.clone(),
            });
            for (name, axis) in [("x", "x"), ("y", "y")] {
                if let Some(was) = checked
                    .property(&[nth], axis)
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    edits.push(Edit::number(&path, name, Some(was + i64::from(step))));
                }
            }
            landed.push(path);
        }

        self.history.separate();
        self.edit(Edit::Many(edits));
        self.selection = landed;
        self.reload_from_document();
        self.status = format!("{what} {} node(s)", nodes.len());
        self.refresh_labels();
    }

    // ---------------------------------------------------------- arranging

    /// Everything selected, in file order, when they all share one parent.
    ///
    /// `None` when they do not, which is when none of the arrange commands mean
    /// anything: see [`crate::arrange`] for why that is a property of having no
    /// layout engine rather than a shortcut.
    fn siblings_selected(&self) -> Option<Vec<Vec<usize>>> {
        let (_, parent) = self.selection.first()?.split_last()?;
        let parent = parent.to_vec();
        let same = self
            .selection
            .iter()
            .all(|path| path.split_last().is_some_and(|(_, above)| above == parent));
        if !same {
            return None;
        }
        let mut paths = self.selection.clone();
        paths.sort();
        Some(paths)
    }

    /// Whether a command can be given now.
    fn can_arrange(&self, command: Command) -> bool {
        if self.preview {
            return false;
        }
        match command.needs() {
            Needs::Several => self.siblings_selected().is_some_and(|it| it.len() >= 2),
            Needs::Spread => self.siblings_selected().is_some_and(|it| it.len() >= 3),
            Needs::Holder => match self.selection.as_slice() {
                [one] => self.is_container(one) && self.child_count(one) > 0,
                _ => false,
            },
        }
    }

    /// Gives one of the arrange commands.
    pub fn arrange(&mut self, command: Command) {
        if !self.can_arrange(command) {
            self.status = String::from(command.needs().why());
            self.refresh_labels();
            return;
        }
        // Group and ungroup move nodes in the tree; everything else changes
        // four numbers and is the same shape of edit as a drag.
        if command.is_structural() {
            if command == Command::Group {
                self.group();
            } else {
                self.ungroup();
            }
            return;
        }

        let (Some(paths), Some(primary)) =
            (self.siblings_selected(), self.selection.last().cloned())
        else {
            return;
        };
        // The anchor is the one wearing the handles, wherever it sits in file
        // order — see [`crate::arrange`] for why it is that one and not the
        // first.
        let anchor = paths.iter().position(|path| *path == primary).unwrap_or(0);
        let rects: Vec<Rect> = paths
            .iter()
            .filter_map(|path| self.node_id(path).and_then(|id| self.form_layout(id)))
            .collect();
        if rects.len() != paths.len() {
            return;
        }

        let placed = arrange::arrange(command, &rects, anchor);
        let moved: Vec<(Vec<usize>, Rect, Rect)> = paths
            .iter()
            .zip(&rects)
            .zip(&placed)
            .map(|((path, was), now)| (path.clone(), *now, *was))
            .collect();
        if moved.iter().all(|(_, now, was)| now == was) {
            self.status = format!("already {}", command.done());
            self.refresh_labels();
            return;
        }

        self.history.separate();
        self.write_rects(&moved);
        self.reload_from_document();
        self.status = format!("{} — {} nodes", command.done(), paths.len());
        self.refresh_labels();
    }

    /// Edits that move a set of siblings into `to`, appended in file order,
    /// each landing with the rectangle given for it.
    ///
    /// Also hands back where `to` itself ended up: taking a node out from in
    /// front of it moves it, and every edit after this one has to name it by
    /// where it is *then*. `moves` must be in file order, which is what makes
    /// the shifting arithmetic here a matter of counting rather than guessing.
    fn move_into(&self, moves: &[(Vec<usize>, Rect)], to: &[usize]) -> (Vec<Edit>, Vec<usize>) {
        let mut edits = Vec::new();
        let mut landing = to.to_vec();
        let held = self.child_count(to);
        let mut sources: Vec<Vec<usize>> = moves.iter().map(|(path, _)| path.clone()).collect();

        for (step, (_, rect)) in moves.iter().enumerate() {
            let from = sources[step].clone();
            let index = held + step;
            edits.push(Edit::Move {
                from: from.clone(),
                to: landing.clone(),
                index,
            });
            landing = denise_forms::after_removing(&landing, &from).unwrap_or(landing);
            let mut landed = landing.clone();
            landed.push(index);
            for (name, value) in [("x", rect.x), ("y", rect.y)] {
                edits.push(Edit::number(&landed, name, Some(i64::from(value))));
            }
            // Everything still to come was shifted by that removal too.
            for later in sources.iter_mut().skip(step + 1) {
                *later =
                    denise_forms::after_removing(later, &from).unwrap_or_else(|| later.clone());
            }
        }
        (edits, landing)
    }

    /// Puts the selection inside a new panel that takes their bounding box.
    fn group(&mut self) {
        let Some(paths) = self.siblings_selected() else {
            return;
        };
        let Some((_, parent)) = paths[0].split_last() else {
            return;
        };
        let parent = parent.to_vec();
        let rects: Vec<Rect> = paths
            .iter()
            .filter_map(|path| self.node_id(path).and_then(|id| self.form_layout(id)))
            .collect();
        if rects.len() != paths.len() {
            return;
        }

        let all = arrange::bounds(&rects);
        // Last of its siblings, so putting it there shifts nothing that is
        // about to be named. It ends up further forward than the nodes it will
        // hold, which is where a container of them belongs.
        let index = self.child_count(&parent);
        let mut panel = parent.clone();
        panel.push(index);

        let moves: Vec<(Vec<usize>, Rect)> = paths
            .iter()
            .zip(&rects)
            .map(|(path, rect)| {
                (
                    path.clone(),
                    Rect::new(rect.x - all.x, rect.y - all.y, rect.width, rect.height),
                )
            })
            .collect();

        let mut edits = vec![Edit::Insert {
            parent,
            index,
            text: denise_forms::seed("panel", all),
        }];
        let (moving, landed) = self.move_into(&moves, &panel);
        edits.extend(moving);

        self.history.separate();
        self.edit(Edit::Many(edits));
        self.selection = vec![landed];
        self.reload_from_document();
        self.status = format!("{} {} nodes", Command::Group.done(), paths.len());
        self.refresh_labels();
    }

    /// Takes the selected panel's children out of it, and the panel away.
    fn ungroup(&mut self) {
        let Some(panel) = self.selection.last().cloned() else {
            return;
        };
        let Some((&index, parent)) = panel.split_last() else {
            return;
        };
        let parent = parent.to_vec();
        let Some(origin) = self.node_id(&panel).and_then(|id| self.form_layout(id)) else {
            return;
        };
        let held = self.child_count(&panel);
        let rects: Vec<Rect> = (0..held)
            .filter_map(|child| {
                let mut path = panel.clone();
                path.push(child);
                self.node_id(&path).and_then(|id| self.form_layout(id))
            })
            .collect();
        if rects.len() != held {
            return;
        }

        let mut edits = Vec::new();
        for (step, rect) in rects.iter().enumerate() {
            // Always the first one still inside: the ones before it have left.
            let mut from = panel.clone();
            from.push(0);
            let at = index + 1 + step;
            edits.push(Edit::Move {
                from,
                to: parent.clone(),
                index: at,
            });
            let mut landed = parent.clone();
            landed.push(at);
            for (name, value) in [("x", rect.x + origin.x), ("y", rect.y + origin.y)] {
                edits.push(Edit::number(&landed, name, Some(i64::from(value))));
            }
        }
        // The panel is empty now, and an empty panel is all that is left of the
        // grouping. Taking it out shifts everything that landed after it back
        // over the hole it leaves.
        let mut gone = parent.clone();
        gone.push(index);
        edits.push(Edit::remove(&gone));

        self.history.separate();
        self.edit(Edit::Many(edits));
        self.selection = (0..held)
            .map(|step| {
                let mut path = parent.clone();
                path.push(index + step);
                path
            })
            .collect();
        self.reload_from_document();
        self.status = format!("{} {} nodes", Command::Ungroup.done(), held);
        self.refresh_labels();
    }

    /// Puts the selected node in front of its siblings, or behind them.
    ///
    /// **By reordering the file**, not by writing `z=`. Siblings are drawn in
    /// file order, so the order in the file is the order on the screen, and a
    /// person reading the file sees the stacking without having to hold a second
    /// rule in their head. A `z=` written here would be that second rule.
    ///
    /// Siblings only: file order decides between two nodes of the same parent
    /// and nothing else, so there is no such thing as bringing a node in front
    /// of its uncle.
    pub fn restack(&mut self, front: bool) {
        let Some(path) = self.selection.last().cloned() else {
            return;
        };
        let Some((&index, parent)) = path.split_last() else {
            return;
        };
        let parent = parent.to_vec();
        let target = if front {
            self.child_count(&parent).saturating_sub(1)
        } else {
            0
        };
        let side = if front { "front" } else { "back" };
        if index == target {
            self.status = format!("already at the {side}");
            self.refresh_labels();
            return;
        }

        let mut landed = parent.clone();
        landed.push(target);
        self.history.separate();
        self.edit(Edit::Move {
            from: path,
            to: parent.clone(),
            index: target,
        });
        self.selection = vec![landed];
        self.reload_from_document();

        // `z` overrules file order, so on a form that sets it this has moved the
        // node in the file and nowhere a person can see. Saying so beats leaving
        // somebody to wonder why nothing happened.
        self.status = if self.stacked_by_z(&parent) {
            format!(
                "moved to the {side} of the file — but `z` is set here, and `z` is what decides"
            )
        } else {
            format!("moved to the {side}")
        };
        self.refresh_labels();
    }

    /// Whether anything among a parent's children sets `z`, which overrules the
    /// file order [`Self::restack`] moves.
    fn stacked_by_z(&self, parent: &[usize]) -> bool {
        self.placed
            .iter()
            .filter(|node| node.path.len() == parent.len() + 1 && node.path.starts_with(parent))
            .any(|node| {
                self.document
                    .form()
                    .property(&node.path, "z")
                    .is_some_and(|z| z != "0")
            })
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
        self.reselected();
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
                Point::new(
                    view.x + view.width / 2,
                    view.y + self.scale.n(PALETTE_ROW) / 2,
                )
            });
        self.placing = Placing::Pressed { kind, from };
        // `over` is where in the *form* to hold it, so it travels with the zoom.
        let over = Point::new(self.zoom.on_screen_n(over.x), self.zoom.on_screen_n(over.y));
        self.carry_to(Point::new(stage.x + over.x, stage.y + over.y));
        matches!(self.placing, Placing::Carrying { .. })
    }

    /// Poses a rubber band across a rectangle in the form's own coordinates.
    ///
    /// For `--snapshot`, which has no pointer to draw one with. The band is left
    /// in flight, because in flight is when there is anything to see. It is
    /// scoped to whatever container its first corner is in, rather than needing
    /// that container to have been selected first — a snapshot has nobody to
    /// select it.
    pub fn band_over(&mut self, rect: Rect) {
        let stage = self.ui.bounds(self.chrome.stage).unwrap_or(Rect::ZERO);
        let rect = self.on_screen(rect);
        let from = Point::new(stage.x + rect.x, stage.y + rect.y);
        let to = Point::new(from.x + rect.width, from.y + rect.height);
        let scope = self.container_at(from, &[]).unwrap_or_default();
        self.begin_band(scope, from, false);
        self.band_to(to);
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
        self.reselected();
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
        // `dx` and `dy` are given in the form's own pixels — the units the
        // caller is thinking in — so the pointer has to travel further than that
        // when the canvas is magnified.
        let (dx, dy) = (self.zoom.on_screen_n(dx), self.zoom.on_screen_n(dy));
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
            .map(|rect| self.in_form(rect))
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
        // A grab handle is aimed at by a hand, so it is the display's size and
        // not the form's: the same seven logical pixels at 25% as at 400%. It
        // is the one thing drawn over the canvas that does not scale with what
        // is under it, and `Grip::at` is given the same reach so that what can
        // be pressed is what can be seen.
        let reach = self.scale.n(canvas::HANDLE);
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

        // The numbers, when the tab order is showing. They replace the
        // selection rather than sitting beside it: this mode is about the
        // sequence, and eight resize handles over it would be noise.
        //
        // Before the closure below, which borrows `self` for the rest of the
        // function and so cannot be mixed with asking the tree anything.
        if self.ordering.is_some() {
            let stops = self.tab_stops();
            let picked: Vec<Vec<usize>> = self
                .ordering
                .as_ref()
                .map(|it| it.picked.clone())
                .unwrap_or_default();
            let mut badges: Vec<NodeId> = Vec::new();
            for (nth, path) in stops.iter().enumerate() {
                let Some(bounds) = self.path_bounds(path) else {
                    continue;
                };
                // Done in this run reads as done: a filled badge rather than an
                // outlined one, so the eye can see how far it has got.
                let settled = picked.contains(path);
                let badge = to_canvas(Rect::new(bounds.x, bounds.y, 22, 16));
                if let Some(id) = self.ui.add(
                    self.chrome.canvas,
                    Panel {
                        fill: Some(if settled {
                            Role::Primary
                        } else {
                            Role::Base300
                        }),
                        border: Some(Role::Primary),
                        border_width: 1,
                        radius: Radius::Field,
                        backdrop: false,
                    },
                    badge,
                ) {
                    self.ui.set_z(id, 210);
                    badges.push(id);
                }
                if let Some(id) = self.ui.add(
                    self.chrome.canvas,
                    Label::new(format!("{}", nth + 1))
                        .with_size(self.scale.text(Text::Caption))
                        .with_align(denise_ui::Align::Center, denise_ui::Align::Center)
                        .with_role(if settled {
                            Role::PrimaryContent
                        } else {
                            Role::BaseContent
                        }),
                    badge,
                ) {
                    self.ui.set_z(id, 215);
                    badges.push(id);
                }
            }
            self.overlay = badges;
            return;
        }

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
            // `guide.at` is an offset in the parent's own form coordinates,
            // because that is the space `place` lined the edges up in; `parent`
            // came out of the tree and is on screen. The line itself is one
            // screen pixel at any zoom — it marks an edge rather than measuring
            // one.
            let zoom = self.zoom;
            for guide in guides {
                let at = zoom.on_screen_n(guide.at);
                let line = if guide.vertical {
                    Rect::new(parent.x + at, parent.y, 1, parent.height)
                } else {
                    Rect::new(parent.x, parent.y + at, parent.width, 1)
                };
                add(&mut self.ui, line, Panel::filled(Role::Accent), 190);
            }
        }

        // Where a drag would drop, drawn round the container that would take it.
        if let Some(target) = self.dropping.clone()
            && let Some(bounds) = if target.is_empty() {
                self.ui.bounds(self.chrome.stage)
            } else {
                self.path_bounds(&target)
            }
        {
            add(
                &mut self.ui,
                bounds,
                Panel {
                    fill: None,
                    border: Some(Role::Accent),
                    border_width: 2,
                    radius: Radius::Box,
                    backdrop: false,
                },
                195,
            );
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
                        Grip::handle_rect(bounds, corner, reach),
                        Panel::filled(Role::Primary),
                        210,
                    );
                }
            }
        }
        // The band on top of everything, since it is the thing being drawn now.
        // Four hairlines rather than a bordered panel: a band has square
        // corners, and every rounding this toolkit offers is a rounding.
        if let Some(rect) = self.band.as_ref().filter(|band| band.moved).map(Band::rect) {
            for edge in [
                Rect::new(rect.x, rect.y, rect.width, 1),
                Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
                Rect::new(rect.x, rect.y, 1, rect.height),
                Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
            ] {
                add(&mut self.ui, edge, Panel::filled(Role::Accent), 230);
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
            let tall = self.scale.n(Text::Caption.line());
            let tag = Rect::new(bounds.x, bounds.y - tall - self.scale.n(1), 200, tall);
            if let Some(id) = self.ui.add(
                self.chrome.canvas,
                Label::new(text)
                    .with_size(self.scale.text(Text::Caption))
                    .with_role(Role::Primary),
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
            Message::New => self.begin_new(),
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
            Message::Renamed => self.finish_rename(),
            Message::Arrange(command) => self.arrange(command),
            Message::NewKind(index) => self.choose_kind(index),
            Message::NewSize(index) => self.choose_size(index),
            Message::Create => self.create(),
            Message::Never => {
                self.close_new();
                self.status = String::from("no new form, then");
                self.refresh_labels();
            }
            Message::Reload => self.take_theirs(),
            Message::KeepMine => self.keep_mine(),
            Message::Preview => self.toggle_preview(),
            Message::TabOrder => self.toggle_tab_order(),
            Message::Theme => self.cycle_theme(),
            Message::Zoom => self.cycle_zoom(),
            Message::PaletteMode => self.cycle_palette_mode(),
            Message::OpenCode(row) => self.open_code(row),
            Message::ItemAdd(row) => self.add_item(row),
            Message::ItemRemove(row, nth) => self.remove_item(row, nth),
            Message::ItemUp(row, nth) => self.move_item(row, nth, nth.wrapping_sub(1)),
            Message::ItemDown(row, nth) => self.move_item(row, nth, nth + 1),
            Message::Key(code) => {
                let events = self.keyboard.press_key(&mut self.ui, code);
                self.ui.handle(&events);
            }
            Message::Fired(index) => self.log_fired(index, None),
            Message::FiredBool(index, value) => self.log_fired(index, Some(value.to_string())),
            Message::FiredIndex(index, value) => self.log_fired(index, Some(value.to_string())),
            Message::FiredNumber(index, value) => {
                self.log_fired(index, Some(crate::inspector::trim(value)));
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
        // Nothing selected is the form, whose properties are at the empty path.
        let edits: Vec<Edit> = if self.selection.is_empty() {
            vec![Edit::property(&[], name, None)]
        } else {
            self.selection
                .iter()
                .map(|path| Edit::property(path, name, None))
                .collect()
        };
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

    /// A press on an event's name in the inspector: the second of two inside
    /// [`code::DOUBLE_CLICK_MS`] opens the handler, as the button beside it
    /// would. A single press is left to the tree, so nothing else changes.
    fn press_event_name(&mut self, at: Point) -> bool {
        let row = self.inspector.as_ref().and_then(|pane| {
            pane.rows.iter().position(|row| {
                is_event(row.property)
                    && row
                        .name
                        .and_then(|name| self.ui.bounds(name))
                        .is_some_and(|bounds| bounds.contains(at))
            })
        });
        let Some(row) = row else {
            self.last_name_press = None;
            return false;
        };
        let now = self.now_ms;
        let paired = matches!(
            self.last_name_press,
            Some((seen, then)) if seen == row && now.saturating_sub(then) <= code::DOUBLE_CLICK_MS
        );
        if paired {
            self.last_name_press = None;
            self.open_code(row);
            true
        } else {
            self.last_name_press = Some((row, now));
            false
        }
    }

    /// Opens the handler behind an event row in the editor, writing one first
    /// when nothing in the code answers the name yet. See [`code`].
    ///
    /// Three things have to be known, and each missing one is a status line
    /// rather than a dialog: the event needs a name, the form needs a file for
    /// the sidecar to sit beside, and the code needs a path — which is asked
    /// for once and remembered in that sidecar.
    pub fn open_code(&mut self, row: usize) {
        let Some((property, editor)) = self.inspector.as_ref().and_then(|pane| {
            let row = pane.rows.get(row)?;
            let editor = match row.editor {
                Editor::Field(id) => Some(id),
                _ => None,
            };
            Some((row.property, editor))
        }) else {
            return;
        };
        let PropertyKind::Message(payload) = property.kind else {
            return;
        };
        // What the field holds *now*, not what it was given: the name being
        // opened is usually the one just typed.
        let event = editor
            .and_then(|id| self.ui.widget::<TextInput<Message>>(id))
            .map(|field| field.text().trim().to_string())
            .unwrap_or_default();
        if event.is_empty() {
            self.status = format!(
                "`{}` has no name yet — type one, and it can be opened",
                property.name
            );
            self.refresh_labels();
            return;
        }
        let Some(form) = self.document.path().map(Path::to_path_buf) else {
            self.status =
                String::from("save the form first — where its code lives is remembered beside it");
            self.refresh_labels();
            return;
        };
        let link = match code::read_link(&form) {
            Some(link) => link,
            None => {
                let Some(chosen) = pick_code(&form) else {
                    return;
                };
                if let Err(why) = code::write_link(&form, &chosen) {
                    self.status = why;
                    self.refresh_labels();
                    return;
                }
                code::Link {
                    code: chosen,
                    handlers: None,
                }
            }
        };
        let code_path = link.code;
        let handlers = link.handlers.as_deref();

        let form_name = form.file_name().map_or_else(
            || form.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let kind = self
            .selected
            .and_then(|id| self.ui.kind(id))
            .unwrap_or("form");
        let fired_by = format!("`{}` of a `{kind}`", property.name);

        let created = !code_path.exists();
        let mut source = if created {
            code::header(&form_name)
        } else {
            match std::fs::read_to_string(&code_path) {
                Ok(source) => source,
                Err(why) => {
                    self.status = format!("could not read {}: {why}", code_path.display());
                    self.refresh_labels();
                    return;
                }
            }
        };
        let ((line, column), ensured) = code::ensure(
            &mut source,
            &event,
            payload,
            &fired_by,
            &form_name,
            handlers,
        );
        if (created || ensured != code::Ensured::Found)
            && let Err(why) = std::fs::write(&code_path, &source)
        {
            self.status = format!("could not write {}: {why}", code_path.display());
            self.refresh_labels();
            return;
        }

        let function = code::snake(&event);
        let file = code::display_name(&code_path);
        let did = match (created, ensured) {
            (true, _) => format!("wrote {file} with `fn {function}` — "),
            (false, code::Ensured::AddedMethod) => {
                format!(
                    "added `fn {function}` to `impl {}` in {file} — ",
                    handlers.unwrap_or_default()
                )
            }
            (false, code::Ensured::AddedFunction) => match handlers {
                // Asked for a method and could not oblige: say why, or the
                // free function looks like a choice rather than a fallback.
                Some(handlers) => {
                    format!("no `impl {handlers}` in {file}, so `fn {function}` went at the end — ")
                }
                None => format!("added `fn {function}` to {file} — "),
            },
            (false, code::Ensured::Found) => String::new(),
        };
        self.status = match code::launch(&self.settings.editor, &code_path, line, column) {
            Ok(said) | Err(said) => format!("{did}{said}"),
        };
        self.refresh_labels();
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
/// The edits that turn one rectangle into another, and only for what changed.
///
/// One edit per number, so a run of nudges along one axis still coalesces into
/// a single step; a caller with several of these wraps them in one [`Edit::Many`].
fn rect_edits(path: &[usize], rect: Rect, was: Rect) -> Vec<Edit> {
    [
        ("x", rect.x, was.x),
        ("y", rect.y, was.y),
        ("w", rect.width, was.width),
        ("h", rect.height, was.height),
    ]
    .into_iter()
    .filter(|(_, new, old)| new != old)
    .map(|(name, new, _)| Edit::number(path, name, Some(i64::from(new))))
    .collect()
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

/// The form's title, which is the form node's **argument** rather than one of
/// its properties — so the schema has no descriptor for it, and the inspector
/// needs one to draw a row with.
static TITLE: Property = Property::new(
    "title",
    PropertyKind::Text,
    "What this form is called. Shown in a window's title bar.",
);

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

/// Asks, once, which file holds a form's code.
///
/// The platform's *save* dialog rather than its open one, because the file may
/// not exist yet and a save dialog is the one that lets a name be typed. An
/// existing file chosen here is appended to, never replaced, whatever the
/// dialog's own warning says.
fn pick_code(form: &Path) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Which file holds this form's code?")
        .add_filter("Rust source", &["rs"]);
    if let Some(directory) = form.parent() {
        dialog = dialog.set_directory(directory);
    }
    // Named for the form, the way Delphi's unit was, rather than `Untitled`:
    // the pairing the sidecar exists to record is at least suggested.
    if let Some(stem) = form.file_stem() {
        dialog = dialog.set_file_name(format!("{}.rs", stem.to_string_lossy()));
    }
    dialog.save_file()
}

fn pick_save() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Denise form", &["dform"])
        .set_title("Save the form as")
        .set_file_name("form.dform")
        .save_file()
}

/// A function pointer per message name, because one cannot carry an index.
///
/// `|value| Message::FiredBool(3, value)` captures nothing, so it coerces to a
/// `fn(bool) -> Message` — which is what a `Checkbox` holds. What it cannot do is
/// take the `3` from a variable, so there is one of these per name and the
/// engine is handed the one belonging to the name it just resolved.
macro_rules! by_name {
    ($($index:literal)*) => {
        const FLAGS: &[fn(bool) -> Message] =
            &[$(|value| Message::FiredBool($index, value)),*];
        const CHOICES: &[fn(usize) -> Message] =
            &[$(|value| Message::FiredIndex($index, value)),*];
        const NUMBERS: &[fn(f32) -> Message] =
            &[$(|value| Message::FiredNumber($index, value)),*];
    };
}

by_name!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63);

/// What the designer supplies a form it did not write.
struct Design {
    base: PathBuf,
    missing: Vec<String>,
    /// Every message name the form used, in the order they were first seen. The
    /// index into this is what a fired message carries.
    names: Vec<String>,
}

impl Wiring<Message> for Design {
    /// Every name resolves, and carries which name it was.
    ///
    /// A designer cannot know an application's message type, and a form that
    /// would not open because the designer had never heard of `on-press=greet`
    /// would be useless. So every name is accepted, and what it resolves to
    /// remembers the name — which is what preview mode's log shows, and the
    /// answer to "is `on-press=greet` wired up" without writing the application
    /// first.
    fn message(&mut self, name: &str, payload: Payload) -> Option<Handler<Message>> {
        let index = match self.names.iter().position(|held| held == name) {
            Some(index) => index,
            None => {
                self.names.push(name.to_string());
                self.names.len() - 1
            }
        };
        // Past the end of the table: the message still fires, and the log says
        // it cannot name it rather than naming the wrong one.
        if index >= NAMES {
            return Some(match payload {
                Payload::None => Handler::Plain(Message::Inert),
                Payload::Bool => Handler::Bool(|_| Message::Inert),
                Payload::Index => Handler::Index(|_| Message::Inert),
                Payload::Number => Handler::Number(|_| Message::Inert),
            });
        }
        Some(match payload {
            Payload::None => Handler::Plain(Message::Fired(index)),
            Payload::Bool => Handler::Bool(FLAGS[index]),
            Payload::Index => Handler::Index(CHOICES[index]),
            Payload::Number => Handler::Number(NUMBERS[index]),
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

    /// The whole chrome at 2x is the 1x layout doubled, node for node.
    ///
    /// This is the test the DPI fix is worth having. The designer took the scale
    /// factor from `run_with` and dropped it from its first commit, so on a
    /// Retina Mac every pane, row and label came out at half the size it was
    /// drawn at — correct on a Pi, which is why nothing caught it. Half the
    /// crate's rectangles now go through [`Scale`] on the way into the tree, and
    /// the one that does not is invisible at 1x, which is where the rest of
    /// these tests live. So it is checked at 2x, against arithmetic rather than
    /// against a blessed picture.
    #[test]
    fn the_chrome_at_twice_the_scale_is_the_same_layout_doubled() {
        let (one, two) = (at_scale(1.0), at_scale(2.0));
        let (mine, theirs) = (one.chrome.every(), two.chrome.every());
        assert_eq!(mine.len(), theirs.len());

        for ((name, here), (_, there)) in mine.iter().zip(&theirs) {
            let here = one.ui.bounds(*here).expect(name);
            let there = two.ui.bounds(*there).expect(name);
            assert_eq!(
                there,
                here.scaled(2.0),
                "{name} is not where twice the scale puts it"
            );
        }

        // And the text in them, which a rectangle cannot show. A widget defaults
        // its text to 16 px and knows nothing about the display, so one that is
        // never told a size stays 16 px while everything around it doubles —
        // which is exactly what the palette's list did.
        for ((name, here), (_, there)) in mine.iter().zip(&theirs) {
            let (Some(here), Some(there)) = (
                one.ui.get_property(*here, "size"),
                two.ui.get_property(*there, "size"),
            ) else {
                continue;
            };
            let sized = |value: Value| match value {
                Value::Int(size) => size,
                other => panic!("{name} reports a size of {other:?}"),
            };
            assert_eq!(
                sized(there),
                sized(here) * 2,
                "{name} draws its text at the same size on a display of twice the density"
            );
        }
    }

    /// And the two panes that are rebuilt rather than built once.
    ///
    /// The outline and the inspector are torn down and drawn again on every
    /// selection, from their own constants, so they can be wrong in a way the
    /// docked panes above cannot.
    #[test]
    fn the_panes_that_are_rebuilt_scale_with_the_rest() {
        let (mut one, mut two) = (at_scale(1.0), at_scale(2.0));
        // Something selected, so the inspector has rows rather than the form's.
        for designer in [&mut one, &mut two] {
            assert!(
                designer.select_named("busy"),
                "the reference form has a node called `busy`"
            );
        }

        let content = |designer: &Designer| {
            (
                designer.outline.as_ref().expect("an outline").content,
                designer.inspector.as_ref().expect("an inspector").content,
            )
        };
        let (outline_one, inspector_one) = content(&one);
        let (outline_two, inspector_two) = content(&two);

        for (name, here, there) in [
            ("the outline", outline_one, outline_two),
            ("the inspector", inspector_one, inspector_two),
        ] {
            let here = one.ui.bounds(here).expect(name);
            let there = two.ui.bounds(there).expect(name);
            assert_eq!(
                there,
                here.scaled(2.0),
                "{name} is not where twice the scale puts it"
            );
        }
    }

    /// The canvas follows the display, so 100% is actual size *on this screen*.
    ///
    /// #154 left it at one screen pixel per form pixel, which is what a kiosk
    /// panel does and which drew the form at half the size of the toolbar
    /// beside it on a Retina display — reported as a bug the first time anybody
    /// used it, and it is one: nobody eyeballing a canvas is counting device
    /// pixels, and `denise-forms render --scale` is where a pixel-exact check
    /// belongs. So the stage is the form's size times the display's scale, and
    /// the zoom control multiplies on top of that.
    #[test]
    fn the_form_on_the_canvas_is_actual_size_on_this_display() {
        let (one, two) = (at_scale(1.0), at_scale(2.0));
        assert_eq!((one.zoom.percent(), two.zoom.percent()), (100, 100));
        let size = one.document.form().size();

        for (name, designer, factor) in [("1x", &one, 1), ("2x", &two, 2)] {
            let stage = designer
                .ui
                .bounds(designer.chrome.stage)
                .expect("the stage is there");
            assert_eq!(
                (stage.width, stage.height),
                (size.width as i32 * factor, size.height as i32 * factor),
                "the form is not actual size at {name}"
            );
        }

        // And only at 1x is a form pixel a screen pixel, which is what the
        // conversions may skip.
        assert!(one.zoom.is_unit(), "1x at 100% converts nothing");
        assert!(!two.zoom.is_unit(), "2x at 100% is still a conversion");
    }

    /// Fitting measures the window in screen pixels and answers in the user's.
    ///
    /// The display's scale is in the room being measured and is put back on by
    /// `set_zoom`, so counting it once is the whole job: counting it twice fits
    /// a form to half the window and reports a percentage nobody asked for.
    #[test]
    fn fitting_does_not_count_the_displays_scale_twice() {
        let (mut one, mut two) = (at_scale(1.0), at_scale(2.0));
        one.zoom_to_fit();
        two.zoom_to_fit();
        assert_eq!(
            one.zoom.percent(),
            two.zoom.percent(),
            "the same form in the same window fits at the same percentage"
        );
    }

    /// Simulating another theme does not put the chrome back at half size.
    ///
    /// `Theme::scaled` carries the display's metrics, and `set_theme` replaces
    /// the whole theme — so a switch that passes the constant straight through
    /// silently undoes the one multiplication the application is supposed to do.
    #[test]
    fn simulating_a_theme_keeps_this_displays_metrics() {
        let mut designer = at_scale(2.0);
        assert_eq!(
            designer.ui.theme().metrics,
            theme::DARK.metrics.scaled(2.0),
            "the designer did not start at this display's metrics"
        );

        // Each theme's own furniture, at this display's scale — not the same
        // numbers for all of them, because high contrast draws a thicker border
        // on purpose and that is the part being simulated.
        for expected in [theme::DARK, theme::LIGHT, theme::HIGH_CONTRAST] {
            while designer.ui.theme().name != expected.name {
                designer.cycle_theme();
            }
            assert_eq!(
                designer.ui.theme().metrics,
                expected.metrics.scaled(2.0),
                "{} lost the display's metrics",
                expected.name
            );
        }
    }

    /// The remembered window size is logical, and a run does not inflate it.
    ///
    /// `Settings` hands its size straight back to `WindowConfig`, which is
    /// logical, while the resize event that fills it in is physical. Storing one
    /// as the other doubles the window on every launch until `Settings::sane`
    /// stops it at 16,384.
    #[test]
    fn a_run_on_a_dense_display_does_not_grow_the_window() {
        let mut designer = at_scale(2.0);
        designer.remember_size(Size::new(2560, 1600));
        let settings = designer.settings();
        assert_eq!((settings.width, settings.height), (1280, 800));
    }

    /// A designer on the reference form at a display scale.
    ///
    /// The surface grows with the factor, exactly as a real one does: the window
    /// is a fixed amount of desk, and a denser display puts more pixels behind
    /// it.
    fn at_scale(scale: f32) -> Designer {
        let document = Document::open(repo("forms/reference.dform")).expect("the form opens");
        let size = Size::new(
            (WINDOW.width as f32 * scale) as u32,
            (WINDOW.height as f32 * scale) as u32,
        );
        Designer::new(size, scale, Settings::default(), document)
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
        designer.follow_previewed_tabs();
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

    /// One line with every run of whitespace squeezed to a single space.
    ///
    /// What "the same line apart from the number" has to mean: `y=8` becoming
    /// `y=16` is a character wider, so the columns somebody lined up shift with
    /// it, and that is the edit doing its job rather than reformatting.
    fn squeezed(line: &str) -> String {
        line.split_whitespace().collect::<Vec<_>>().join(" ")
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

    // ------------------------------------------------------------- preview

    #[test]
    fn preview_lets_the_form_behave_and_design_mode_stops_it() {
        // `hello.dform`, because it fits the canvas whole: the reference form is
        // wider than the pane, and a press meant for a button in it would land
        // on the inspector instead and prove nothing.
        let mut designer = designer_on("forms/hello.dform");
        let scrim = designer.chrome.scrim.expect("a scrim");
        assert!(designer.ui.visible(scrim), "designing without a scrim");

        // A press on the form's button does nothing while designing.
        let greet = designer
            .placed
            .iter()
            .find(|node| node.kind == "button")
            .expect("hello.dform has a button")
            .path
            .clone();
        let at = middle(&designer, &greet);
        let canvas = designer
            .ui
            .bounds(designer.chrome.canvas)
            .expect("a canvas");
        assert!(canvas.contains(at), "{at:?} is not over {canvas:?}");
        feed(
            &mut designer,
            &[
                button_at(ElementState::Down, at),
                button_at(ElementState::Up, at),
            ],
        );
        assert!(
            designer.ui.drain_messages().next().is_none(),
            "the form acted while it was being designed"
        );

        press_key(&mut designer, KeyCode::F5, false);
        assert!(designer.previewing());
        assert!(
            !designer.ui.visible(scrim),
            "the scrim survived into preview"
        );
        // And nothing that belongs to designing came with it.
        assert!(designer.selection.is_empty());
        assert!(
            designer.overlay.is_empty(),
            "handles were left on a running form"
        );

        // Now the same press reaches the button.
        feed(
            &mut designer,
            &[
                button_at(ElementState::Down, at),
                button_at(ElementState::Up, at),
            ],
        );
        let messages: Vec<Message> = designer.ui.drain_messages().collect();
        assert_eq!(
            messages,
            vec![Message::Fired(names_of(&designer, "greet"))],
            "the form did not act"
        );

        press_key(&mut designer, KeyCode::F5, false);
        assert!(!designer.previewing());
        let scrim = designer.chrome.scrim.expect("a scrim again");
        assert!(designer.ui.visible(scrim));
    }

    /// A tab picked while the form is running brings its page up.
    ///
    /// The strip has always moved its own highlight and fired `on-change`. What
    /// it does not do is show the page, because `Tabs` does not own that — and
    /// while previewing there is no application to own it either, so the
    /// designer stands in. See `Designer::follow_previewed_tabs`.
    #[test]
    fn a_tab_picked_on_a_running_form_brings_its_page_up() {
        let source = concat!(
            "form \"Tabs\" version=1 width=400 height=300 {\n",
            "    tabs name=sections x=0 y=0 w=400 h=300 selected=1 {\n",
            "        tab \"One\" {\n",
            "            label \"first\" name=on-one x=8 y=8 w=200 h=20\n",
            "        }\n",
            "        tab \"Two\" {\n",
            "            label \"second\" name=on-two x=8 y=8 w=200 h=20\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let mut designer = scratch("running-tabs", source);
        press_key(&mut designer, KeyCode::F5, false);
        assert!(designer.previewing());

        let one = designer
            .pages
            .iter()
            .find(|page| page.ordinal == 0)
            .expect("a first page")
            .id;
        let two = designer
            .pages
            .iter()
            .find(|page| page.ordinal == 1)
            .expect("a second page")
            .id;
        assert!(
            designer.ui.visible(two) && !designer.ui.visible(one),
            "the file says the second tab opens"
        );

        // Tab zero starts at the strip's left edge whatever the labels say,
        // which is the one point that needs no arithmetic over their widths.
        let strip = designer
            .path_bounds(&path_named(&designer, "sections"))
            .expect("laid out");
        let at = Point::new(strip.x + 2, strip.y + 4);
        feed(
            &mut designer,
            &[
                button_at(ElementState::Down, at),
                button_at(ElementState::Up, at),
            ],
        );

        assert!(
            designer.ui.visible(one),
            "the picked tab's page did not come up"
        );
        assert!(
            !designer.ui.visible(two),
            "the page that was showing stayed up"
        );
    }

    /// The title bar names the open file, and says when it is unsaved.
    ///
    /// `Document::label` is documented as what goes in the title bar and had
    /// never reached one: `WindowConfig::title` is read when the window is
    /// made, so a form opened after start-up left the bar naming the one
    /// before it.
    #[test]
    fn the_title_bar_names_the_file_that_is_open() {
        let mut designer = scratch(
            "titled",
            "form \"T\" version=1 width=80 height=60 {\n    label \"a\" name=a x=0 y=0 w=9 h=9\n}\n",
        );
        let title = designer.window_title().to_string();
        assert!(
            title.contains("denise-designer-titled"),
            "the bar does not name the file: {title:?}"
        );
        assert!(
            !title.contains('•'),
            "nothing has been edited yet: {title:?}"
        );

        // An edit, moved the way `the_title_says_when_there_is_unsaved_work`
        // moves one, and the bar says so as the toolbar does.
        let at = middle(&designer, &path_named(&designer, "a"));
        click_at(&mut designer, at, false);
        press_key(&mut designer, KeyCode::ArrowRight, false);
        assert!(
            designer.window_title().contains('•'),
            "an edited form is not marked unsaved: {:?}",
            designer.window_title()
        );
    }

    /// A window that changed size puts the form back in the middle of it.
    ///
    /// The centring is computed where the tree is built, so before this the
    /// form kept the place the *old* viewport gave it — off to one side, and
    /// clipped if the window shrank.
    #[test]
    fn a_resized_window_puts_the_form_back_in_the_middle() {
        let mut designer = scratch(
            "resized",
            "form \"R\" version=1 width=200 height=120 {\n    label \"a\" x=0 y=0 w=9 h=9\n}\n",
        );

        assert!(off_centre(&designer) <= 1, "not centred to begin with");

        // A wider window, the way the event loop delivers one.
        let bigger = Size::new(WINDOW.width + 600, WINDOW.height + 200);
        designer.remember_size(bigger);
        designer.ui.handle(&[InputEvent::SurfaceResized {
            size: bigger,
            scale_factor: 1.0,
        }]);
        designer.settle_resize();

        assert!(
            off_centre(&designer) <= 1,
            "the form was left where the old viewport put it: off by {}",
            off_centre(&designer)
        );
    }

    /// How far from centred the form sits in the canvas, in pixels.
    fn off_centre(designer: &Designer) -> i32 {
        let view = designer
            .ui
            .bounds(designer.chrome.canvas)
            .expect("a canvas");
        let stage = designer.ui.bounds(designer.chrome.stage).expect("a stage");
        ((stage.x - view.x) - (view.right() - stage.right())).abs()
    }

    /// A window resized mid-drag is placed when the pointer comes up.
    ///
    /// The mirror of `nothing_is_read_from_under_a_drag_in_flight`: a rebuild
    /// hands out new `NodeId`s and the drag is holding the old ones. The resize
    /// is deferred rather than dropped.
    #[test]
    fn a_resize_under_a_drag_waits_for_the_pointer() {
        let mut designer = scratch(
            "resized-mid-drag",
            "form \"D\" version=1 width=200 height=120 {\n    label \"a\" name=a x=0 y=0 w=9 h=9\n}\n",
        );
        assert!(designer.select_named("a"));
        designer.drag_selection(0, 4);
        assert!(designer.drag.is_some(), "no drag to be mid-way through");

        let bigger = Size::new(WINDOW.width + 600, WINDOW.height);
        designer.remember_size(bigger);
        designer.ui.handle(&[InputEvent::SurfaceResized {
            size: bigger,
            scale_factor: 1.0,
        }]);
        designer.settle_resize();
        assert!(
            off_centre(&designer) > 1,
            "the tree was rebuilt out from under the drag"
        );

        designer.release();
        designer.settle_resize();
        assert!(
            off_centre(&designer) <= 1,
            "the resize was dropped along with the drag: off by {}",
            off_centre(&designer)
        );
    }

    /// A rebuild while previewing keeps the tab that was picked.
    ///
    /// The strip is made from the file's `selected`, and the page from
    /// `looking_at`. They have to agree or a resize -- which now rebuilds --
    /// would snap a previewed form back to the tab the file opens on.
    #[test]
    fn a_rebuild_keeps_the_tab_that_was_picked_while_previewing() {
        let source = concat!(
            "form \"Tabs\" version=1 width=400 height=300 {\n",
            "    tabs name=sections x=0 y=0 w=400 h=300 selected=1 {\n",
            "        tab \"One\" {\n",
            "            label \"first\" name=on-one x=8 y=8 w=200 h=20\n",
            "        }\n",
            "        tab \"Two\" {\n",
            "            label \"second\" name=on-two x=8 y=8 w=200 h=20\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let mut designer = scratch("rebuilt-tabs", source);
        press_key(&mut designer, KeyCode::F5, false);

        let strip = designer
            .path_bounds(&path_named(&designer, "sections"))
            .expect("laid out");
        let at = Point::new(strip.x + 2, strip.y + 4);
        feed(
            &mut designer,
            &[
                button_at(ElementState::Down, at),
                button_at(ElementState::Up, at),
            ],
        );
        let one = designer
            .pages
            .iter()
            .find(|page| page.ordinal == 0)
            .expect("a first page")
            .id;
        assert!(designer.ui.visible(one), "the pick did not take");

        // What a resize now does.
        designer.show_form();

        let one = designer
            .pages
            .iter()
            .find(|page| page.ordinal == 0)
            .expect("a first page")
            .id;
        assert!(
            designer.ui.visible(one),
            "the rebuild threw the picked tab away"
        );
        let id = designer
            .node_id(&path_named(&designer, "sections"))
            .expect("a strip");
        let tabs = designer
            .ui
            .widget::<Tabs<Message>>(id)
            .expect("a tabs widget");
        assert_eq!(
            tabs.selected(),
            0,
            "the strip highlights a different tab from the page it is over"
        );
    }

    /// The index the log knows a message name by.
    fn names_of(designer: &Designer, name: &str) -> usize {
        designer
            .names
            .iter()
            .position(|held| held == name)
            .unwrap_or_else(|| {
                panic!(
                    "`{name}` is not a message this form uses: {:?}",
                    designer.names
                )
            })
    }

    fn button_at(state: ElementState, at: Point) -> InputEvent {
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state,
            position: at,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn a_message_the_form_fires_shows_up_in_the_log_by_name() {
        // The whole point of the log: `on-press=retry` is wired, and this says
        // so without an application being written first.
        let mut designer = designer_on("forms/reference.dform");
        press_key(&mut designer, KeyCode::F5, false);

        designer.handle(Message::Fired(names_of(&designer, "retry")));
        assert_eq!(designer.fired, vec!["retry"]);

        // One carrying a value says what the value was.
        let notify = names_of(&designer, "set-notify");
        designer.handle(Message::FiredBool(notify, true));
        assert_eq!(designer.fired, vec!["retry", "set-notify(true)"]);

        let volume = names_of(&designer, "set-volume");
        designer.handle(Message::FiredNumber(volume, 42.5));
        assert_eq!(designer.fired.last().unwrap(), "set-volume(42.5)");

        // A strip, not a transcript.
        for _ in 0..20 {
            designer.handle(Message::Fired(names_of(&designer, "save")));
        }
        assert_eq!(designer.fired.len(), LOGGED);
    }

    #[test]
    fn typing_into_a_field_while_previewing_and_going_back_forgets_it() {
        // The reset the issue asks for, and there is no snapshot behind it: the
        // file is the state, so rebuilding from the file *is* the reset.
        let mut designer = designer_on("forms/reference.dform");
        assert!(designer.select_named("secret"));
        let field = designer.selected().expect("selected");
        let before = text(&designer);

        press_key(&mut designer, KeyCode::F5, false);
        designer.ui.focus(Some(field));
        for character in "hello".chars() {
            feed(&mut designer, &[InputEvent::Text { ch: character }]);
        }
        assert_eq!(
            designer
                .ui
                .widget::<TextInput<Message>>(field)
                .expect("a field")
                .text(),
            "hello",
            "the form would not take the keystrokes"
        );
        assert_eq!(
            text(&designer),
            before,
            "running the form wrote to the file"
        );

        press_key(&mut designer, KeyCode::F5, false);
        assert!(designer.select_named("secret"));
        let field = designer.selected().expect("selected");
        assert_eq!(
            designer
                .ui
                .widget::<TextInput<Message>>(field)
                .expect("a field")
                .text(),
            "",
            "what was typed survived going back to designing"
        );
    }

    #[test]
    fn escape_out_of_a_running_form_goes_back_to_designing_it_rather_than_leaving() {
        let mut designer = designer_on("forms/hello.dform");
        press_key(&mut designer, KeyCode::F5, false);
        assert!(designer.previewing());

        designer.request_exit();
        assert!(!designer.previewing(), "Escape did not come out of preview");
        assert!(!designer.exit_requested(), "Escape closed the designer");

        // And now it means what it always meant.
        designer.request_exit();
        assert!(designer.exit_requested());
    }

    #[test]
    fn the_log_strip_has_no_height_until_there_is_something_to_put_in_it() {
        let mut designer = designer_on("forms/hello.dform");
        let strip = designer.chrome.log;
        assert_eq!(designer.ui.bounds(strip).map(|r| r.height), Some(0));

        press_key(&mut designer, KeyCode::F5, false);
        assert_eq!(designer.ui.bounds(strip).map(|r| r.height), Some(LOG));
        // Across the whole width, above the status line.
        let bounds = designer.ui.bounds(strip).expect("laid out");
        assert_eq!(bounds.x, 0);
        assert_eq!(bounds.width, WINDOW.width as i32);
        assert_eq!(bounds.y + bounds.height, WINDOW.height as i32 - STATUS);

        press_key(&mut designer, KeyCode::F5, false);
        assert_eq!(designer.ui.bounds(strip).map(|r| r.height), Some(0));
    }

    #[test]
    fn the_theme_control_walks_the_built_in_themes_and_says_which() {
        let mut designer = designer_on("forms/reference.dform");
        // `reference.dform` says `theme=dark`, and that is where it starts.
        assert_eq!(designer.ui.theme().name, theme::DARK.name);

        designer.cycle_theme();
        assert_eq!(designer.ui.theme().name, theme::DARK.name);
        assert!(designer.status.contains("dark"), "{}", designer.status);

        designer.cycle_theme();
        assert_eq!(designer.ui.theme().name, theme::LIGHT.name);
        designer.cycle_theme();
        assert_eq!(designer.ui.theme().name, theme::HIGH_CONTRAST.name);

        // Round again to the form's own.
        designer.cycle_theme();
        assert_eq!(designer.simulated, Simulated::Own);
        designer.apply_theme();
        assert_eq!(designer.ui.theme().name, theme::DARK.name);
    }

    #[test]
    fn the_keyboard_comes_up_for_a_field_while_previewing_and_never_while_designing() {
        let mut designer = designer_on("forms/hello.dform");
        assert!(designer.select_named("who"));
        let field = designer.selected().expect("selected");

        // Designing: focus goes where it is put and no keyboard appears.
        designer.ui.focus(Some(field));
        designer.keyboard_turn(0);
        assert!(
            !designer.keyboard_open(),
            "a keyboard over a form being designed"
        );

        press_key(&mut designer, KeyCode::F5, false);
        assert!(designer.select_named("who"));
        let field = designer.selected().expect("selected");
        designer.ui.focus(Some(field));
        designer.keyboard_turn(0);
        assert!(
            designer.keyboard_open(),
            "no keyboard for a field in a running form"
        );
        // And it is over the bottom of the surface, which is the thing being
        // checked: how much of the form it covers.
        let covered = designer
            .keyboard
            .occluded(&designer.ui)
            .expect("the keyboard occludes something");
        assert!(covered.height > 0, "{covered:?}");
        assert!(
            designer.status.contains("keyboard is up"),
            "{}",
            designer.status
        );

        // Going back takes it away.
        press_key(&mut designer, KeyCode::F5, false);
        assert!(!designer.keyboard_open());
    }

    #[test]
    fn preview_takes_nothing_of_the_designers_own_input() {
        // The canvas, the palette and the outline all stop being design mode's.
        let mut designer = designer_on("forms/hello.dform");
        press_key(&mut designer, KeyCode::F5, false);

        // Greyed out, so a press on a row reaches nothing and answers nothing.
        let row = palette_point(&mut designer, "button");
        click_at(&mut designer, row, false);
        let answered: Vec<Message> = designer.ui.drain_messages().collect();
        assert!(answered.is_empty(), "the palette answered: {answered:?}");
        assert_eq!(
            designer.placing,
            Placing::Idle,
            "the palette armed a widget"
        );

        let card = path_named(&designer, "card");
        let at = middle(&designer, &card);
        click_at(&mut designer, at, false);
        assert!(
            designer.selection.is_empty(),
            "the canvas selected something"
        );

        // And they answer again once the form stops running.
        press_key(&mut designer, KeyCode::F5, false);
        let row = palette_point(&mut designer, "button");
        click_at(&mut designer, row, false);
        assert_eq!(
            designer.placing,
            Placing::Armed { kind: "button" },
            "the palette stayed grey"
        );
    }

    // ------------------------------------------------------------- outline

    /// The rows the outline is showing, as `depth:kind name`.
    fn shown_rows(designer: &Designer) -> Vec<String> {
        designer
            .outline
            .as_ref()
            .expect("an outline")
            .rows
            .iter()
            .map(|row| match &row.name {
                Some(name) => format!("{}:{} {name}", row.depth, row.kind),
                None => format!("{}:{}", row.depth, row.kind),
            })
            .collect()
    }

    /// A point inside a row of the outline, in one of its three zones.
    ///
    /// Scrolls the row into view first, which is what the wheel is for: a form
    /// has more nodes than the pane has rows, and a press below the pane is a
    /// press on nothing.
    fn outline_point(designer: &mut Designer, path: &[usize], part: outline::Hit) -> Point {
        let pane = designer.outline.as_ref().expect("an outline");
        let row = pane
            .row_of(path)
            .unwrap_or_else(|| panic!("{path:?} is not showing"));
        let depth = pane.rows[row].depth as i32;

        // Only when it is not already showing, so two points taken one after
        // the other — the ends of a drag — are in the same coordinates.
        let view = designer.chrome.outline_view;
        let bounds = designer.ui.bounds(view).expect("a viewport");
        let top = row as i32 * outline::ROW;
        let scroll = designer.ui.scroll(view);
        if top < scroll.y || top + outline::ROW > scroll.y + bounds.height {
            designer.ui.set_scroll(view, Point::new(0, top));
        }
        let scroll = designer.ui.scroll(view);
        let bounds = designer.ui.bounds(view).expect("a viewport");

        let width = Settings::default().left - GAP * 2;
        let x = match part {
            outline::Hit::Fold => depth * 10 + 4,
            outline::Hit::Eye => width - 4,
            outline::Hit::Body => depth * 10 + 40,
        };
        Point::new(
            bounds.x + x,
            bounds.y + row as i32 * outline::ROW - scroll.y + outline::ROW / 2,
        )
    }

    #[test]
    fn the_outline_lists_every_node_indented_and_not_only_the_named_ones() {
        let designer = designer_on("forms/hello.dform");
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:panel card",
                "1:label",
                "1:label",
                "1:text-input who",
                "1:button",
                "1:label greeting",
            ],
            "the canvas cannot show what this pane is for"
        );
    }

    #[test]
    fn the_triangle_folds_a_subtree_away_and_back() {
        let mut designer = designer_on("forms/hello.dform");
        let card = path_named(&designer, "card");
        assert_eq!(shown_rows(&designer).len(), 6);

        let at = outline_point(&mut designer, &card, outline::Hit::Fold);
        click_at(&mut designer, at, false);
        assert_eq!(shown_rows(&designer), vec!["0:panel card"]);
        // Folding is not selecting: the press was on the triangle.
        assert!(designer.selection.is_empty(), "{:?}", designer.selection);

        click_at(&mut designer, at, false);
        assert_eq!(shown_rows(&designer).len(), 6);
    }

    #[test]
    fn selecting_in_either_pane_selects_in_the_other() {
        let mut designer = designer_on("forms/reference.dform");
        let volume = path_named(&designer, "volume");

        // Canvas to outline.
        let at = middle(&designer, &volume);
        click_at(&mut designer, at, false);
        let pane = designer.outline.as_ref().expect("an outline");
        assert!(
            pane.row_of(&volume).is_some(),
            "the row is not even showing"
        );

        // Outline to canvas: a different node, picked in the pane.
        let stars = path_named(&designer, "stars");
        let at = outline_point(&mut designer, &stars, outline::Hit::Body);
        click_at(&mut designer, at, false);
        assert_eq!(designer.selection, vec![stars.clone()]);
        assert_eq!(
            designer.ui.kind(designer.selected().unwrap()),
            Some("rating")
        );
        // And the canvas drew handles round it, which is the other half of
        // "selection is shared".
        assert_eq!(designer.overlay.len(), 10, "the canvas did not follow");
    }

    #[test]
    fn shift_in_the_outline_adds_to_the_selection() {
        let mut designer = designer_on("forms/reference.dform");
        let first = path_named(&designer, "notify");
        let second = path_named(&designer, "dark");

        let at = outline_point(&mut designer, &first, outline::Hit::Body);
        click_at(&mut designer, at, false);
        let at = outline_point(&mut designer, &second, outline::Hit::Body);
        click_at(&mut designer, at, true);
        assert_eq!(designer.selection.len(), 2, "{:?}", designer.selection);
    }

    #[test]
    fn the_eye_hides_a_node_here_and_the_file_never_learns() {
        let mut designer = designer_on("forms/reference.dform");
        let field = path_named(&designer, "full-name");
        let panel = path_named(&designer, "form-section");
        let before = text(&designer);

        // Clicking the canvas there finds the field.
        let at = middle(&designer, &field);
        click_at(&mut designer, at, false);
        assert_eq!(designer.selection, vec![field.clone()]);

        let eye = outline_point(&mut designer, &field, outline::Hit::Eye);
        click_at(&mut designer, eye, false);
        assert!(!designer.ui.visible(designer.node_id(&field).unwrap()));
        assert_eq!(text(&designer), before, "the eye wrote to the file");
        assert!(
            !designer.history.is_dirty(),
            "the eye made the form modified"
        );

        // And now the same press reaches what was behind it.
        click_at(&mut designer, at, false);
        assert_eq!(designer.selection, vec![panel], "{:?}", designer.selection);

        // Opening the eye again puts it back.
        let eye = outline_point(&mut designer, &field, outline::Hit::Eye);
        click_at(&mut designer, eye, false);
        assert!(designer.ui.visible(designer.node_id(&field).unwrap()));
        click_at(&mut designer, at, false);
        assert_eq!(designer.selection, vec![field]);
    }

    #[test]
    fn the_eye_does_not_reveal_what_the_file_itself_hides() {
        let mut designer = designer_on("forms/reference.dform");
        let scrim = path_named(&designer, "scrim");
        assert!(!designer.ui.visible(designer.node_id(&scrim).unwrap()));

        // Hiding and showing it again leaves it as the file left it.
        designer.toggle_hidden(&scrim);
        designer.toggle_hidden(&scrim);
        assert!(
            !designer.ui.visible(designer.node_id(&scrim).unwrap()),
            "the eye overrode `visible=#false`"
        );
    }

    /// A small form: two panels and a label, so every row fits the pane at once.
    fn two_panels() -> Designer {
        let source = concat!(
            "form \"Two\" version=1 width=300 height=200 {\n",
            "    label \"loose\" name=loose x=4 y=4 w=80 h=20\n",
            "    panel name=left x=4 y=30 w=120 h=120 {\n",
            "        label \"in-left\" name=inside x=4 y=4 w=80 h=20\n",
            "    }\n",
            "    panel name=right x=140 y=30 w=120 h=120\n",
            "}\n",
        );
        scratch("two", source)
    }

    // ---------------------------------------------------------------- zoom

    /// **The numbers never change.** This is the whole of #154.
    ///
    /// A drag of twenty form pixels writes twenty, at 50%, at 100% and at 400%.
    /// If the conversion were missing anywhere the file would move by the
    /// screen distance instead — twice as far at 200%, half as far at 50% — and
    /// the file would silently disagree with what was drawn.
    #[test]
    fn a_drag_writes_form_pixels_at_every_magnification() {
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            // Snapping off, so this measures the conversion rather than the
            // grid: with it on a twenty-pixel drag is *supposed* to land
            // somewhere else. That it lands on the same somewhere else at every
            // magnification is the next test.
            designer.snapping = false;
            designer.set_zoom(Zoom::at(percent));
            let was = rect_of(&designer, "loose");

            assert!(designer.select_named("loose"));
            designer.drag_selection(20, 12);
            designer.release();

            let now = rect_of(&designer, "loose");
            assert_eq!(
                (now.x - was.x, now.y - was.y),
                (20, 12),
                "at {percent}% a drag of 20,12 form pixels wrote {},{}",
                now.x - was.x,
                now.y - was.y
            );
            assert_eq!(
                (now.width, now.height),
                (was.width, was.height),
                "at {percent}% a move resized it"
            );
        }
    }

    /// And with snapping on, the magnification still does not reach the file.
    ///
    /// The grid and the sibling edges are the **file's**, so the same gesture
    /// has to settle on the same numbers whatever the canvas is showing. A
    /// `SNAP` measured on screen instead would make a form four times stickier
    /// at 400%, and the file would record where the pointer happened to be
    /// rather than where it was aimed.
    #[test]
    fn snapping_settles_on_the_same_numbers_at_every_magnification() {
        let mut landed: Vec<(u16, Rect)> = Vec::new();
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            assert!(designer.select_named("loose"));
            designer.drag_selection(20, 12);
            designer.release();
            landed.push((percent, rect_of(&designer, "loose")));
        }
        let (_, first) = landed[0];
        for (percent, rect) in &landed {
            assert_eq!(
                *rect, first,
                "at {percent}% the same drag snapped somewhere else"
            );
        }
        // And it really did snap, or this would be testing nothing.
        assert_ne!(first, rect_of(&two_panels(), "loose"));
    }

    /// And what the inspector shows is what the file says, at any zoom.
    ///
    /// The pane reads the *tree*, which is in screen units — so without the
    /// conversion a 300x200 form would report 600x400 at 200%, and typing into
    /// the field would then write that back.
    #[test]
    fn the_inspector_reports_design_pixels_at_every_magnification() {
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            // Selecting alone, so this is the pane as `value_of` builds it. An
            // earlier version called `sync_rect` here and tested only the path
            // that redraws the four rows during a drag — which was already
            // converted, so it passed while the build path was still showing
            // magnified numbers.
            assert!(designer.select_named("right"));

            assert_eq!(
                shown_rect(&designer),
                rect_of(&designer, "right"),
                "at {percent}% the inspector and the file disagree"
            );

            // And again through the drag path, which is a different reader.
            designer.sync_rect();
            assert_eq!(
                shown_rect(&designer),
                rect_of(&designer, "right"),
                "at {percent}% a redraw of the rows disagrees with the file"
            );
        }
    }

    /// Every property measured in pixels reads as the file wrote it, too.
    ///
    /// `Form::build_scaled` multiplies them into the widget — that is how a
    /// magnified form gets thicker borders rather than the same ones in a bigger
    /// box — so the inspector has to divide them back out. A spinner whose file
    /// says `thickness=3` holds 12 at 400%, and showing 12 would be showing a
    /// number nobody wrote and inviting somebody to edit it.
    #[test]
    fn a_property_measured_in_pixels_reads_as_the_file_wrote_it() {
        for percent in [50, 100, 200, 400] {
            let mut designer = designer_on("forms/reference.dform");
            designer.set_zoom(Zoom::at(percent));
            assert!(designer.select_named("busy"));

            let pane = designer.inspector.as_ref().expect("an inspector");
            let row = pane
                .rows
                .iter()
                .find(|row| row.property.name == "thickness")
                .expect("a spinner has a thickness");
            assert!(row.property.pixels, "thickness is a length");
            assert_eq!(
                row.shown,
                designer
                    .document
                    .form()
                    .property(&path_named(&designer, "busy"), "thickness")
                    .expect("the file writes it"),
                "at {percent}% the inspector shows a thickness nobody wrote"
            );
        }
    }

    /// Typing a number into the inspector means that number, at any zoom.
    ///
    /// The other direction of the same conversion: `place` takes a form number
    /// and the tree wants a screen one.
    #[test]
    fn a_typed_rectangle_means_form_pixels_at_every_magnification() {
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            assert!(designer.select_named("right"));
            let id = designer.selected().expect("selected");

            assert!(designer.place(id, "x", "160"));
            let layout = designer.ui.layout(id).expect("a layout");
            assert_eq!(
                designer.in_form(layout).x,
                160,
                "at {percent}% typing x=160 put it somewhere else"
            );
        }
    }

    /// Every command that reads a rectangle to write one is in form pixels too.
    ///
    /// A drag is the obvious crossing and not the only one: nudging, aligning,
    /// grouping and ungrouping all take what the tree holds and put it in the
    /// file. Each was written when those were the same thing.
    #[test]
    fn the_commands_that_move_nodes_write_form_pixels_at_every_magnification() {
        // Nudging: one arrow key is one pixel in the file.
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            let was = rect_of(&designer, "right");
            assert!(designer.select_named("right"));
            designer.nudge(3, -2);
            let now = rect_of(&designer, "right");
            assert_eq!(
                (now.x - was.x, now.y - was.y),
                (3, -2),
                "at {percent}% a nudge of 3,-2 moved it {},{}",
                now.x - was.x,
                now.y - was.y
            );
        }

        // Aligning: the anchor does not move and the others land on it.
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            designer.selection = vec![
                path_named(&designer, "right"),
                path_named(&designer, "left"),
            ];
            designer.selected = designer.selection.last().and_then(|p| designer.node_id(p));
            let anchor = rect_of(&designer, "left");
            designer.arrange(Command::Left);
            assert_eq!(
                rect_of(&designer, "right").x,
                anchor.x,
                "at {percent}% aligning left did not line them up in the file"
            );
            assert_eq!(rect_of(&designer, "left"), anchor, "the anchor moved");
        }

        // Grouping: the panel is the bounding box, in form pixels, and nothing
        // inside it appears to move.
        for percent in [50, 100, 200, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            let (left, right) = (rect_of(&designer, "left"), rect_of(&designer, "right"));
            designer.selection = vec![
                path_named(&designer, "left"),
                path_named(&designer, "right"),
            ];
            designer.selected = designer.selection.last().and_then(|p| designer.node_id(p));
            designer.arrange(Command::Group);

            let held = rect_of(&designer, "left");
            assert_eq!(
                (held.x, held.y),
                (0, 0),
                "at {percent}% the first node is not at the group's corner"
            );
            let other = rect_of(&designer, "right");
            assert_eq!(
                (other.x, other.y),
                (right.x - left.x, right.y - left.y),
                "at {percent}% grouping moved the second one"
            );
        }
    }

    /// And typing into such a row writes the file's number, not the screen's.
    ///
    /// The other direction of the same conversion. Without it, typing `6` into
    /// `thickness` at 400% would write 6 to the file — correctly — and then draw
    /// a hairline until something rebuilt the form.
    #[test]
    fn typing_a_length_writes_what_was_typed_and_draws_it_magnified() {
        for percent in [100, 200, 400] {
            let mut designer = designer_on("forms/reference.dform");
            designer.set_zoom(Zoom::at(percent));
            assert!(designer.select_named("busy"));
            let id = designer.selected().expect("selected");

            let row = designer
                .inspector
                .as_ref()
                .expect("an inspector")
                .rows
                .iter()
                .position(|row| row.property.name == "thickness")
                .expect("a spinner has a thickness");
            designer.commit(row, String::from("6"));

            assert_eq!(
                designer
                    .document
                    .form()
                    .property(&path_named(&designer, "busy"), "thickness")
                    .as_deref(),
                Some("6"),
                "at {percent}% the file did not get the number that was typed"
            );
            assert_eq!(
                designer.ui.get_property(id, "thickness"),
                Some(Value::Int(Zoom::at(percent).on_screen_n(6))),
                "at {percent}% the widget is not drawing it magnified"
            );
        }
    }

    /// The stage is the form's own size, magnified — and nothing else is.
    #[test]
    fn the_stage_is_the_form_at_the_magnification_asked_for() {
        let mut designer = two_panels();
        let design = designer.document.form().size();

        for percent in [50, 100, 200, 400] {
            designer.set_zoom(Zoom::at(percent));
            let stage = designer
                .ui
                .bounds(designer.chrome.stage)
                .expect("the stage is there");
            let want = Zoom::at(percent).on_screen_size(design);
            assert_eq!(
                (stage.width, stage.height),
                (want.width as i32, want.height as i32),
                "the stage is the wrong size at {percent}%"
            );
            // And the document is untouched by any of it.
            assert_eq!(designer.document.form().size(), design);
        }
    }

    /// Magnifying is not an edit: nothing to undo, and nothing written.
    #[test]
    fn changing_the_magnification_does_not_touch_the_document() {
        let mut designer = two_panels();
        let before = text(&designer);

        designer.zoom_in();
        designer.zoom_in();
        designer.zoom_out();
        designer.zoom_to_fit();
        designer.zoom_actual();

        assert_eq!(text(&designer), before, "zooming wrote to the file");
        assert!(!designer.stale, "zooming left the form behind the file");
    }

    /// A grab handle is the display's size, not the form's.
    ///
    /// The one thing over the canvas that does not scale with what is under it:
    /// it is aimed at by a hand, and a hand does not get smaller when the form
    /// does. At 25% a handle that scaled with the form would be under two
    /// pixels across.
    #[test]
    fn a_grab_handle_keeps_its_size_however_far_the_form_is_zoomed() {
        for percent in [25, 100, 400] {
            let mut designer = two_panels();
            designer.set_zoom(Zoom::at(percent));
            assert!(designer.select_named("right"));
            designer.refresh_overlay();

            let bounds = designer
                .path_bounds(&path_named(&designer, "right"))
                .expect("bounds");
            let handle = Grip::handle_rect(bounds, Grip::HANDLES[0], canvas::HANDLE);
            assert_eq!(
                (handle.width, handle.height),
                (canvas::HANDLE, canvas::HANDLE),
                "the handle changed size at {percent}%"
            );
            // And what can be pressed is what is drawn: the corner is a grip.
            let corner = Point::new(bounds.x, bounds.y);
            assert!(
                matches!(
                    Grip::at(bounds, corner, canvas::HANDLE),
                    Some(Grip::Resize { .. })
                ),
                "the corner is not a resize grip at {percent}%"
            );
        }
    }

    /// What the four rectangle rows are showing.
    fn shown_rect(designer: &Designer) -> Rect {
        let pane = designer.inspector.as_ref().expect("an inspector");
        let axis = |what: &str| {
            pane.rows
                .iter()
                .find(|row| row.property.name == what && row.node)
                .and_then(|row| row.shown.parse::<i32>().ok())
                .unwrap_or_default()
        };
        Rect::new(axis("x"), axis("y"), axis("w"), axis("h"))
    }

    // --------------------------------------------------------- collections

    /// A form with a select whose options carry a comment.
    fn with_options() -> Designer {
        let source = concat!(
            "form \"Opts\" version=1 width=300 height=200 {\n",
            "    select name=job x=4 y=4 w=120 h=24 {\n",
            "        option \"Reader\"\n",
            "        // the usual one\n",
            "        option \"Author\"\n",
            "    }\n",
            "}\n",
        );
        scratch("opts", source)
    }

    /// The row a list property is on, for the selected node.
    fn list_row_of(designer: &Designer, name: &str) -> usize {
        designer
            .inspector
            .as_ref()
            .expect("an inspector")
            .rows
            .iter()
            .position(|row| row.property.name == name)
            .expect("the widget describes its collection")
    }

    /// A table whose rows are placeholder content, with and without the block.
    fn with_rows(design: bool) -> Designer {
        let rows = if design {
            concat!(
                "        design {\n",
                "            row \"Ada\"\n",
                "            row \"Grace\"\n",
                "        }\n",
            )
        } else {
            ""
        };
        let source = format!(
            concat!(
                "form \"Recs\" version=1 width=300 height=200 {{\n",
                "    table name=t x=4 y=4 w=280 h=120 {{\n",
                "        column \"Name\"\n",
                "{}",
                "    }}\n",
                "}}\n",
            ),
            rows
        );
        scratch("recs", &source)
    }

    /// The inspector edits placeholder rows the same way it edits real content,
    /// and writes them where the engine will not load them.
    ///
    /// #160: a `row` is a `PropertyKind::Placeholder`, which differs from a
    /// `List` in where it lives and who builds it, and not at all in what the
    /// pane does with one.
    #[test]
    fn the_inspector_edits_the_rows_a_table_shows_on_a_canvas() {
        let mut designer = with_rows(true);
        assert!(designer.select_named("t"));
        let row = list_row_of(&designer, "row");

        let pane = designer.inspector.as_ref().expect("an inspector");
        assert_eq!(pane.rows[row].shown, "2 rows");

        designer.add_item(row);
        let path = path_named(&designer, "t");
        assert_eq!(
            designer.document.form().items(&path, "row"),
            ["Ada", "Grace", "row"],
            "adding did not append one"
        );
        // And it went inside `design`, not onto the table.
        let text = designer.document.form().text();
        assert!(text.contains("design {"), "{text}");
        let design = text.split("design {").nth(1).expect("the block");
        assert!(
            design.contains("row \"row\""),
            "the new row is outside: {text}"
        );
    }

    /// The first placeholder brings the block with it.
    #[test]
    fn adding_the_first_row_writes_the_design_block_too() {
        let mut designer = with_rows(false);
        assert!(designer.select_named("t"));
        let row = list_row_of(&designer, "row");
        // An empty collection shows nothing, the same as a `select` with no
        // options: the row is there because the widget has the property.
        assert_eq!(
            designer.inspector.as_ref().expect("a pane").rows[row].shown,
            ""
        );

        designer.add_item(row);
        let text = designer.document.form().text();
        assert!(text.contains("design {"), "no block was written: {text}");
        assert_eq!(
            designer
                .document
                .form()
                .items(&path_named(&designer, "t"), "row"),
            ["row"],
        );
        // Still a form the engine will read, block and all.
        denise_forms::Form::parse(&text).expect("the result parses");

        // And one undo takes the whole thing back, block included.
        designer.undo();
        assert!(
            !designer.document.form().text().contains("design {"),
            "undo left the block behind: {}",
            designer.document.form().text()
        );
    }

    /// The inspector shows a `select`'s options, and adds one.
    ///
    /// #105's own acceptance: a collection is the widget's content, written as
    /// child nodes, and the pane edits it where it lives.
    #[test]
    fn the_inspector_edits_the_options_a_select_holds() {
        let mut designer = with_options();
        assert!(designer.select_named("job"));
        let row = list_row_of(&designer, "option");

        let pane = designer.inspector.as_ref().expect("an inspector");
        assert_eq!(
            pane.rows[row].shown, "2 options",
            "the row does not say how many there are"
        );

        designer.add_item(row);
        assert_eq!(
            designer
                .document
                .form()
                .items(&path_named(&designer, "job"), "option"),
            ["Reader", "Author", "option"],
            "adding did not append one"
        );
        assert!(
            text(&designer).contains("// the usual one"),
            "the comment went"
        );
    }

    /// Removing and reordering reach the file, and carry the comment.
    ///
    /// The reason the items are edited where they live rather than rewritten as
    /// a block: `Edit::Move` takes a node's leading trivia with it, so a comment
    /// written above the second option stays above the second option.
    #[test]
    fn removing_and_reordering_an_option_carries_what_was_written_above_it() {
        let mut designer = with_options();
        assert!(designer.select_named("job"));
        let row = list_row_of(&designer, "option");
        let job = path_named(&designer, "job");

        designer.move_item(row, 1, 0);
        assert_eq!(
            designer.document.form().items(&job, "option"),
            ["Author", "Reader"],
            "the reorder did not reach the file"
        );
        let moved = text(&designer);
        let comment = moved
            .find("// the usual one")
            .expect("the comment is still there");
        let author = moved
            .find("option \"Author\"")
            .expect("`Author` is still there");
        assert!(
            comment < author,
            "the comment left its option behind:\n{moved}"
        );

        designer.remove_item(row, 0);
        assert_eq!(
            designer.document.form().items(&job, "option"),
            ["Reader"],
            "the removal did not reach the file"
        );
        // And the comment went with the option it explained.
        assert!(!text(&designer).contains("// the usual one"));
    }

    /// Retyping one option writes that option, and leaves the others alone.
    #[test]
    fn retyping_one_option_writes_only_that_one() {
        let mut designer = with_options();
        assert!(designer.select_named("job"));
        let row = list_row_of(&designer, "option");

        designer.commit_items(row, "option", "Reader\nEditor");
        let after = text(&designer);
        assert_eq!(
            designer
                .document
                .form()
                .items(&path_named(&designer, "job"), "option"),
            ["Reader", "Editor"]
        );
        assert!(
            after.contains("// the usual one"),
            "the untouched comment went:\n{after}"
        );

        // A list of a different length is not something a poll can mean, and is
        // refused rather than guessed at.
        designer.commit_items(row, "option", "Reader");
        assert_eq!(
            designer
                .document
                .form()
                .items(&path_named(&designer, "job"), "option"),
            ["Reader", "Editor"],
            "a shorter list was taken as an edit"
        );
    }

    /// Every collection edit is one undo step, and undo is byte-exact.
    #[test]
    fn a_collection_edit_undoes_to_the_byte() {
        for step in 0..3 {
            let mut designer = with_options();
            assert!(designer.select_named("job"));
            let row = list_row_of(&designer, "option");
            let before = text(&designer);

            match step {
                0 => designer.add_item(row),
                1 => designer.remove_item(row, 0),
                _ => designer.move_item(row, 1, 0),
            }
            assert_ne!(text(&designer), before, "step {step} changed nothing");

            designer.undo();
            assert_eq!(text(&designer), before, "step {step} did not undo exactly");
        }
    }

    /// A designer on a form file of this test's own, so two tests running at
    /// once never read a file the other is halfway through writing.
    fn scratch(name: &str, source: &str) -> Designer {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let ordinal = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("denise-designer-{name}-{ordinal}.dform"));
        std::fs::write(&path, source).expect("writing");
        let document = Document::open(&path).expect("the form opens");
        Designer::new(WINDOW, 1.0, Settings::default(), document)
    }

    /// A form whose tabs carry pages.
    fn with_tab_pages() -> Designer {
        let source = concat!(
            "form \"Tabs\" version=1 width=400 height=300 {\n",
            "    tabs name=sections x=0 y=0 w=400 h=300 selected=0 {\n",
            "        tab \"One\" {\n",
            "            button \"a\" name=a x=0 y=0 w=80 h=24\n",
            "        }\n",
            "        tab \"Two\" {\n",
            "            button \"b\" name=b x=0 y=0 w=80 h=24\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        scratch("tabs", source)
    }

    /// Selecting a widget on a tab that is not showing brings its page up.
    ///
    /// #161: a page that is not showing is not in the tree's order at all, so
    /// without this the second tab's contents could be seen in the outline and
    /// never reached. Every selection path runs through `reselected`, so this
    /// works from the outline, from a click, and from `select_named`.
    #[test]
    fn selecting_a_widget_on_another_tab_shows_that_page() {
        let mut designer = with_tab_pages();

        // The file opens on the first tab, so the second page is not showing
        // and nothing in it can be hit.
        let second = designer
            .placed
            .iter()
            .find(|p| p.name.as_deref() == Some("b"))
            .expect("the second page's button is in the outline")
            .path
            .clone();
        let id = designer.node_id(&second).expect("it was built");
        assert!(!designer.ui.visible(id) || designer.ui.hit_test(Point::new(20, 60)) != Some(id));

        // Selecting it brings its page into view.
        assert!(designer.select_named("b"));
        assert!(
            designer.ui.visible(id),
            "the page holding the selected widget is still hidden"
        );

        // And it is designer state: the file still opens on the first tab.
        assert!(
            designer.document.form().text().contains("selected=0"),
            "looking at a tab wrote to the file: {}",
            designer.document.form().text()
        );
    }

    /// Which tab the designer is looking at survives a rebuild.
    #[test]
    fn the_tab_being_looked_at_outlives_an_edit() {
        let mut designer = with_tab_pages();
        assert!(designer.select_named("b"));
        let id = designer
            .node_id(
                &designer
                    .placed
                    .iter()
                    .find(|p| p.name.as_deref() == Some("b"))
                    .expect("named")
                    .path
                    .clone(),
            )
            .expect("built");
        assert!(designer.ui.visible(id));

        // An edit rebuilds the tree from the document; the page must come back.
        designer.nudge(1, 0);
        let again = designer
            .placed
            .iter()
            .find(|p| p.name.as_deref() == Some("b"))
            .expect("still named")
            .path
            .clone();
        let id = designer.node_id(&again).expect("rebuilt");
        assert!(
            designer.ui.visible(id),
            "the rebuild put the designer back on the file's tab"
        );
    }

    #[test]
    fn dragging_a_row_onto_a_panel_reparents_it() {
        let mut designer = two_panels();
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "1:label inside",
                "0:panel right"
            ]
        );
        let loose = path_named(&designer, "loose");
        let right = path_named(&designer, "right");
        let before = text(&designer);

        let from = outline_point(&mut designer, &loose, outline::Hit::Body);
        let onto = outline_point(&mut designer, &right, outline::Hit::Body);
        drag_from_to(&mut designer, from, onto);

        let after = text(&designer);
        assert_ne!(after, before, "the drag wrote nothing");
        // Inside the panel, indented for its new depth, and the panel grew the
        // braces it did not have.
        assert!(
            after.contains("panel name=right x=140 y=30 w=120 h=120 {\n        label \"loose\""),
            "{after}"
        );
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:panel left",
                "1:label inside",
                "0:panel right",
                "1:label loose"
            ]
        );
        // And it is what is selected, so the inspector followed it.
        assert_eq!(designer.selection, vec![path_named(&designer, "loose")]);
        denise_forms::Form::parse(&after).expect("still a form");

        // One step, and exact.
        assert_eq!(designer.history.depth().0, 1);
        designer.undo();
        assert_eq!(text(&designer), before, "undoing the move was not exact");
    }

    #[test]
    fn dragging_a_row_out_of_a_panel_puts_it_on_the_form() {
        let mut designer = two_panels();
        let inside = path_named(&designer, "inside");
        let loose = path_named(&designer, "loose");
        let before = text(&designer);

        // Above the first row, which is the form's own first child.
        let from = outline_point(&mut designer, &inside, outline::Hit::Body);
        let onto = outline_point(&mut designer, &loose, outline::Hit::Body);
        let above = Point::new(onto.x, onto.y - outline::ROW / 2 + 2);
        drag_from_to(&mut designer, from, above);

        let after = text(&designer);
        assert!(
            after.contains("{\n    label \"in-left\""),
            "it kept the panel's indentation:\n{after}"
        );
        // The panel it left has no children now, so it has no braces either.
        assert!(
            after.contains("panel name=left x=4 y=30 w=120 h=120\n"),
            "an empty pair of braces was left behind:\n{after}"
        );
        denise_forms::Form::parse(&after).expect("still a form");

        designer.undo();
        assert_eq!(text(&designer), before);
    }

    #[test]
    fn dragging_a_row_between_two_others_reorders_them() {
        let mut designer = two_panels();
        let before = text(&designer);
        let right = path_named(&designer, "right");
        let loose = path_named(&designer, "loose");

        // The last of the form's children to the front of them.
        let from = outline_point(&mut designer, &right, outline::Hit::Body);
        let onto = outline_point(&mut designer, &loose, outline::Hit::Body);
        let above = Point::new(onto.x, onto.y - outline::ROW / 2 + 2);
        drag_from_to(&mut designer, from, above);

        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:panel right",
                "0:label loose",
                "0:panel left",
                "1:label inside"
            ],
            "{}",
            text(&designer)
        );
        denise_forms::Form::parse(&text(&designer)).expect("still a form");
        designer.undo();
        assert_eq!(text(&designer), before, "undoing a reorder was not exact");
    }

    #[test]
    fn a_row_cannot_be_dragged_into_itself() {
        let mut designer = two_panels();
        let left = path_named(&designer, "left");
        let inside = path_named(&designer, "inside");
        let before = text(&designer);

        let from = outline_point(&mut designer, &left, outline::Hit::Body);
        let onto = outline_point(&mut designer, &inside, outline::Hit::Body);
        drag_from_to(&mut designer, from, onto);

        assert_eq!(text(&designer), before, "a panel went inside its own child");
        assert!(designer.outline_drag.is_none(), "the drag is still going");
    }

    #[test]
    fn f2_renames_a_node_and_escape_leaves_it_alone() {
        let mut designer = designer_on("forms/hello.dform");
        assert!(designer.select_named("who"));
        let before = text(&designer);

        press_key(&mut designer, KeyCode::F2, false);
        let (_, field) = designer
            .outline
            .as_ref()
            .expect("an outline")
            .renaming
            .expect("a field over the row");
        assert_eq!(
            designer
                .ui
                .widget::<TextInput<Message>>(field)
                .unwrap()
                .text(),
            "who",
            "the field did not start with the name it has"
        );

        // Escape writes nothing.
        press_key(&mut designer, KeyCode::Escape, false);
        assert!(designer.outline.as_ref().unwrap().renaming.is_none());
        assert_eq!(text(&designer), before);

        // Enter does.
        press_key(&mut designer, KeyCode::F2, false);
        let (_, field) = designer.outline.as_ref().unwrap().renaming.unwrap();
        designer
            .ui
            .widget_mut::<TextInput<Message>>(field)
            .unwrap()
            .set_text("visitor");
        designer.handle(Message::Renamed);

        let after = text(&designer);
        assert!(after.contains("name=visitor"), "{after}");
        assert!(!after.contains("name=who"), "{after}");
        assert!(designer.outline_names().any(|name| name == "visitor"));
        assert_eq!(designer.history.depth().0, 1);
        designer.undo();
        assert_eq!(text(&designer), before, "undoing a rename was not exact");
    }

    #[test]
    fn a_node_with_no_name_can_be_given_one() {
        let mut designer = designer_on("forms/hello.dform");
        let button = vec![0, 3];
        let at = outline_point(&mut designer, &button, outline::Hit::Body);
        click_at(&mut designer, at, false);

        press_key(&mut designer, KeyCode::F2, false);
        let (_, field) = designer.outline.as_ref().unwrap().renaming.unwrap();
        assert_eq!(
            designer
                .ui
                .widget::<TextInput<Message>>(field)
                .unwrap()
                .text(),
            "",
            "it offered a name to a node that has none"
        );
        designer
            .ui
            .widget_mut::<TextInput<Message>>(field)
            .unwrap()
            .set_text("greet");
        designer.handle(Message::Renamed);

        let line = text(&designer)
            .lines()
            .find(|line| line.contains("\"Greet\""))
            .expect("the button")
            .to_string();
        assert!(line.contains("name=greet"), "{line}");
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

    /// Every kind the palette is currently offering, headings skipped.
    fn offered(designer: &Designer) -> Vec<&'static str> {
        (0..designer.shown.len())
            .filter_map(|row| designer.palette_kind(row))
            .collect()
    }

    /// The default palette wears its glyphs: every widget row carries its
    /// widget's icon, and no heading carries one.
    #[test]
    fn the_palette_rows_wear_their_glyphs_by_default() {
        let designer = designer_on("forms/hello.dform");
        assert_eq!(designer.settings().palette, PaletteMode::Both);
        let list = designer
            .ui
            .widget::<List<Message>>(designer.chrome.palette)
            .expect("a palette list");
        assert_eq!(list.items().len(), designer.shown.len());
        for (row, item) in list.items().iter().enumerate() {
            match designer.shown[row] {
                Shelf::Heading(_) => {
                    assert_eq!(item.leading_icon(), None, "a heading wears a glyph")
                }
                Shelf::Widget(_) => assert!(
                    item.leading_icon().is_some(),
                    "`{}` has no glyph",
                    item.text()
                ),
            }
        }
    }

    /// Text mode is the palette as it originally was: names, no glyphs — and
    /// the button beside the heading is what gets there.
    #[test]
    fn text_mode_strips_the_glyphs_and_the_button_says_which_mode_it_is() {
        let mut designer = designer_on("forms/hello.dform");
        designer.cycle_palette_mode();
        assert_eq!(designer.settings().palette, PaletteMode::Text);
        let list = designer
            .ui
            .widget::<List<Message>>(designer.chrome.palette)
            .expect("a palette list");
        assert!(
            list.items()
                .iter()
                .all(|item| item.leading_icon().is_none()),
            "text mode still shows glyphs"
        );
        let button = designer
            .ui
            .widget::<Button<Message>>(designer.chrome.mode_button)
            .expect("the mode button");
        assert_eq!(button.label(), "text");
    }

    /// Glyphs mode swaps the list for tiles, and the slots keep hitting: a
    /// press in the middle of a tile arms exactly what a press on its row
    /// would have, and a press on a heading arms nothing.
    #[test]
    fn a_tile_in_glyphs_mode_arms_its_widget_the_way_a_row_does() {
        let mut designer = designer_on("forms/hello.dform");
        designer.cycle_palette_mode();
        designer.cycle_palette_mode();
        assert_eq!(designer.settings().palette, PaletteMode::Glyphs);
        assert_eq!(
            designer.slots.len(),
            designer.shown.len(),
            "every entry has a slot"
        );

        let view = designer
            .ui
            .bounds(designer.chrome.palette_view)
            .expect("a palette");
        let centre = |slot: Rect| {
            Point::new(
                view.x + slot.x + slot.width / 2,
                view.y + slot.y + slot.height / 2,
            )
        };

        let row = (0..designer.shown.len())
            .find(|row| designer.palette_kind(*row) == Some("button"))
            .expect("a button tile");
        designer.press_palette(centre(designer.slots[row]));
        designer.drop_at(centre(designer.slots[row]));
        assert_eq!(
            designer.placing.kind(),
            Some("button"),
            "the tile did not arm its widget"
        );

        // Headings take a full row of their own, so the first slot is one —
        // and pressing it gives up what was armed, as it does in the list.
        assert!(matches!(designer.shown[0], Shelf::Heading(_)));
        designer.press_palette(centre(designer.slots[0]));
        assert_eq!(designer.placing.kind(), None);

        // And the way back round is a list again, glyphs and all.
        designer.cycle_palette_mode();
        assert_eq!(designer.settings().palette, PaletteMode::Both);
        assert!(
            designer
                .ui
                .widget::<List<Message>>(designer.chrome.palette)
                .is_some(),
            "cycling back did not rebuild the list"
        );
    }

    /// The events come last under their own heading, whatever order the
    /// widget listed them in — and a node with no events gets no heading.
    #[test]
    fn events_are_filed_last_under_their_own_heading() {
        let mut designer = designer_on("forms/hello.dform");
        assert!(designer.select_named("who"));
        let pane = designer.inspector.as_ref().expect("a pane");
        let first_event = pane
            .rows
            .iter()
            .position(|row| is_event(row.property))
            .expect("a text input fires on-submit");
        assert!(
            pane.rows[first_event..]
                .iter()
                .all(|row| is_event(row.property)),
            "a property came after an event"
        );
        assert!(
            pane.rows[..first_event]
                .iter()
                .all(|row| !is_event(row.property)),
            "an event came before the properties"
        );
        assert!(
            first_event > 0,
            "geometry and the widget's own properties come first"
        );

        assert!(designer.select_named("greeting"));
        let pane = designer.inspector.as_ref().expect("a pane");
        assert!(
            pane.rows.iter().all(|row| !is_event(row.property)),
            "a label fires nothing"
        );
    }

    /// An event with no name has no handler to go to, and the status line
    /// says so rather than a dialog asking for a file it could not use.
    #[test]
    fn opening_an_unnamed_event_says_so() {
        let mut designer = designer_on("forms/hello.dform");
        assert!(designer.select_named("who"));
        let index = row(&designer, "on-submit");
        let Editor::Field(field) = designer.inspector.as_ref().unwrap().rows[index].editor else {
            panic!("an event is edited in a field");
        };
        designer
            .ui
            .widget_mut::<TextInput<Message>>(field)
            .expect("the field")
            .set_text("");
        designer.open_code(index);
        assert!(
            designer.status.contains("no name yet"),
            "{}",
            designer.status
        );
    }

    /// The whole way through: the sidecar names the file, the file is written
    /// with a handler shaped for the event, opening it again writes nothing,
    /// and a double-click on the event's name is the same as the button.
    ///
    /// Unix only, because the editor is stood in for by `true`, which is a
    /// program every Unix has and Windows does not.
    #[cfg(unix)]
    #[test]
    fn opening_an_event_writes_its_handler_once_and_opens_it() {
        let dir = std::env::temp_dir().join(format!("denise-code-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let form = dir.join("hello.dform");
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../forms/hello.dform"),
            &form,
        )
        .unwrap();
        let code_path = dir.join("src").join("app.rs");
        code::write_link(&form, &code_path).unwrap();

        let mut designer = designer_on_file(&form);
        designer.settings.editor = String::from("true {file}:{line}:{column}");
        assert!(designer.select_named("who"));
        let index = row(&designer, "on-submit");

        designer.open_code(index);
        let source = std::fs::read_to_string(&code_path).expect("the file was created");
        assert!(
            source.starts_with("//! Code behind `hello.dform`"),
            "{source}"
        );
        assert!(source.contains("fn greet() {"), "{source}");
        assert!(
            source.contains("fired by `on-submit` of a `text-input`"),
            "{source}"
        );
        assert!(designer.status.starts_with("wrote "), "{}", designer.status);
        assert!(designer.status.contains("opened "), "{}", designer.status);

        designer.open_code(index);
        assert_eq!(
            std::fs::read_to_string(&code_path).unwrap(),
            source,
            "a handler that is there is not written again"
        );
        assert!(
            designer.status.starts_with("opened "),
            "{}",
            designer.status
        );

        // The double-click: two presses on the name inside the window open
        // it; a first press alone is left to the tree.
        designer.status.clear();
        let name = designer.inspector.as_ref().unwrap().rows[index]
            .name
            .expect("the name label");
        let at = designer.ui.bounds(name).expect("the label is laid out");
        let at = Point::new(at.x + at.width / 2, at.y + at.height / 2);
        let press = InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: at,
            modifiers: denise::Modifiers::NONE,
        };
        designer.keyboard_turn(1_000);
        assert!(!designer.claim(&press), "one press is not a double-click");
        assert!(designer.status.is_empty());
        designer.keyboard_turn(1_000 + code::DOUBLE_CLICK_MS);
        assert!(designer.claim(&press), "the second press opens the handler");
        assert!(
            designer.status.starts_with("opened "),
            "{}",
            designer.status
        );

        // And two slow presses are two single presses.
        designer.status.clear();
        designer.keyboard_turn(5_000);
        designer.claim(&press);
        designer.keyboard_turn(5_000 + code::DOUBLE_CLICK_MS + 1);
        assert!(!designer.claim(&press), "too slow to pair");
        assert!(designer.status.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A temporary copy of `hello.dform` beside a sidecar and a code file, for
    /// the tests that read and write the code behind it.
    fn code_behind(
        tag: &str,
        form_text: Option<&str>,
        code: &str,
        sidecar: &str,
    ) -> (std::path::PathBuf, Designer) {
        let dir = std::env::temp_dir().join(format!("denise-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let form = dir.join("hello.dform");
        let text = form_text.map_or_else(
            || {
                std::fs::read_to_string(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../forms/hello.dform"
                ))
                .unwrap()
            },
            str::to_string,
        );
        std::fs::write(&form, text).unwrap();
        std::fs::write(dir.join("src").join("app.rs"), code).unwrap();
        std::fs::write(code::sidecar(&form), sidecar).unwrap();
        let designer = designer_on_file(&form);
        (dir, designer)
    }

    /// With a handlers type in the sidecar, the placeholder is a method in
    /// that type's impl, and the status line says so.
    #[cfg(unix)]
    #[test]
    fn a_handlers_type_gets_its_placeholder_as_a_method() {
        let (dir, mut designer) = code_behind(
            "method",
            None,
            "struct App;\n\nimpl App {\n    fn new() -> Self {\n        Self\n    }\n}\n",
            "code = src/app.rs\nhandlers = App\n",
        );
        designer.settings.editor = String::from("true {file}");
        assert!(designer.select_named("who"));
        let index = row(&designer, "on-submit");
        designer.open_code(index);
        let source = std::fs::read_to_string(dir.join("src").join("app.rs")).unwrap();
        assert!(
            source.contains("    }\n\n    /// `greet` — fired by `on-submit` of a `text-input` in hello.dform.\n    fn greet(&mut self) {\n        todo!(\"greet\")\n    }\n}\n"),
            "{source}"
        );
        assert!(
            !source.contains("\nfn greet()"),
            "no free function:\n{source}"
        );
        assert!(
            designer
                .status
                .starts_with("added `fn greet` to `impl App`"),
            "{}",
            designer.status
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A handlers type whose impl is not in the file still gets a handler —
    /// a free function — and the status line says why it is not a method.
    #[cfg(unix)]
    #[test]
    fn a_handlers_type_with_no_impl_in_the_file_falls_back_and_says_so() {
        let (dir, mut designer) = code_behind(
            "noimpl",
            None,
            "fn main() {}\n",
            "code = src/app.rs\nhandlers = App\n",
        );
        designer.settings.editor = String::from("true {file}");
        assert!(designer.select_named("who"));
        designer.open_code(row(&designer, "on-submit"));
        let source = std::fs::read_to_string(dir.join("src").join("app.rs")).unwrap();
        assert!(
            source.ends_with("fn greet() {\n    todo!(\"greet\")\n}\n"),
            "{source}"
        );
        assert!(
            designer.status.starts_with("no `impl App` in "),
            "{}",
            designer.status
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The code's vocabulary reaches the inspector: an event the code answers
    /// is left alone, one it does not is marked, and the tooltip lists what
    /// the code does answer.
    #[test]
    fn an_event_the_code_does_not_answer_is_marked_and_the_tooltip_says_what_it_would() {
        let code = "impl App {\n    fn greet(&mut self) {}\n    fn save(&mut self) {}\n}\n";
        let (dir, mut designer) =
            code_behind("vocab", None, code, "code = src/app.rs\nhandlers = App\n");
        assert!(designer.select_named("who"));
        let fields = designer.fields(&designer.selection.clone(), &[designer.selected.unwrap()]);
        let submit = fields
            .iter()
            .find(|f| f.property.name == "on-submit")
            .unwrap();
        assert_eq!(submit.answered, Some(true), "`greet` is answered");
        assert!(
            fields
                .iter()
                .filter(|f| !is_event(f.property))
                .all(|f| f.answered.is_none())
        );
        let name = designer.inspector.as_ref().unwrap().rows[row(&designer, "on-submit")]
            .name
            .unwrap();
        let tooltip = designer.ui.tooltip(name).unwrap_or_default();
        assert!(tooltip.contains("Answered by "), "{tooltip}");
        assert!(tooltip.contains("greet, save"), "{tooltip}");
        std::fs::remove_dir_all(&dir).unwrap();

        // The same form asking for a name the code has never heard of.
        let unanswered = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../forms/hello.dform"
        ))
        .unwrap()
        .replace("on-submit=greet", "on-submit=nope");
        let (dir, mut designer) = code_behind(
            "vocab2",
            Some(&unanswered),
            code,
            "code = src/app.rs\nhandlers = App\n",
        );
        assert!(designer.select_named("who"));
        let fields = designer.fields(&designer.selection.clone(), &[designer.selected.unwrap()]);
        let submit = fields
            .iter()
            .find(|f| f.property.name == "on-submit")
            .unwrap();
        assert_eq!(
            submit.answered,
            Some(false),
            "`nope` is the load error, early"
        );
        std::fs::remove_dir_all(&dir).unwrap();

        // And with no code to ask, nothing is judged.
        let mut plain = designer_on("forms/hello.dform");
        assert!(plain.select_named("who"));
        let fields = plain.fields(&plain.selection.clone(), &[plain.selected.unwrap()]);
        assert!(fields.iter().all(|f| f.answered.is_none()));
    }

    /// In glyphs mode the tiles wrap at the column count and never leave the
    /// pane, whatever the filter has done to the shelves.
    #[test]
    fn glyph_tiles_stay_inside_the_pane_and_under_their_headings() {
        let mut designer = designer_on("forms/hello.dform");
        designer.cycle_palette_mode();
        designer.cycle_palette_mode();
        let width = designer.scale.n(designer.settings().left - GAP * 2);
        for (row, slot) in designer.shown.iter().zip(&designer.slots) {
            assert!(
                slot.x >= 0 && slot.right() <= width,
                "{slot:?} leaves the pane"
            );
            if matches!(row, Shelf::Heading(_)) {
                assert_eq!(slot.width, width, "a heading shares its row");
            }
        }
        // No two slots overlap: a press can only mean one thing.
        for (i, a) in designer.slots.iter().enumerate() {
            for b in &designer.slots[i + 1..] {
                assert!(!a.intersects(b), "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn the_filter_narrows_the_palette_and_giving_it_up_puts_it_back() {
        let mut designer = designer_on("forms/hello.dform");
        let all = offered(&designer).len();
        assert_eq!(all, denise_ui::widgets::all().len());

        set_filter(&mut designer, "prog");
        assert_eq!(offered(&designer), vec!["progress", "radial-progress"]);

        set_filter(&mut designer, "nothing called this");
        assert!(designer.shown.is_empty());

        set_filter(&mut designer, "");
        assert_eq!(offered(&designer).len(), all);
    }

    #[test]
    fn the_palette_is_shelves_rather_than_one_flat_list() {
        let designer = designer_on("forms/hello.dform");

        // A heading for every group, each with something under it, in the
        // order `Group::ALL` gives — which is the catalogue's order and not
        // this file's.
        let headings: Vec<Group> = designer
            .shown
            .iter()
            .filter_map(|row| match row {
                Shelf::Heading(group) => Some(*group),
                Shelf::Widget(_) => None,
            })
            .collect();
        assert_eq!(headings, Group::ALL.to_vec());

        // The first row is a heading, never a widget adrift above one.
        assert!(matches!(designer.shown.first(), Some(Shelf::Heading(_))));

        // And every widget the toolkit ships is on exactly one shelf.
        let mut kinds = offered(&designer);
        assert_eq!(kinds.len(), denise_ui::widgets::all().len());
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), denise_ui::widgets::all().len());
    }

    #[test]
    fn a_shelf_the_filter_empties_takes_its_heading_with_it() {
        let mut designer = designer_on("forms/hello.dform");
        set_filter(&mut designer, "prog");

        // `progress` and `radial-progress` are both indicators, so exactly one
        // heading survives — a heading over nothing is worse than no heading.
        let headings: Vec<Group> = designer
            .shown
            .iter()
            .filter_map(|row| match row {
                Shelf::Heading(group) => Some(*group),
                Shelf::Widget(_) => None,
            })
            .collect();
        assert_eq!(headings, vec![Group::Indicator]);
        assert_eq!(designer.shown.len(), 3, "a heading and its two widgets");
    }

    #[test]
    fn the_filter_searches_what_a_widget_is_and_not_only_what_it_is_called() {
        let mut designer = designer_on("forms/hello.dform");

        // Nothing is called `dropdown`; `select` is one, and says so.
        set_filter(&mut designer, "dropdown");
        assert_eq!(offered(&designer), vec!["select"]);

        // Nor `switch`, which is what a `toggle` looks like. `tabs` comes too,
        // because it is for *switching* what is below — which is the search
        // working rather than failing: both are reasonable answers, and neither
        // is reachable by spelling.
        set_filter(&mut designer, "switch");
        let found = offered(&designer);
        assert!(found.contains(&"toggle"), "{found:?}");
        assert!(
            !found.iter().any(|kind| kind.contains("switch")),
            "one of these is called `switch` after all: {found:?}",
        );
    }

    #[test]
    fn a_row_under_the_pointer_says_what_the_widget_is() {
        let mut designer = designer_on("forms/hello.dform");
        let at = palette_point(&mut designer, "toggle");

        designer.input(&[InputEvent::PointerMoved { position: at }]);
        assert_eq!(
            designer.ui.tooltip(designer.chrome.palette),
            Some(
                denise_ui::widgets::all()
                    .iter()
                    .find(|w| w.kind == "toggle")
                    .expect("a toggle")
                    .doc
            ),
            "the palette says nothing about the row under the pointer",
        );

        // A heading has nothing to say, and must not leave the last row's line
        // hanging over it.
        let heading = (0..designer.shown.len())
            .find(|row| matches!(designer.shown[*row], Shelf::Heading(_)))
            .expect("a heading");
        let view = designer
            .ui
            .bounds(designer.chrome.palette_view)
            .expect("a palette");
        designer.input(&[InputEvent::PointerMoved {
            position: Point::new(
                view.x + view.width / 2,
                view.y + heading as i32 * PALETTE_ROW + PALETTE_ROW / 2,
            ),
        }]);
        assert_eq!(designer.ui.tooltip(designer.chrome.palette), None);
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

    /// The row for a property, when the pane has one at all.
    fn showing_row(designer: &Designer, name: &str) -> Option<usize> {
        designer
            .inspector
            .as_ref()
            .and_then(|pane| pane.rows.iter().position(|row| row.property.name == name))
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
    fn nothing_selected_is_the_form_rather_than_an_empty_pane() {
        let mut designer = designer_on("forms/reference.dform");
        select(&mut designer, "volume");
        assert!(
            showing_row(&designer, "role").is_some(),
            "a slider has a role"
        );

        press_key(&mut designer, KeyCode::Escape, false);
        // The form node is on neither the canvas nor the outline, so this pane
        // is the only way to reach what it says about itself.
        assert_eq!(showing(&designer, "title"), "Reference");
        assert_eq!(showing(&designer, "kind"), "screen");
        assert_eq!(showing(&designer, "width"), "1024");
        assert_eq!(showing(&designer, "background"), "base-200");
        assert!(
            showing_row(&designer, "role").is_none(),
            "a form has no `role`"
        );
        // And polling it is not a crash.
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
        assert_eq!(
            canvas.y,
            TOOLBAR + ARRANGE,
            "the toolbar and the arrange bar are not both docked"
        );
        assert_eq!(
            canvas.width,
            WINDOW.width as i32 - settings.left - settings.right
        );
        assert_eq!(
            canvas.height,
            WINDOW.height as i32 - TOOLBAR - ARRANGE - STATUS
        );
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
        assert_eq!(canvas.height, 1000 - TOOLBAR - ARRANGE - STATUS);

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
        // a twenty-seventh widget appears here without the designer changing.
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
        assert!(designer.select_named("volume"));

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
        assert!(designer.select_named("notify"));
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
        assert!(designer.making(), "New did not ask what to make");
        designer.handle(Message::Create);
        assert_eq!(
            designer.outline_names().count(),
            0,
            "a new form names nothing"
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

    /// Every file in the awkward corpus, by path.
    ///
    /// `denise-forms/tests/awkward/` — see its README. Walked rather than
    /// listed, so defending a new way of writing a form by hand is adding a
    /// file and nothing else.
    fn awkward() -> Vec<std::path::PathBuf> {
        let mut found: Vec<std::path::PathBuf> =
            std::fs::read_dir(repo("denise-forms/tests/awkward"))
                .expect("the corpus directory is there")
                .filter_map(|entry| {
                    let path = entry.ok()?.path();
                    (path.extension()? == "dform").then_some(path)
                })
                .collect();
        found.sort();
        assert!(found.len() >= 6, "the corpus went missing: {found:?}");
        found
    }

    #[test]
    fn every_awkward_form_opens_and_saves_without_changing_a_byte() {
        // #88's own words: a corpus of hand-written forms with deliberately odd
        // formatting round-trips byte-for-byte through the designer's
        // load-and-save path, headlessly. Not `Form::parse` to `Form::text` —
        // that is asserted next door in `denise-forms/tests/awkward.rs`. This is
        // `Document::open` to `Document::save`, through a real file, which is
        // the path a person's form actually takes: the temporary file, the
        // rename, and the designer having built the whole thing into a tree in
        // between.
        for path in awkward() {
            let source = std::fs::read(&path).expect("readable");
            let name = path
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .to_string();
            let out =
                std::env::temp_dir().join(format!("denise-awkward-{}-{name}", std::process::id()));
            let _ = std::fs::remove_file(&out);

            let document = Document::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut designer = Designer::new(WINDOW, 1.0, Settings::default(), document);
            // Built, drawn and inspected — everything short of an edit — because
            // a round trip that only holds while nothing has looked at the form
            // is not the one #88 is asking for.
            assert!(!designer.placed.is_empty(), "{name} built nothing");
            designer.select_named("who");
            designer.document.save(Some(out.clone())).expect("saving");

            assert_eq!(
                std::fs::read(&out).expect("reading back"),
                source,
                "{name} came back different from how it went in",
            );
            let _ = std::fs::remove_file(&out);
        }
    }

    #[test]
    fn nudging_a_node_in_an_awkward_form_is_a_one_line_diff() {
        // The second half of #88's "done when", through the canvas rather than
        // through `Edit`: pick a node, press the arrow key, and exactly the line
        // that node is written on is the line that changed.
        for path in awkward() {
            let name = path
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .to_string();
            let document = Document::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut designer = Designer::new(WINDOW, 1.0, Settings::default(), document);
            let before = text(&designer);

            // The first node in the file, whatever it is.
            let first = designer.placed.first().expect("a node").path.clone();
            let was: i32 = designer
                .document
                .form()
                .property(&first, "y")
                .expect("every node in the corpus is placed")
                .parse()
                .expect("a whole number");
            designer.selection = vec![first.clone()];
            designer.selected = designer.node_id(&first);
            designer.reselected();
            designer.nudge(0, 8);

            let changed = diff(&before, &text(&designer));
            assert_eq!(
                changed.len(),
                1,
                "{name} nudged one node and changed {} lines: {changed:#?}",
                changed.len(),
            );
            // And the same line, with the number changed and nothing else about
            // it moved — every other property still there, still in the order
            // the file wrote them, with whatever comment was on the end.
            let was_line = before
                .lines()
                .find(|line| line.contains(&format!("y={was}")))
                .expect("the line it was on");
            assert_eq!(
                squeezed(&changed[0]),
                squeezed(was_line).replace(&format!("y={was}"), &format!("y={}", was + 8)),
                "{name}: the line came back rewritten rather than edited",
            );
        }
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

    // ------------------------------------------------- the band and the tree

    /// A point on the stage, from a position in the form's own coordinates.
    fn on_stage(designer: &Designer, x: i32, y: i32) -> Point {
        let stage = designer.ui.bounds(designer.chrome.stage).expect("a stage");
        Point::new(stage.x + x, stage.y + y)
    }

    /// Presses at `from`, drags to `to`, lets go — with shift held or not.
    fn sweep(designer: &mut Designer, from: Point, to: Point, shift: bool) {
        let modifiers = if shift {
            denise::Modifiers::SHIFT
        } else {
            denise::Modifiers::NONE
        };
        feed(
            designer,
            &[
                InputEvent::PointerMoved { position: from },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Down,
                    position: from,
                    modifiers,
                },
                InputEvent::PointerMoved { position: to },
                InputEvent::PointerButton {
                    button: PointerButton::Left,
                    state: ElementState::Up,
                    position: to,
                    modifiers,
                },
            ],
        );
    }

    /// What is selected, by name, in the order it was selected.
    fn selected_names(designer: &Designer) -> Vec<String> {
        designer
            .selection
            .iter()
            .map(|path| {
                designer
                    .placed
                    .iter()
                    .find(|node| node.path == *path)
                    .and_then(|node| node.name.clone())
                    .unwrap_or_else(|| String::from("?"))
            })
            .collect()
    }

    #[test]
    fn a_band_over_the_canvas_takes_everything_it_wholly_encloses() {
        let mut designer = two_panels();
        let before = text(&designer);

        // From the empty bottom-right corner up past everything.
        let (from, to) = (on_stage(&designer, 295, 195), on_stage(&designer, 2, 2));
        sweep(&mut designer, from, to, false);

        let mut names = selected_names(&designer);
        names.sort();
        // The three top-level nodes — and not `inside`, which belongs to the
        // panel rather than to the form.
        assert_eq!(names, vec!["left", "loose", "right"]);
        assert_eq!(designer.status, "3 selected");
        assert_eq!(text(&designer), before, "a band must not touch the file");
        assert_eq!(designer.history.depth().0, 0);
    }

    #[test]
    fn a_band_that_only_brushes_a_node_does_not_take_it() {
        let mut designer = two_panels();
        // Across the right panel's top-left corner, enclosing none of it.
        let (from, to) = (on_stage(&designer, 100, 10), on_stage(&designer, 200, 100));
        sweep(&mut designer, from, to, false);
        assert!(
            designer.selection.is_empty(),
            "took something it only brushed: {:?}",
            selected_names(&designer)
        );
    }

    #[test]
    fn a_band_inside_a_panel_takes_the_panels_children_and_not_the_panel() {
        let mut designer = two_panels();
        // The first press takes hold of the panel.
        let background = on_stage(&designer, 100, 140);
        click_at(&mut designer, background, false);
        assert_eq!(selected_names(&designer), vec!["left"]);

        // The second, over the same background, is a band across its children.
        let corner = on_stage(&designer, 2, 28);
        sweep(&mut designer, background, corner, false);
        assert_eq!(selected_names(&designer), vec!["inside"]);
    }

    #[test]
    fn a_band_that_never_travelled_leaves_the_selection_where_it_was() {
        let mut designer = two_panels();
        let background = on_stage(&designer, 100, 140);
        click_at(&mut designer, background, false);
        click_at(&mut designer, background, false);
        assert_eq!(
            selected_names(&designer),
            vec!["left"],
            "a press that went nowhere gave up the panel"
        );
    }

    #[test]
    fn an_empty_band_inside_a_panel_leaves_the_panel_held() {
        let mut designer = two_panels();
        let background = on_stage(&designer, 100, 140);
        click_at(&mut designer, background, false);
        // A band over a corner of the panel with nothing in it.
        let corner = on_stage(&designer, 120, 100);
        sweep(&mut designer, background, corner, false);
        assert_eq!(selected_names(&designer), vec!["left"]);
    }

    #[test]
    fn shift_adds_what_a_band_takes_to_what_was_already_held() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        let (from, to) = (on_stage(&designer, 270, 20), on_stage(&designer, 135, 160));
        sweep(&mut designer, from, to, true);
        assert_eq!(selected_names(&designer), vec!["loose", "right"]);
    }

    #[test]
    fn dragging_a_node_onto_a_panel_makes_it_a_child_and_leaves_it_looking_still() {
        let mut designer = two_panels();
        designer.toggle_snapping();
        let loose = path_named(&designer, "loose");
        let before = text(&designer);
        let was = designer.path_bounds(&loose).expect("laid out");

        let from = middle(&designer, &loose);
        let to = on_stage(&designer, 200, 90);
        drag_from_to(&mut designer, from, to);

        let after = text(&designer);
        // In the panel, and the panel grew the braces it did not have.
        assert!(
            after.contains("panel name=right x=140 y=30 w=120 h=120 {\n        label \"loose\""),
            "{after}"
        );
        // The numbers changed because the space they are in changed: the node
        // is where it was on the screen, and its rectangle says something else.
        assert!(after.contains("x=20 y=50"), "{after}");
        let now = designer
            .path_bounds(&path_named(&designer, "loose"))
            .expect("laid out");
        assert_eq!(
            (now.x - was.x, now.y - was.y),
            (to.x - from.x, to.y - from.y),
            "it did not end where the pointer left it"
        );

        denise_forms::Form::parse(&after).expect("still a form");
        assert_eq!(designer.selection, vec![path_named(&designer, "loose")]);
        assert_eq!(designer.history.depth().0, 1, "a reparent is one step");
        designer.undo();
        assert_eq!(
            text(&designer),
            before,
            "undoing the reparent was not exact"
        );
    }

    #[test]
    fn dragging_a_node_off_a_panel_puts_it_on_the_form() {
        let mut designer = two_panels();
        designer.toggle_snapping();
        let inside = path_named(&designer, "inside");
        let before = text(&designer);

        let from = middle(&designer, &inside);
        let to = on_stage(&designer, 200, 180);
        drag_from_to(&mut designer, from, to);

        let after = text(&designer);
        // Out of the panel, which is left without children and so without a
        // block to hold them.
        assert!(
            after.contains("panel name=left x=4 y=30 w=120 h=120\n"),
            "{after}"
        );
        assert!(after.contains("x=160 y=170"), "{after}");
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "0:panel right",
                "0:label inside"
            ]
        );
        denise_forms::Form::parse(&after).expect("still a form");
        designer.undo();
        assert_eq!(
            text(&designer),
            before,
            "undoing the reparent was not exact"
        );
    }

    #[test]
    fn a_drop_on_something_that_cannot_hold_children_lands_in_what_holds_it() {
        let mut designer = two_panels();
        let inside = path_named(&designer, "inside");
        let onto = middle(&designer, &inside);

        let loose = path_named(&designer, "loose");
        let from = middle(&designer, &loose);
        drag_from_to(&mut designer, from, onto);

        // A label cannot hold anything, so the panel behind it took the drop.
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:panel left",
                "1:label inside",
                "1:label loose",
                "0:panel right"
            ]
        );
    }

    #[test]
    fn a_panel_dropped_into_another_takes_its_children_with_it() {
        let mut designer = two_panels();
        let before = text(&designer);
        let background = on_stage(&designer, 100, 140);
        let onto = on_stage(&designer, 200, 90);
        drag_from_to(&mut designer, background, onto);

        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel right",
                "1:panel left",
                "2:label inside"
            ]
        );
        denise_forms::Form::parse(&text(&designer)).expect("still a form");
        designer.undo();
        assert_eq!(
            text(&designer),
            before,
            "undoing the reparent was not exact"
        );
    }

    #[test]
    fn a_panel_dragged_over_its_own_background_is_moved_and_not_swallowed() {
        let mut designer = two_panels();
        designer.toggle_snapping();
        let before = text(&designer);
        let background = on_stage(&designer, 100, 140);
        let onto = on_stage(&designer, 110, 150);
        drag_from_to(&mut designer, background, onto);

        // A node cannot be dropped into itself, and the pointer never left it,
        // so this is the plain move it looks like.
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "1:label inside",
                "0:panel right"
            ]
        );
        assert_eq!(
            diff(&before, &text(&designer)),
            vec!["    panel name=left x=14 y=40 w=120 h=120 {"],
            "a plain move is one line: {}",
            text(&designer)
        );
    }

    // ------------------------------------------------------------- z-order

    #[test]
    fn bring_to_front_and_send_to_back_reorder_the_file() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        let before = text(&designer);

        press_key(&mut designer, KeyCode::PageUp, false);
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:panel left",
                "1:label inside",
                "0:panel right",
                "0:label loose"
            ],
            "it did not go to the end of its siblings"
        );
        assert_eq!(designer.status, "moved to the front");
        // Still what is selected, which is what makes a second press mean
        // something.
        assert_eq!(selected_names(&designer), vec!["loose"]);
        // And no `z` was written: file order is the stacking.
        assert!(!text(&designer).contains("z="), "{}", text(&designer));

        press_key(&mut designer, KeyCode::PageDown, false);
        assert_eq!(
            text(&designer),
            before,
            "back to the front is back to where it was"
        );
        assert_eq!(designer.status, "moved to the back");
    }

    #[test]
    fn bringing_the_front_one_further_forward_writes_nothing() {
        let mut designer = two_panels();
        select(&mut designer, "right");
        let before = text(&designer);
        press_key(&mut designer, KeyCode::PageUp, false);
        assert_eq!(designer.status, "already at the front");
        assert_eq!(text(&designer), before);
        assert_eq!(designer.history.depth().0, 0);
    }

    #[test]
    fn reordering_a_form_that_sets_z_says_that_z_is_what_decides() {
        let source = concat!(
            "form \"Z\" version=1 width=200 height=120 {\n",
            "    label \"a\" name=a x=4 y=4 w=40 h=20\n",
            "    label \"b\" name=b x=4 y=30 w=40 h=20 z=5\n",
            "}\n",
        );
        let mut designer = scratch("z", source);

        select(&mut designer, "a");
        press_key(&mut designer, KeyCode::PageUp, false);
        assert!(
            designer.status.contains("`z` is what decides"),
            "{}",
            designer.status
        );
        // It moved in the file all the same: the file is the thing being edited.
        assert_eq!(shown_rows(&designer), vec!["0:label b", "0:label a"]);
    }

    #[test]
    fn undoing_a_reorder_puts_the_file_back_byte_for_byte() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        let before = text(&designer);
        press_key(&mut designer, KeyCode::PageUp, false);
        assert_ne!(text(&designer), before);
        designer.undo();
        assert_eq!(text(&designer), before);
    }

    // ------------------------------------------------------- more than one

    /// Three labels of different sizes and a panel to drop them in. The same
    /// three rectangles `arrange`'s own tests use.
    fn three() -> Designer {
        scratch(
            "three",
            concat!(
                "form \"Three\" version=1 width=400 height=300 {\n",
                "    label \"a\" name=a x=10 y=10 w=40 h=20\n",
                "    label \"b\" name=b x=100 y=50 w=60 h=40\n",
                "    label \"c\" name=c x=60 y=90 w=20 h=10\n",
                "    panel name=box x=200 y=150 w=150 h=120\n",
                "}\n",
            ),
        )
    }

    /// Clicks each of them in turn, holding shift after the first — so the last
    /// named is the one wearing the handles.
    fn pick(designer: &mut Designer, names: &[&str]) {
        // From nothing: a plain click on something already held keeps the whole
        // selection — which is what lets a group be dragged — so starting over
        // has to actually start over.
        press_key(designer, KeyCode::Escape, false);
        for (nth, name) in names.iter().enumerate() {
            let path = path_named(designer, name);
            let at = middle(designer, &path);
            click_at(designer, at, nth > 0);
        }
        assert_eq!(
            designer.selection.len(),
            names.len(),
            "picking {names:?} gave {:?}",
            selected_names(designer)
        );
    }

    /// The rectangle the file gives a node, by name.
    fn rect_of(designer: &Designer, name: &str) -> Rect {
        let path = path_named(designer, name);
        let number = |what: &str| {
            designer
                .document
                .form()
                .property(&path, what)
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or_default()
        };
        Rect::new(number("x"), number("y"), number("w"), number("h"))
    }

    #[test]
    fn aligning_moves_the_others_onto_the_one_wearing_the_handles() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        let before = text(&designer);
        designer.arrange(Command::Left);

        // `c` was picked last, so `c` is the anchor and `c` does not move.
        assert_eq!(rect_of(&designer, "c"), Rect::new(60, 90, 20, 10));
        assert_eq!(rect_of(&designer, "a"), Rect::new(60, 10, 40, 20));
        assert_eq!(rect_of(&designer, "b"), Rect::new(60, 50, 60, 40));
        assert!(
            designer.status.starts_with("aligned left"),
            "{}",
            designer.status
        );

        // One gesture, one step.
        assert_eq!(designer.history.depth().0, 1);
        designer.undo();
        assert_eq!(
            text(&designer),
            before,
            "undoing the alignment was not exact"
        );
    }

    #[test]
    fn the_same_size_is_the_anchors_and_nothing_moves() {
        let mut designer = three();
        pick(&mut designer, &["a", "c", "b"]);
        designer.arrange(Command::SameSize);
        for name in ["a", "b", "c"] {
            let rect = rect_of(&designer, name);
            assert_eq!((rect.width, rect.height), (60, 40), "{name}");
        }
        assert_eq!(rect_of(&designer, "a").x, 10, "it moved as well as resized");
    }

    #[test]
    fn spacing_evenly_writes_the_one_in_the_middle_and_leaves_the_ends() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        let before = text(&designer);
        designer.arrange(Command::SpaceDown);

        // Top edges 10, 50, 90 with heights 20, 40, 10: 90 of span, 70 of it
        // occupied, so 20 to share over two gaps.
        assert_eq!(rect_of(&designer, "a").y, 10, "the top one moved");
        assert_eq!(rect_of(&designer, "b").y, 40);
        assert_eq!(rect_of(&designer, "c").y, 90, "the bottom one moved");
        assert_eq!(
            diff(&before, &text(&designer)),
            vec!["    label \"b\" name=b x=100 y=40 w=60 h=40"],
            "only the one in the middle should have moved"
        );
    }

    #[test]
    fn a_command_that_would_change_nothing_says_so_and_writes_nothing() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        designer.arrange(Command::Left);
        let settled = text(&designer);
        let steps = designer.history.depth().0;
        designer.arrange(Command::Left);
        assert_eq!(text(&designer), settled);
        assert_eq!(designer.history.depth().0, steps, "it wrote a second step");
        assert_eq!(designer.status, "already aligned left");
    }

    #[test]
    fn a_command_needs_the_selection_it_says_it_needs() {
        let mut designer = three();
        // Nothing selected: none of them.
        for command in Command::ALL {
            assert!(!designer.can_arrange(command), "{command:?} with nothing");
        }
        // One: still nothing, and `box` is a panel with nothing in it.
        select(&mut designer, "box");
        for command in Command::ALL {
            assert!(!designer.can_arrange(command), "{command:?} with one");
        }
        // Two: everything but spacing and ungrouping.
        pick(&mut designer, &["a", "b"]);
        for command in Command::ALL {
            let wanted = !matches!(
                command,
                Command::SpaceAcross | Command::SpaceDown | Command::Ungroup
            );
            assert_eq!(
                designer.can_arrange(command),
                wanted,
                "{command:?} with two"
            );
        }
        // Three: everything but ungrouping.
        pick(&mut designer, &["a", "b", "c"]);
        for command in Command::ALL {
            let wanted = command != Command::Ungroup;
            assert_eq!(
                designer.can_arrange(command),
                wanted,
                "{command:?} with three"
            );
        }
    }

    #[test]
    fn nodes_in_different_panels_have_no_shared_space_to_be_lined_up_in() {
        let mut designer = two_panels();
        pick(&mut designer, &["loose", "inside"]);
        assert!(designer.siblings_selected().is_none());
        for command in Command::ALL {
            assert!(!designer.can_arrange(command), "{command:?}");
        }
        // And the button says what it wants instead of what it does.
        let (_, id) = designer
            .chrome
            .arrange_buttons
            .iter()
            .find(|(command, _)| *command == Command::Left)
            .copied()
            .expect("an align-left button");
        assert!(!designer.ui.enabled(id), "the button is still live");
        assert_eq!(
            designer.ui.tooltip(id),
            Some(Needs::Several.why()),
            "the tooltip does not say why"
        );
    }

    #[test]
    fn giving_a_command_that_cannot_be_given_says_what_it_wanted() {
        let mut designer = three();
        select(&mut designer, "a");
        let before = text(&designer);
        designer.arrange(Command::Left);
        assert_eq!(designer.status, Needs::Several.why());
        assert_eq!(text(&designer), before);
    }

    #[test]
    fn grouping_puts_them_in_a_new_panel_at_their_bounding_box() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        let before = text(&designer);
        designer.arrange(Command::Group);

        let after = text(&designer);
        // The bounding box of the three, and their rectangles translated into
        // it so that nothing appears to have moved.
        assert!(after.contains("panel x=10 y=10 w=150 h=90 {"), "{after}");
        assert_eq!(rect_of(&designer, "a"), Rect::new(0, 0, 40, 20));
        assert_eq!(rect_of(&designer, "b"), Rect::new(90, 40, 60, 40));
        assert_eq!(rect_of(&designer, "c"), Rect::new(50, 80, 20, 10));
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:panel box",
                "0:panel",
                "1:label a",
                "1:label b",
                "1:label c"
            ]
        );
        // The new panel is what is selected, which is what makes the next thing
        // you do be to it.
        assert_eq!(designer.selection, vec![vec![1]]);
        denise_forms::Form::parse(&after).expect("still a form");

        assert_eq!(designer.history.depth().0, 1, "grouping is one step");
        designer.undo();
        assert_eq!(text(&designer), before, "undoing the group was not exact");
    }

    #[test]
    fn grouping_does_not_move_anything_on_the_screen() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        let was: Vec<Rect> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                let path = path_named(&designer, name);
                designer.path_bounds(&path).expect("laid out")
            })
            .collect();

        designer.arrange(Command::Group);

        for (name, before) in ["a", "b", "c"].iter().zip(&was) {
            let path = path_named(&designer, name);
            let now = designer.path_bounds(&path).expect("laid out");
            assert_eq!(now, *before, "`{name}` moved on the screen");
        }
    }

    #[test]
    fn ungrouping_takes_them_out_and_the_panel_away() {
        let mut designer = two_panels();
        select(&mut designer, "left");
        let before = text(&designer);
        designer.arrange(Command::Ungroup);

        let after = text(&designer);
        assert!(
            !after.contains("name=left"),
            "the panel is still there: {after}"
        );
        // Out of the panel and into the panel's own space: it was at 4,30 and
        // the label was at 4,4 inside it.
        assert_eq!(rect_of(&designer, "inside"), Rect::new(8, 34, 80, 20));
        assert_eq!(
            shown_rows(&designer),
            vec!["0:label loose", "0:label inside", "0:panel right"]
        );
        assert_eq!(selected_names(&designer), vec!["inside"]);
        denise_forms::Form::parse(&after).expect("still a form");

        assert_eq!(designer.history.depth().0, 1, "ungrouping is one step");
        designer.undo();
        assert_eq!(text(&designer), before, "undoing the ungroup was not exact");
    }

    #[test]
    fn grouping_and_ungrouping_puts_every_rectangle_back_where_it_was() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        let was: Vec<Rect> = ["a", "b", "c"]
            .iter()
            .map(|name| rect_of(&designer, name))
            .collect();

        designer.arrange(Command::Group);
        designer.arrange(Command::Ungroup);

        for (name, before) in ["a", "b", "c"].iter().zip(&was) {
            assert_eq!(
                rect_of(&designer, name),
                *before,
                "`{name}` came back wrong"
            );
        }
        assert!(
            !text(&designer).contains("panel x=10"),
            "the panel survived: {}",
            text(&designer)
        );
    }

    #[test]
    fn dragging_one_of_several_takes_them_all_and_is_one_step() {
        let mut designer = three();
        designer.toggle_snapping();
        pick(&mut designer, &["a", "b", "c"]);
        let before = text(&designer);

        let path = path_named(&designer, "b");
        let from = middle(&designer, &path);
        drag_from_to(&mut designer, from, Point::new(from.x + 12, from.y + 30));

        assert_eq!(rect_of(&designer, "a"), Rect::new(22, 40, 40, 20));
        assert_eq!(rect_of(&designer, "b"), Rect::new(112, 80, 60, 40));
        assert_eq!(rect_of(&designer, "c"), Rect::new(72, 120, 20, 10));
        assert_eq!(designer.history.depth().0, 1, "one drag, one step");
        designer.undo();
        assert_eq!(text(&designer), before);
    }

    #[test]
    fn nudging_several_is_one_step_and_not_one_each() {
        let mut designer = three();
        pick(&mut designer, &["a", "b", "c"]);
        let before = text(&designer);
        press_key(&mut designer, KeyCode::ArrowRight, false);

        assert_eq!(rect_of(&designer, "a").x, 11);
        assert_eq!(rect_of(&designer, "b").x, 101);
        assert_eq!(rect_of(&designer, "c").x, 61);
        assert_eq!(designer.history.depth().0, 1);
        designer.undo();
        assert_eq!(text(&designer), before);
    }

    #[test]
    fn dropping_several_onto_a_panel_reparents_all_of_them() {
        let mut designer = three();
        designer.toggle_snapping();
        pick(&mut designer, &["a", "b"]);
        let before = text(&designer);

        // Onto the empty panel, which is at 200,150 and 150 by 120.
        let path = path_named(&designer, "b");
        let from = middle(&designer, &path);
        let onto = Point::new(from.x + 150, from.y + 150);
        drag_from_to(&mut designer, from, onto);

        assert_eq!(
            shown_rows(&designer),
            vec!["0:label c", "0:panel box", "1:label a", "1:label b"],
            "{}",
            text(&designer)
        );
        denise_forms::Form::parse(&text(&designer)).expect("still a form");
        assert_eq!(designer.history.depth().0, 1, "one drop, one step");
        designer.undo();
        assert_eq!(text(&designer), before, "undoing the drop was not exact");
    }

    // ------------------------------------------------------- the clipboard

    /// Ctrl-something, which is Cmd-something on a Mac and either here.
    fn press_command(designer: &mut Designer, code: KeyCode) {
        press_with(designer, code, denise::Modifiers::CTRL);
    }

    #[test]
    fn copying_a_panel_and_pasting_it_gains_the_whole_subtree_with_names_of_its_own() {
        let mut designer = two_panels();
        select(&mut designer, "left");
        let before = text(&designer);
        designer.copy();
        assert!(
            designer.status.starts_with("copied 1"),
            "{}",
            designer.status
        );

        // Nothing selected, so it lands on the form itself.
        press_key(&mut designer, KeyCode::Escape, false);
        designer.paste();

        let after = text(&designer);
        // Offset by twice the grid so the copy is not hidden behind what it
        // came from, and every name in it is one nobody had.
        assert!(
            after.contains(concat!(
                "    panel name=left2 x=12 y=38 w=120 h=120 {\n",
                "        label \"in-left\" name=inside2 x=4 y=4 w=80 h=20\n",
                "    }\n",
            )),
            "{after}"
        );
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "1:label inside",
                "0:panel right",
                "0:panel left2",
                "1:label inside2"
            ]
        );
        denise_forms::Form::parse(&after).expect("still a form");

        assert_eq!(designer.history.depth().0, 1, "a paste is one step");
        designer.undo();
        assert_eq!(text(&designer), before, "undoing the paste was not exact");
    }

    #[test]
    fn what_goes_on_the_clipboard_is_form_source_and_reads_as_form_source() {
        let mut designer = two_panels();
        select(&mut designer, "left");
        designer.copy();
        assert_eq!(
            designer.clipboard.take().as_deref(),
            Some(concat!(
                "panel name=left x=4 y=30 w=120 h=120 {\n",
                "    label \"in-left\" name=inside x=4 y=4 w=80 h=20\n",
                "}\n",
            )),
            "the clipboard is not holding the source somebody could read"
        );
    }

    #[test]
    fn form_source_somebody_typed_somewhere_else_pastes_in() {
        let mut designer = two_panels();
        designer
            .clipboard
            .put("button \"Go\" name=go x=10 y=10 w=60 h=24 on-press=go\n");
        press_key(&mut designer, KeyCode::Escape, false);
        designer.paste();

        assert!(
            text(&designer).contains("button \"Go\" name=go x=18 y=18 w=60 h=24 on-press=go"),
            "{}",
            text(&designer)
        );
        assert_eq!(designer.status, "pasted 1 node(s)");
    }

    #[test]
    fn nonsense_on_the_clipboard_is_reported_rather_than_pasted() {
        let mut designer = two_panels();
        let before = text(&designer);

        // A widget this engine does not have.
        designer.clipboard.put("banana x=0 y=0 w=10 h=10\n");
        designer.paste();
        assert!(designer.status.contains("banana"), "{}", designer.status);
        assert_eq!(text(&designer), before, "it pasted anyway");

        // A widget it does have, with a property it does not.
        designer
            .clipboard
            .put("label \"a\" x=0 y=0 w=10 h=10 flavour=salt\n");
        designer.paste();
        assert!(designer.status.contains("flavour"), "{}", designer.status);
        assert_eq!(text(&designer), before);

        // And not KDL at all.
        designer.clipboard.put("}}} nope\n");
        designer.paste();
        assert!(
            designer.status.starts_with("that is not form source"),
            "{}",
            designer.status
        );
        assert_eq!(text(&designer), before);
        assert_eq!(
            designer.history.depth().0,
            0,
            "a refused paste wrote a step"
        );
    }

    #[test]
    fn an_empty_clipboard_says_so_rather_than_doing_nothing() {
        let mut designer = two_panels();
        designer.paste();
        assert_eq!(designer.status, "there is nothing on the clipboard");
    }

    #[test]
    fn pasting_twice_gives_two_copies_and_not_two_clashes() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        designer.copy();
        press_key(&mut designer, KeyCode::Escape, false);
        designer.paste();
        press_key(&mut designer, KeyCode::Escape, false);
        designer.paste();

        let after = text(&designer);
        assert!(after.contains("name=loose2"), "{after}");
        assert!(after.contains("name=loose3"), "{after}");
        denise_forms::Form::parse(&after).expect("two of the same name would not load");
    }

    #[test]
    fn a_paste_lands_inside_a_selected_panel_and_beside_a_selected_label() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        designer.copy();

        // A panel is somewhere to put things.
        select(&mut designer, "right");
        designer.paste();
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "1:label inside",
                "0:panel right",
                "1:label loose2"
            ]
        );

        // A label is not, so its parent takes it.
        select(&mut designer, "inside");
        designer.paste();
        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "1:label inside",
                "1:label loose3",
                "0:panel right",
                "1:label loose2"
            ]
        );
    }

    #[test]
    fn cut_takes_it_out_and_puts_it_where_a_paste_can_find_it() {
        let mut designer = two_panels();
        select(&mut designer, "left");
        let before = text(&designer);
        designer.cut();

        assert!(
            !text(&designer).contains("name=left"),
            "{}",
            text(&designer)
        );
        assert_eq!(designer.status, "cut 1 node(s)");
        assert_eq!(designer.history.depth().0, 1, "a cut is one step");

        designer.paste();
        assert!(
            text(&designer).contains("name=left"),
            "it did not come back"
        );
        // The name is free again, so it comes back as itself.
        assert!(
            !text(&designer).contains("name=left2"),
            "{}",
            text(&designer)
        );

        designer.undo();
        designer.undo();
        assert_eq!(text(&designer), before, "undoing the cut was not exact");
    }

    #[test]
    fn duplicate_puts_another_one_beside_it_rather_than_inside_it() {
        let mut designer = two_panels();
        select(&mut designer, "left");
        designer.duplicate();

        assert_eq!(
            shown_rows(&designer),
            vec![
                "0:label loose",
                "0:panel left",
                "1:label inside",
                "0:panel right",
                "0:panel left2",
                "1:label inside2"
            ],
            "a duplicated panel went inside itself: {}",
            text(&designer)
        );
        assert_eq!(designer.status, "duplicated 1 node(s)");
        // And the copy is what is selected, which is what makes the next thing
        // you do be to it.
        assert_eq!(selected_names(&designer), vec!["left2"]);
    }

    #[test]
    fn several_selected_copy_and_paste_as_several() {
        let mut designer = three();
        pick(&mut designer, &["a", "c"]);
        designer.copy();
        press_key(&mut designer, KeyCode::Escape, false);
        designer.paste();

        let after = text(&designer);
        assert!(after.contains("name=a2"), "{after}");
        assert!(after.contains("name=c2"), "{after}");
        assert!(
            !after.contains("name=b2"),
            "it took one nobody picked: {after}"
        );
        assert_eq!(designer.status, "pasted 2 node(s)");
        assert_eq!(designer.history.depth().0, 1, "one paste, one step");
        assert_eq!(selected_names(&designer), vec!["a2", "c2"]);
    }

    #[test]
    fn the_keys_are_wired_to_the_same_things_the_methods_are() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        press_command(&mut designer, KeyCode::C);
        assert!(designer.status.starts_with("copied"), "{}", designer.status);

        press_key(&mut designer, KeyCode::Escape, false);
        press_command(&mut designer, KeyCode::V);
        assert!(text(&designer).contains("name=loose2"));

        select(&mut designer, "loose");
        press_command(&mut designer, KeyCode::D);
        assert!(
            designer.status.starts_with("duplicated"),
            "{}",
            designer.status
        );

        select(&mut designer, "loose");
        press_command(&mut designer, KeyCode::X);
        assert!(designer.status.starts_with("cut"), "{}", designer.status);
    }

    #[test]
    fn a_copy_with_nothing_selected_says_so_and_leaves_the_clipboard_alone() {
        let mut designer = two_panels();
        select(&mut designer, "loose");
        designer.copy();
        press_key(&mut designer, KeyCode::Escape, false);
        designer.copy();
        assert_eq!(designer.status, "nothing selected to copy");
        assert!(
            designer
                .clipboard
                .take()
                .is_some_and(|it| it.contains("loose")),
            "an empty copy wiped the clipboard"
        );
    }

    // --------------------------------------------------- the form's own kind

    #[test]
    fn a_new_form_asks_what_kind_it_is_and_writes_only_what_is_not_a_default() {
        let mut designer = designer_on("forms/reference.dform");
        designer.handle(Message::New);
        assert!(designer.making());

        // Every kind is offered, and picking one says what it is for.
        designer.handle(Message::NewKind(2));
        designer.handle(Message::NewSize(1));
        designer.handle(Message::Create);

        assert!(!designer.making(), "the sheet stayed up");
        assert_eq!(
            designer.document.form().kind(),
            denise_forms::FormKind::Dialog
        );
        assert_eq!(designer.document.form().size(), Size::new(1024, 600));
        assert_eq!(
            text(&designer),
            "form \"Untitled\" version=1 kind=dialog width=1024 height=600\n",
            "a new form should say nothing it does not have to"
        );
        // A form nobody has edited is not unsaved work.
        assert_eq!(designer.history.depth().0, 0);
    }

    #[test]
    fn a_new_drawer_says_how_far_it_comes_in_because_it_has_to() {
        let mut designer = designer_on("forms/hello.dform");
        designer.handle(Message::New);
        designer.handle(Message::NewKind(3));
        designer.handle(Message::NewSize(0));
        designer.handle(Message::Create);

        let form = designer.document.form();
        assert_eq!(form.kind(), denise_forms::FormKind::Drawer);
        // A third of the axis it comes in along, which is 800 for a drawer.
        assert_eq!(form.extent(), 800 / 3);
        denise_forms::Form::parse(&text(&designer)).expect("a drawer with no extent will not load");
    }

    #[test]
    fn giving_up_the_sheet_leaves_the_form_that_was_open() {
        let mut designer = designer_on("forms/reference.dform");
        let before = text(&designer);
        designer.handle(Message::New);
        designer.handle(Message::NewKind(1));
        designer.handle(Message::Never);

        assert!(!designer.making());
        assert_eq!(text(&designer), before, "it made one anyway");

        // And Escape is the other way out.
        designer.handle(Message::New);
        press_key(&mut designer, KeyCode::Escape, false);
        assert!(!designer.making());
        assert_eq!(text(&designer), before);
    }

    #[test]
    fn while_the_sheet_is_up_the_canvas_takes_none_of_the_presses() {
        let mut designer = designer_on("forms/reference.dform");
        let path = path_named(&designer, "volume");
        let at = middle(&designer, &path);

        designer.handle(Message::New);
        // A press where a node is: it belongs to the sheet over it, not to the
        // form behind it.
        let rest = designer.input(&[button(ElementState::Down, at)]);
        assert_eq!(
            rest.len(),
            1,
            "design mode took a press meant for the sheet"
        );
        assert!(
            designer.selection.is_empty(),
            "it selected something anyway"
        );
    }

    #[test]
    fn a_drawer_is_drawn_attached_to_the_side_of_the_screen_it_comes_in_over() {
        let designer = scratch(
            "drawer",
            "form \"D\" version=1 kind=drawer width=800 height=480 side=after extent=200\n",
        );
        let surface = designer
            .ui
            .bounds(designer.chrome.surface.expect("a screen to come in over"))
            .expect("laid out");
        let stage = designer.ui.bounds(designer.chrome.stage).expect("laid out");

        assert_eq!((surface.width, surface.height), (800, 480));
        // Against the right edge, the full height, as far in as it says.
        assert_eq!(stage.width, 200);
        assert_eq!(stage.height, 480);
        assert_eq!(stage.right(), surface.right());
        assert_eq!(stage.y, surface.y);
    }

    #[test]
    fn a_shelf_comes_in_from_the_bottom_unless_it_says_otherwise() {
        let designer = scratch(
            "shelf",
            "form \"S\" version=1 kind=shelf width=800 height=480 extent=120\n",
        );
        let surface = designer
            .ui
            .bounds(designer.chrome.surface.expect("a screen"))
            .expect("laid out");
        let stage = designer.ui.bounds(designer.chrome.stage).expect("laid out");
        assert_eq!(stage.width, 800);
        assert_eq!(stage.height, 120);
        assert_eq!(stage.bottom(), surface.bottom());
    }

    #[test]
    fn a_dialog_is_drawn_on_a_backdrop_and_a_screen_is_not() {
        let dialog = scratch(
            "dialog",
            "form \"A\" version=1 kind=dialog width=380 height=180\n",
        );
        let backdrop = dialog
            .ui
            .bounds(dialog.chrome.surface.expect("a backdrop"))
            .expect("laid out");
        let stage = dialog.ui.bounds(dialog.chrome.stage).expect("laid out");
        assert_eq!((stage.width, stage.height), (380, 180));
        assert!(
            backdrop.width > stage.width && backdrop.height > stage.height,
            "the backdrop does not reach past the dialog: {backdrop:?}"
        );

        // A screen is the whole surface, so there is nothing behind it.
        let screen = designer_on("forms/hello.dform");
        assert!(screen.chrome.surface.is_none());
    }

    #[test]
    fn the_forms_own_size_is_changed_from_the_pane_and_undone_from_it() {
        let mut designer = designer_on("forms/hello.dform");
        press_key(&mut designer, KeyCode::Escape, false);
        assert_eq!(showing(&designer, "width"), "460");

        let before = text(&designer);
        write(&mut designer, "width", "640");
        designer.settle();

        assert_eq!(designer.document.form().size(), Size::new(640, 260));
        let stage = designer.ui.bounds(designer.chrome.stage).expect("laid out");
        assert_eq!(stage.width, 640, "the canvas did not follow the file");

        designer.undo();
        assert_eq!(text(&designer), before, "undoing the resize was not exact");
    }

    #[test]
    fn changing_the_kind_changes_which_rows_the_pane_has() {
        let mut designer = designer_on("forms/hello.dform");
        press_key(&mut designer, KeyCode::Escape, false);
        assert!(
            showing_row(&designer, "extent").is_none(),
            "a screen has none"
        );
        assert!(showing_row(&designer, "resizable").is_none());

        // The dropdown writes through the same door a field does.
        designer.commit_form(
            denise_forms::form_property(denise_forms::FormKind::Screen, "kind").expect("kind"),
            "window",
            false,
        );
        assert_eq!(
            designer.document.form().kind(),
            denise_forms::FormKind::Window
        );
        assert!(
            showing_row(&designer, "resizable").is_some(),
            "a window has one"
        );
        assert_eq!(showing(&designer, "resizable"), "#true", "and its default");
        assert!(
            showing_row(&designer, "dim").is_none(),
            "that is a dialog's"
        );
    }

    #[test]
    fn every_form_property_the_schema_declares_has_a_value_in_the_pane() {
        // The one place descriptor names are matched against accessors by hand.
        // A property added to the format and forgotten here would show as empty
        // for ever; this is what says so instead.
        let designer = designer_on("forms/reference.dform");
        for kind in [
            denise_forms::FormKind::Screen,
            denise_forms::FormKind::Window,
            denise_forms::FormKind::Dialog,
            denise_forms::FormKind::Drawer,
            denise_forms::FormKind::Shelf,
            denise_forms::FormKind::Fragment,
        ] {
            for property in denise_forms::FORM_PROPERTIES
                .iter()
                .chain(denise_forms::kind_properties(kind))
            {
                // `name`, `min-width` and `min-height` are genuinely empty when
                // the file does not write them; the rest all resolve to
                // something.
                if matches!(property.name, "name" | "min-width" | "min-height") {
                    continue;
                }
                assert!(
                    !designer.form_value(property).is_empty(),
                    "`{}` has no value for a {kind:?}",
                    property.name
                );
            }
        }
    }

    #[test]
    fn the_forms_title_is_edited_in_the_pane_and_is_the_nodes_argument() {
        let mut designer = designer_on("forms/hello.dform");
        press_key(&mut designer, KeyCode::Escape, false);
        assert_eq!(showing(&designer, "title"), "Hello");

        write(&mut designer, "title", "Greeting");
        designer.settle();
        assert_eq!(designer.document.form().title(), "Greeting");
        assert!(
            text(&designer).contains("form \"Greeting\""),
            "{}",
            text(&designer)
        );
        // And it cannot be taken away: a form with no title is not a form.
        let title = designer
            .form_fields()
            .into_iter()
            .find(|field| field.property.name == "title")
            .expect("a title row");
        assert!(title.written);
        assert!(!title.resettable, "the title offered a reset");
    }

    // ----------------------------------------- the other editor (#100)

    /// A private copy of a form file, so a test may edit it the way a person's
    /// text editor would.
    fn copied(form: &str, tag: u32) -> std::path::PathBuf {
        let source = std::fs::read_to_string(repo(form)).expect("the form is there");
        let path = std::env::temp_dir().join(format!(
            "denise-designer-{}-{tag}.dform",
            std::process::id()
        ));
        std::fs::write(&path, source).expect("a temporary form");
        path
    }

    fn designer_on_file(path: &std::path::Path) -> Designer {
        let document = Document::open(path).expect("the form opens");
        Designer::new(WINDOW, 1.0, Settings::default(), document)
    }

    /// What a text editor does: read it, change one thing, write it back.
    fn edit_in_another_editor(path: &std::path::Path, from: &str, to: &str) {
        let source = std::fs::read_to_string(path).expect("the file is there");
        assert!(source.contains(from), "nothing to replace: `{from}`");
        std::fs::write(path, source.replace(from, to)).expect("the file is writable");
    }

    #[test]
    fn a_rectangle_moved_in_a_text_editor_moves_on_the_canvas() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));
        let who = path_named(&designer, "who");
        let before = designer.path_bounds(&who).expect("a rectangle");

        edit_in_another_editor(&path, "name=who x=20 y=82", "name=who x=120 y=82");
        designer.check_file();

        let after = designer.path_bounds(&who).expect("a rectangle");
        assert_eq!(after.x - before.x, 100, "the node did not move: {after:?}");
        // And the selection came back, by name rather than by position.
        assert_eq!(designer.selection, vec![path_named(&designer, "who")]);
        assert_eq!(designer.selected, designer.node_id(&who));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_selection_survives_a_node_being_inserted_above_it() {
        // The case a path cannot survive and a name can: everything after the
        // new node shifts by one, so a selection kept by position would come
        // back pointing at the wrong widget.
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("greeting"));
        let was = path_named(&designer, "greeting");

        edit_in_another_editor(
            &path,
            "        label \"Hello, Denise\"",
            "        label \"New\" x=0 y=0 w=10 h=10\n        label \"Hello, Denise\"",
        );
        designer.check_file();

        let now = path_named(&designer, "greeting");
        assert_ne!(now, was, "nothing shifted, so this proves nothing");
        assert_eq!(designer.selection, vec![now]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unsaved_work_makes_it_ask_instead_of_reloading() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));
        designer.nudge(0, 8);
        assert!(designer.history.is_dirty());
        let mine = text(&designer);

        edit_in_another_editor(&path, "name=who x=20", "name=who x=120");
        designer.check_file();

        assert!(designer.clashing(), "it reloaded over unsaved work");
        assert_eq!(text(&designer), mine, "the form changed under the question");
    }

    #[test]
    fn keeping_mine_leaves_the_file_alone_and_stops_asking() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));
        designer.nudge(0, 8);
        edit_in_another_editor(&path, "name=who x=20", "name=who x=120");
        designer.check_file();
        assert!(designer.clashing());

        let theirs = std::fs::read_to_string(&path).expect("the file is there");
        let mine = text(&designer);
        designer.keep_mine();

        assert!(!designer.clashing());
        assert_eq!(text(&designer), mine, "keeping mine changed the form");
        assert_eq!(
            std::fs::read_to_string(&path).ok().as_deref(),
            Some(theirs.as_str()),
            "keeping mine wrote to the file",
        );
        assert!(
            designer.history.is_dirty(),
            "the unsaved work stopped being unsaved"
        );

        // The question was answered, so it is not asked again on the next frame.
        designer.check_file();
        assert!(!designer.clashing(), "it asked twice about one change");

        // And saving now is what overwrites it — with the answer already given.
        designer.handle(Message::Save);
        assert_eq!(
            std::fs::read_to_string(&path).ok().as_deref(),
            Some(mine.as_str()),
        );
        // Which the designer must not then read back as somebody else's edit.
        designer.check_file();
        assert!(!designer.clashing(), "its own save came back as a conflict");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reloading_takes_the_file_and_drops_what_was_unsaved() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));
        designer.nudge(0, 8);
        edit_in_another_editor(&path, "name=who x=20", "name=who x=120");
        designer.check_file();
        assert!(designer.clashing());

        let theirs = std::fs::read_to_string(&path).expect("the file is there");
        designer.take_theirs();

        assert!(!designer.clashing());
        assert_eq!(text(&designer), theirs, "reload did not take the file");
        assert!(
            !designer.history.is_dirty(),
            "a freshly read file is not modified"
        );
        assert!(
            !designer.history.can_undo(),
            "the old history outlived the form"
        );
        assert_eq!(designer.selection, vec![path_named(&designer, "who")]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn escape_answers_the_question_the_way_that_loses_nothing() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));
        designer.nudge(0, 8);
        let mine = text(&designer);
        edit_in_another_editor(&path, "name=who x=20", "name=who x=120");
        designer.check_file();
        assert!(designer.clashing());

        designer.input(&[InputEvent::Key {
            code: KeyCode::Escape,
            state: ElementState::Down,
            repeat: false,
            modifiers: Default::default(),
        }]);

        assert!(!designer.clashing());
        assert_eq!(text(&designer), mine);
        assert!(!designer.exit_requested(), "Escape got past the sheet");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_caught_halfway_through_being_written_is_not_a_reload() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        let before = text(&designer);

        std::fs::write(
            &path,
            "form \"Hello\" version=1 width=460 height=260 {\n    lab",
        )
        .expect("a half-written file");
        designer.check_file();

        assert!(!designer.clashing(), "a broken file put a question up");
        assert_eq!(text(&designer), before, "a broken file replaced the form");
        assert!(
            designer.status.contains("does not parse"),
            "said nothing about it: {}",
            designer.status
        );

        // And the write that finishes it is noticed like any other.
        std::fs::write(&path, before.replace("name=who x=20", "name=who x=120"))
            .expect("the rest of it");
        designer.check_file();
        assert!(text(&designer).contains("name=who x=120"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_is_read_from_under_a_drag_in_flight() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));
        let before = text(&designer);

        // Mid-drag: the pointer is down and the drag is holding ids from the
        // tree it started in, which a reload would replace under it.
        designer.drag_selection(0, 12);
        assert!(designer.drag.is_some());
        edit_in_another_editor(&path, "name=who x=20", "name=who x=120");
        designer.check_file();
        assert_eq!(text(&designer), before, "the tree changed mid-drag");

        // And once the pointer is up, the same change is still there to be
        // read — and by then the drag is an unsaved edit, so it is asked about
        // rather than taken.
        designer.release();
        designer.check_file();
        assert!(designer.clashing(), "the change was lost with the drag");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_is_read_from_under_a_caret() {
        let path = copied("forms/hello.dform", line!());
        let mut designer = designer_on_file(&path);
        assert!(designer.select_named("who"));

        // The caret is in one of the inspector's fields, holding a value that
        // is not in the file yet. A reload replaces the inspector, and what is
        // half typed into it has nowhere to have been kept.
        let index = row(&designer, "x");
        let pane = designer.inspector.as_ref().expect("a pane");
        let Editor::Field(field) = pane.rows[index].editor else {
            panic!("`x` is not a field");
        };
        designer.ui.focus(Some(field));

        edit_in_another_editor(&path, "name=who x=20", "name=who x=120");
        designer.check_file();
        assert!(
            !text(&designer).contains("name=who x=120"),
            "read under the caret"
        );

        // And the moment the caret is elsewhere, the change is taken.
        designer.ui.focus(None);
        designer.check_file();
        assert!(
            text(&designer).contains("name=who x=120"),
            "{}",
            text(&designer)
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_designer_with_a_file_open_keeps_looking_at_it() {
        // A form with nothing animating in it: `hello.dform` focuses its field,
        // and a blinking caret is a wake of its own.
        let path =
            std::env::temp_dir().join(format!("denise-watched-{}.dform", std::process::id()));
        std::fs::write(
            &path,
            "form \"F\" version=1 width=200 height=120 {\n    label \"a\" x=0 y=0 w=9 h=9\n}\n",
        )
        .expect("a temporary form");
        let designer = designer_on_file(&path);
        assert_eq!(designer.next_frame_in(), Some(watch::EVERY));

        // A form nobody has saved has no file to watch, so nothing wakes the
        // loop for it.
        let blank = Designer::new(WINDOW, 1.0, Settings::default(), Document::blank());
        assert_eq!(blank.next_frame_in(), None);

        // And whatever animates still sets the pace, because it is sooner.
        let watched = designer_on_file(&repo("forms/hello.dform"));
        assert_eq!(watched.next_frame_in(), Some(Duration::from_millis(16)));

        let _ = std::fs::remove_file(&path);
    }

    // ------------------------------------------- the tab order (#98)

    /// The path of the first node of this kind, for the ones the form leaves
    /// unnamed.
    fn path_named_kind(designer: &Designer, kind: &str) -> Vec<usize> {
        designer
            .placed
            .iter()
            .find(|p| p.kind == kind)
            .unwrap_or_else(|| panic!("no `{kind}` in the form"))
            .path
            .clone()
    }

    /// The kinds of the form's tab stops, in the order Tab reaches them.
    fn stops(designer: &mut Designer) -> Vec<String> {
        designer
            .tab_stops()
            .iter()
            .map(|path| {
                let node = designer
                    .placed
                    .iter()
                    .find(|it| it.path == *path)
                    .expect("a stop is a placed node");
                node.name.clone().unwrap_or_else(|| node.kind.to_string())
            })
            .collect()
    }

    #[test]
    fn the_mode_numbers_the_stops_in_the_order_tab_reaches_them() {
        let mut designer = designer_on("forms/hello.dform");
        designer.toggle_tab_order();
        assert!(designer.ordering());

        // `hello.dform` has a field and a button, in that order, and its two
        // labels and its panel are not stops.
        assert_eq!(stops(&mut designer), ["who", "button"]);

        // One badge and one number per stop, drawn over the form.
        assert_eq!(designer.overlay.len(), 4, "a badge and a label each");
    }

    #[test]
    fn clicking_two_siblings_in_order_rewrites_the_file() {
        let mut designer = designer_on("forms/hello.dform");
        let before = text(&designer);
        designer.toggle_tab_order();
        assert_eq!(stops(&mut designer), ["who", "button"]);

        // Click the button first, then the field: the file is rewritten so the
        // button comes first, and Tab follows it.
        let button = path_named_kind(&designer, "button");
        let who = path_named(&designer, "who");
        let at = middle(&designer, &button);
        click_at(&mut designer, at, false);
        let at = middle(&designer, &who);
        click_at(&mut designer, at, false);

        assert_eq!(stops(&mut designer), ["button", "who"]);
        assert_ne!(text(&designer), before, "the file did not change");

        // And it is a *reordering*: every rectangle is where it was, because a
        // rectangle is its own and nothing here moved on the canvas.
        for name in ["who", "greeting", "card"] {
            let path = path_named(&designer, name);
            let rect = designer.document.form().property(&path, "x");
            assert!(rect.is_some(), "`{name}` lost its geometry");
        }
        // Not a claim about blank lines: a node carries its leading trivia
        // with it, so moving one that had a blank line above it moves that
        // blank line too, and where the blanks fall legitimately changes.
        // What must not change is the design, and undo puts even the trivia
        // back — `undoing_a_re_sequence_puts_the_file_back_exactly` is that.
        assert_eq!(
            text(&designer).matches("name=").count(),
            before.matches("name=").count(),
            "reordering lost or gained a node",
        );
    }

    #[test]
    fn undoing_a_re_sequence_puts_the_file_back_exactly() {
        let mut designer = designer_on("forms/hello.dform");
        let before = text(&designer);
        designer.toggle_tab_order();
        let button = path_named_kind(&designer, "button");
        let who = path_named(&designer, "who");
        let at = middle(&designer, &button);
        click_at(&mut designer, at, false);
        let at = middle(&designer, &who);
        click_at(&mut designer, at, false);
        assert_ne!(text(&designer), before);

        designer.undo();
        assert_eq!(text(&designer), before, "undo did not restore the file");
    }

    #[test]
    fn clicking_something_that_is_not_a_stop_says_so_rather_than_moving_it() {
        let mut designer = designer_on("forms/hello.dform");
        designer.toggle_tab_order();
        let before = text(&designer);

        // The card is a panel: drawn, and not a place Tab lands. Its lower
        // reaches, because its middle is over the field that *is* one.
        let card = path_named(&designer, "card");
        let bounds = designer.path_bounds(&card).expect("the card is placed");
        let at = Point::new(bounds.x + bounds.width / 2, bounds.bottom() - 8);
        click_at(&mut designer, at, false);

        assert!(
            designer.status.contains("not a tab stop"),
            "said nothing useful: {}",
            designer.status
        );
        assert_eq!(text(&designer), before, "it moved something anyway");
    }

    #[test]
    fn the_mode_is_the_sequence_and_leaves_the_selection_alone() {
        let mut designer = designer_on("forms/hello.dform");
        assert!(designer.select_named("who"));
        assert!(!designer.selection.is_empty());

        // Turning it on drops the selection: eight resize handles over the
        // numbers would be noise, and this mode is about the order.
        designer.toggle_tab_order();
        assert!(designer.selection.is_empty());

        designer.toggle_tab_order();
        assert!(!designer.ordering());
        assert!(designer.status.contains("designing"));
    }

    #[test]
    fn the_mode_and_preview_are_not_both_on() {
        // The numbers are about the file; preview is about the form running.
        let mut designer = designer_on("forms/hello.dform");
        designer.toggle_preview();
        assert!(designer.previewing());

        designer.toggle_tab_order();
        assert!(designer.ordering());
        assert!(!designer.previewing(), "the form was still running");
    }
}
