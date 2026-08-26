//! The same screen, laid out twice: once by hand, once by `denise-arrange`.
//!
//! ```text
//! cargo run -p arranged              # by the crate
//! cargo run -p arranged -- --by-hand # the arithmetic written out
//! cargo run -p arranged -- --snapshot out.ppm
//! ```
//!
//! The point of having both is that they land in **the same rectangles**, and a
//! test in this file asserts it. That is the whole claim `denise-arrange` makes:
//! it computes what an application would have computed, and calls the same
//! `Ui::set_layout` an application would have called. Nothing about the tree
//! knows which of these two ran.
//!
//! What differs is the code. The by-hand version has to know that the title is
//! as wide as its text, which means measuring it, which means asking the tree —
//! and then it has to subtract, add the gaps, and keep the running x. The
//! arranged version says *hug*, *flex*, *hug* and stops.
//!
//! Resize the window and both follow, because both are re-run on a resize. That
//! is the honest answer to "when does layout happen": when the application says
//! so. See `docs/arrange.md`.

use std::time::{Duration, Instant};

use denise::{DamageTracker, Frame, InputEvent, Rect, Size, theme};
use denise_arrange::{Arrange, Flow, Sizing};
use denise_ui::widgets::{Button, Label, List, Panel};
use denise_ui::{NodeId, Ui, Void};
use denise_winit::{DeniseApp, WindowConfig, run_with};

/// Space inside the window and between the pieces.
const PAD: i32 = 12;
const GAP: i32 = 8;
/// The toolbar's height, and the sidebar's width. Fixed, because they are.
const BAR: i32 = 44;
const SIDE: i32 = 160;

/// Which way the rectangles were computed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum How {
    ByHand,
    Arranged,
}

/// The screen: a toolbar over a sidebar beside a body.
struct Screen {
    ui: Ui<Void>,
    how: How,
    bar: NodeId,
    title: NodeId,
    spacer: NodeId,
    save: NodeId,
    below: NodeId,
    sidebar: NodeId,
    body: NodeId,
    body_text: NodeId,
    started: Instant,
    exit: bool,
}

impl Screen {
    fn new(size: Size, how: How) -> Self {
        let mut ui: Ui<Void> = Ui::new(size, theme::DARK);
        let root = ui.root();

        // Building the tree is the same either way — the shape of a screen is
        // not a layout question. Only the rectangles differ, and every one of
        // these is `Rect::ZERO` until something computes it.
        let bar = ui
            .add(root, Panel::filled(denise::Role::Base200), Rect::ZERO)
            .expect("a root takes children");
        let title = ui
            .add(bar, Label::new("Settings"), Rect::ZERO)
            .expect("in the bar");
        let spacer = ui
            .add(bar, Panel::default(), Rect::ZERO)
            .expect("in the bar");
        let save = ui
            .add(bar, Button::<Void>::inert("Save"), Rect::ZERO)
            .expect("in the bar");

        let below = ui
            .add(root, Panel::default(), Rect::ZERO)
            .expect("a root child");
        let sidebar = ui
            .add(
                below,
                List::<Void>::inert(["Nettverk", "Skjerm", "Lyd", "Om"]),
                Rect::ZERO,
            )
            .expect("below takes children");
        let body = ui
            .add(below, Panel::filled(denise::Role::Base100), Rect::ZERO)
            .expect("below takes children");
        let body_text = ui
            .add(body, Label::new("Whatever the sidebar left."), Rect::ZERO)
            .expect("in the body");

        let mut screen = Self {
            ui,
            how,
            bar,
            title,
            spacer,
            save,
            below,
            sidebar,
            body,
            body_text,
            started: Instant::now(),
            exit: false,
        };
        screen.lay_out(size);
        screen
    }

    /// Computes every rectangle, whichever way this run was asked for.
    fn lay_out(&mut self, size: Size) {
        match self.how {
            How::ByHand => self.by_hand(size),
            How::Arranged => self.arranged(size),
        }
    }

    /// The arithmetic, written out.
    ///
    /// Not a straw man: this is what the gallery does throughout, and it is
    /// perfectly readable. What it is not is *short*, and every number in it has
    /// to be kept in step with every other by hand.
    fn by_hand(&mut self, size: Size) {
        let (w, h) = (size.width as i32, size.height as i32);
        let inner = Rect::new(PAD, PAD, w - PAD * 2, h - PAD * 2);

        self.ui
            .set_layout(self.bar, Rect::new(inner.x, inner.y, inner.width, BAR));

        // Inside the bar. The title is as wide as its text, so it has to be
        // measured — which is the query `denise-arrange` makes for you.
        let content = Rect::new(GAP, GAP, inner.width - GAP * 2, BAR - GAP * 2);
        let title_w = self
            .ui
            .measure(self.title, denise_ui::Offer::tall(content.height))
            .width
            .unwrap_or(0);
        let save_w = self
            .ui
            .measure(self.save, denise_ui::Offer::tall(content.height))
            .width
            .unwrap_or(0);
        let spacer_w = (content.width - title_w - save_w - GAP * 2).max(0);

        let mut x = content.x;
        for (id, width) in [
            (self.title, title_w),
            (self.spacer, spacer_w),
            (self.save, save_w),
        ] {
            self.ui
                .set_layout(id, Rect::new(x, content.y, width, content.height));
            x += width + GAP;
        }

        // Below the bar: a fixed sidebar beside a body that takes the rest.
        let below = Rect::new(
            inner.x,
            inner.y + BAR + GAP,
            inner.width,
            (inner.height - BAR - GAP).max(0),
        );
        self.ui.set_layout(self.below, below);
        self.ui
            .set_layout(self.sidebar, Rect::new(0, 0, SIDE, below.height));
        let body = Rect::new(
            SIDE + GAP,
            0,
            (below.width - SIDE - GAP).max(0),
            below.height,
        );
        self.ui.set_layout(self.body, body);
        let line = self
            .ui
            .measure(self.body_text, denise_ui::Offer::wide(body.width - GAP * 2))
            .height
            .unwrap_or(0);
        self.ui.set_layout(
            self.body_text,
            Rect::new(GAP, GAP, body.width - GAP * 2, line),
        );
    }

    /// The same rectangles, said rather than computed.
    fn arranged(&mut self, size: Size) {
        let mut arrange = Arrange::new(Flow::Column);
        let screen = arrange.root();
        arrange.set_padding(screen, PAD);
        arrange.set_gap(screen, GAP);

        let bar = arrange.group(screen, Flow::Row, Sizing::Fixed(BAR), Some(self.bar));
        arrange.set_padding(bar, GAP);
        arrange.set_gap(bar, GAP);
        arrange.node(bar, self.title, Sizing::Hug);
        arrange.node(bar, self.spacer, Sizing::Flex(1));
        arrange.node(bar, self.save, Sizing::Hug);

        let below = arrange.group(screen, Flow::Row, Sizing::Flex(1), Some(self.below));
        arrange.set_gap(below, GAP);
        arrange.node(below, self.sidebar, Sizing::Fixed(SIDE));

        let body = arrange.group(below, Flow::Column, Sizing::Flex(1), Some(self.body));
        arrange.set_padding(body, GAP);
        arrange.node(body, self.body_text, Sizing::Hug);

        arrange.apply(&mut self.ui, Rect::from_size(size));
    }

    /// Every rectangle this screen placed, for comparing the two ways.
    #[cfg(test)]
    fn rectangles(&self) -> Vec<Option<Rect>> {
        [
            self.bar,
            self.title,
            self.spacer,
            self.save,
            self.below,
            self.sidebar,
            self.body,
            self.body_text,
        ]
        .iter()
        .map(|id| self.ui.layout(*id))
        .collect()
    }
}

impl DeniseApp for Screen {
    fn update(&mut self, events: &[InputEvent], damage: &mut DamageTracker) {
        for event in events {
            match event {
                // Layout happens when the application says so, and a resize is
                // when it says so. Nothing runs on the frames in between.
                InputEvent::SurfaceResized { size, .. } => self.lay_out(*size),
                InputEvent::Key {
                    code: denise::KeyCode::Escape,
                    state: denise::ElementState::Down,
                    ..
                } => self.exit = true,
                _ => {}
            }
        }
        self.ui.handle(events);
        self.ui.tick(self.started.elapsed().as_millis() as u64);
        if self.ui.needs_paint() {
            damage.add_full();
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, _damage: &[Rect]) {
        self.ui.paint(frame);
        self.ui.presented();
    }

    fn next_frame_in(&self) -> Option<Duration> {
        self.ui.next_wake_ms().map(|_| Duration::from_millis(16))
    }

    fn exit_requested(&self) -> bool {
        self.exit
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let how = if args.iter().any(|a| a == "--by-hand") {
        How::ByHand
    } else {
        How::Arranged
    };
    let size = Size::new(640, 400);

    if let Some(at) = args.iter().position(|a| a == "--snapshot") {
        let out = args
            .get(at + 1)
            .cloned()
            .unwrap_or_else(|| "arranged.ppm".into());
        let mut screen = Screen::new(size, how);
        return write_ppm(&mut screen, size, &out).map_err(Into::into);
    }

    run_with(
        WindowConfig {
            title: format!("arranged — {how:?}"),
            size,
            resizable: true,
            ..WindowConfig::default()
        },
        move |size, _scale| Screen::new(size, how),
    )?;
    Ok(())
}

fn write_ppm(screen: &mut Screen, size: Size, path: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut pixels = vec![0u32; (size.width as usize) * (size.height as usize)];
    {
        let mut frame = Frame::new(
            &mut pixels,
            size,
            size.width,
            denise::PixelFormat::Xrgb8888,
            denise::BufferAge::Undefined,
        )
        .expect("a frame the size of its own buffer");
        screen.ui.paint(&mut frame);
    }
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", size.width, size.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!("wrote {path} at {}x{}", size.width, size.height);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_ways_land_in_exactly_the_same_rectangles() {
        // The whole claim of the crate, as one assertion: it computes what the
        // application would have computed. If this ever fails, one of the two
        // is wrong and the diff says which rectangle.
        for size in [
            Size::new(640, 400),
            Size::new(1024, 600),
            Size::new(320, 240),
        ] {
            let by_hand = Screen::new(size, How::ByHand);
            let arranged = Screen::new(size, How::Arranged);
            assert_eq!(
                by_hand.rectangles(),
                arranged.rectangles(),
                "the two ways disagree at {size:?}",
            );
        }
    }

    #[test]
    fn the_title_is_as_wide_as_its_text_rather_than_a_number_somebody_chose() {
        let mut screen = Screen::new(Size::new(640, 400), How::Arranged);
        let title = screen.ui.layout(screen.title).expect("placed");
        let measured = screen
            .ui
            .measure(screen.title, denise_ui::Offer::NOTHING)
            .width
            .expect("a label has a width");
        assert_eq!(title.width, measured);
    }

    #[test]
    fn the_spacer_takes_whatever_is_left_so_save_stays_on_the_right() {
        for width in [400u32, 640, 1200] {
            let screen = Screen::new(Size::new(width, 400), How::Arranged);
            let save = screen.ui.layout(screen.save).expect("placed");
            let bar = screen.ui.layout(screen.bar).expect("placed");
            assert_eq!(
                save.right(),
                bar.width - GAP,
                "at {width} the button drifted off the right edge",
            );
        }
    }
}
