//! A widget that scrolls itself — a log view keeping its own top line, a table
//! with a pinned header — and what the tree does with the move it reports
//! through `EventCtx::scrolled`: the rows still on screen are moved within the
//! frame, the strip that came into view is the only part of them painted, and
//! everything that stops that from being safe makes the tree repaint instead.

use std::cell::RefCell;

use denise::{BufferAge, Color, Frame, InputEvent, PixelFormat, Point, Rect, Size, theme};
use denise_render::Pen;
use denise_ui::widget::{Event, EventCtx, Handled, PaintCtx, Widget};
use denise_ui::widgets::Panel;
use denise_ui::{NodeId, Ui};

const SIZE: Size = Size::new(200, 120);
/// Where the ledger sits, and the band along its bottom that never moves.
const BOUNDS: Rect = Rect::new(20, 10, 160, 70);
const BAND: i32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Msg {}

/// Rows whose red channel says which row of *content* they are, a green band
/// at the bottom that is not part of the scroll, and a note of every clip the
/// widget was asked to paint.
struct Ledger {
    top: i32,
    painted: RefCell<Vec<Rect>>,
    /// Also asks for a repaint on every scroll, which takes the move back.
    also_invalidates: bool,
}

impl Ledger {
    fn rows(bounds: Rect) -> Rect {
        Rect::new(bounds.x, bounds.y, bounds.width, bounds.height - BAND)
    }
}

impl Widget<Msg> for Ledger {
    fn accepts_pointer(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut Pen<'_>) {
        self.painted.borrow_mut().push(canvas.clip());
        let rows = Self::rows(ctx.bounds);
        for y in 0..rows.height {
            let content = (self.top + y) as u8;
            canvas.fill_rect(
                Rect::new(rows.x, rows.y + y, rows.width, 1),
                Color::rgb(content, 0, 0),
            );
        }
        canvas.fill_rect(
            Rect::new(rows.x, rows.bottom(), rows.width, BAND),
            Color::rgb(0, 255, 0),
        );
    }

    fn on_event(&mut self, event: &Event<'_>, ctx: &mut EventCtx<'_, Msg>) -> Handled {
        let Event::Input(InputEvent::PointerScroll { delta_y, .. }) = event else {
            return Handled::No;
        };
        let dy = *delta_y as i32;
        self.top += dy;
        ctx.scrolled(Self::rows(ctx.bounds), Point::new(0, dy));
        if self.also_invalidates {
            ctx.invalidate();
        }
        Handled::Yes
    }
}

/// A tree with a ledger in it, and a frame buffer kept between paints — the
/// move happens inside that buffer, so it has to be the same one each time.
struct Fixture {
    ui: Ui<Msg>,
    ledger: NodeId,
    pixels: Vec<u32>,
}

impl Fixture {
    fn new(also_invalidates: bool) -> Self {
        let mut ui: Ui<Msg> = Ui::new(SIZE, theme::DARK);
        let root = ui.root();
        let ledger = ui
            .add(
                root,
                Ledger {
                    top: 100,
                    painted: RefCell::new(Vec::new()),
                    also_invalidates,
                },
                BOUNDS,
            )
            .expect("ledger");
        // The window system draws the pointer; a sprite over the rows would
        // rightly stop the move, and is not what these tests are about.
        ui.show_cursor(false);
        // The pointer is already over the ledger when the gesture starts, as
        // it is in life: arriving there is a hover change, which dirties the
        // node, and a frame that did that as well as scroll is not just a
        // scroll.
        ui.handle(&[InputEvent::PointerMoved {
            position: Point::new(BOUNDS.x + 5, BOUNDS.y + 5),
        }]);
        let mut fixture = Self {
            ui,
            ledger,
            pixels: vec![0; (SIZE.width * SIZE.height) as usize],
        };
        fixture.paint(BufferAge::Undefined);
        fixture
    }

    /// Paints into the kept buffer and hands back the clips the ledger drew.
    fn paint(&mut self, age: BufferAge) -> Vec<Rect> {
        let mut pixels = std::mem::take(&mut self.pixels);
        let painted = self.paint_in(&mut pixels, age);
        self.pixels = pixels;
        painted
    }

    /// Paints into `pixels`, a buffer `age` frames behind the tree.
    fn paint_in(&mut self, pixels: &mut [u32], age: BufferAge) -> Vec<Rect> {
        let mut frame =
            Frame::new(pixels, SIZE, SIZE.width, PixelFormat::Xrgb8888, age).expect("frame");
        self.ui.paint(&mut frame);
        drop(frame);
        self.ui.presented();
        // Read, not `widget_mut`: mutable access invalidates the node, which
        // would make every frame after a paint one that did more than scroll.
        let ledger = self.ui.widget::<Ledger>(self.ledger).expect("ledger");
        ledger.painted.borrow_mut().drain(..).collect()
    }

    fn scroll(&mut self, dy: f32) {
        self.ui.handle(&[InputEvent::PointerScroll {
            delta_x: 0.0,
            delta_y: dy,
            position: Point::new(BOUNDS.x + 5, BOUNDS.y + 5),
        }]);
    }

    fn red_at(&self, x: i32, y: i32) -> u32 {
        (self.pixels[(y * SIZE.width as i32 + x) as usize] >> 16) & 0xFF
    }

    fn green_at(&self, x: i32, y: i32) -> u32 {
        (self.pixels[(y * SIZE.width as i32 + x) as usize] >> 8) & 0xFF
    }

    /// Every row of the ledger reads as the content a fresh paint would give.
    fn assert_rows_are(&self, top: i32) {
        let rows = Ledger::rows(BOUNDS);
        for y in 0..rows.height {
            assert_eq!(
                self.red_at(rows.x + 3, rows.y + y),
                (top + y) as u32 & 0xFF,
                "row {y} of the ledger"
            );
        }
        assert_eq!(
            self.green_at(rows.x + 3, rows.bottom() + 1),
            255,
            "the band"
        );
    }
}

/// The whole point: after a scroll, the ledger is asked to paint the strip
/// that came into view and the band it left out of the move, and nothing
/// else — yet every row reads as a full repaint would.
#[test]
fn a_widget_that_scrolls_itself_paints_only_the_strip_and_what_it_left_out() {
    let mut f = Fixture::new(false);
    let rows = Ledger::rows(BOUNDS);

    f.scroll(3.0);
    let painted = f.paint(BufferAge::Frames(1));
    let strip = Rect::new(rows.x, rows.bottom() - 3, rows.width, 3);
    let band = Rect::new(rows.x, rows.bottom(), rows.width, BAND);
    assert_eq!(painted.len(), 2, "painted {painted:?}");
    assert!(
        painted.contains(&strip),
        "painted {painted:?}, wanted {strip:?}"
    );
    assert!(
        painted.contains(&band),
        "painted {painted:?}, wanted {band:?}"
    );
    f.assert_rows_are(103);

    // And back down, which is the other copy direction and the other strip.
    f.scroll(-5.0);
    let painted = f.paint(BufferAge::Frames(1));
    let strip = Rect::new(rows.x, rows.y, rows.width, 5);
    assert!(
        painted.contains(&strip),
        "painted {painted:?}, wanted {strip:?}"
    );
    assert_eq!(painted.len(), 2, "painted {painted:?}");
    f.assert_rows_are(98);
}

/// Two scrolls in one frame are one move, and a buffer two frames old gets
/// the moves of both frames — which is what a double-buffered panel hands
/// back, and why the record is a ring.
#[test]
fn moves_accumulate_across_events_and_frames() {
    let mut f = Fixture::new(false);
    // A second buffer, holding what the first did before any scroll.
    let mut other = f.pixels.clone();

    f.scroll(2.0);
    f.scroll(3.0);
    let painted = f.paint(BufferAge::Frames(1));
    assert_eq!(painted.len(), 2, "painted {painted:?}");
    f.assert_rows_are(105);

    f.scroll(4.0);
    let painted = f.paint_in(&mut other, BufferAge::Frames(2));
    let rows = Ledger::rows(BOUNDS);
    let strip = Rect::new(rows.x, rows.bottom() - 9, rows.width, 9);
    assert!(
        painted.contains(&strip),
        "painted {painted:?}, wanted {strip:?}"
    );
    std::mem::swap(&mut f.pixels, &mut other);
    f.assert_rows_are(109);
}

/// The reported damage is the whole widget, whatever was actually drawn: the
/// rows moved in this buffer, not in the one on the panel.
#[test]
fn the_damage_reported_is_still_the_whole_widget() {
    let mut f = Fixture::new(false);
    f.scroll(3.0);
    let mut frame = Frame::new(
        &mut f.pixels,
        SIZE,
        SIZE.width,
        PixelFormat::Xrgb8888,
        BufferAge::Frames(1),
    )
    .expect("frame");
    f.ui.paint(&mut frame);
    drop(frame);
    let damage = f.ui.damage().to_vec();
    assert_eq!(damage, vec![BOUNDS], "damage {damage:?}");
}

/// A widget that says it moved and also asks for a repaint gets the repaint:
/// `invalidate` means "draw me again", and that wins.
#[test]
fn invalidating_as_well_takes_the_move_back() {
    let mut f = Fixture::new(true);
    f.scroll(3.0);
    let painted = f.paint(BufferAge::Frames(1));
    assert_eq!(painted, vec![BOUNDS], "painted {painted:?}");
    f.assert_rows_are(103);
}

/// A node painted over the rows — a bar the application floated above its
/// log — would be moved with them and leave a ghost. The tree repaints.
#[test]
fn a_node_painted_over_the_rows_stops_the_move() {
    let mut f = Fixture::new(false);
    let root = f.ui.root();
    f.ui.add(root, Panel::default(), Rect::new(60, 20, 80, 24))
        .expect("bar");
    f.paint(BufferAge::Undefined);

    f.scroll(3.0);
    let painted = f.paint(BufferAge::Frames(1));
    assert_eq!(painted, vec![BOUNDS], "painted {painted:?}");
    f.assert_rows_are(103);
}

/// Something else dirty in the same frame — a label elsewhere, the widget's
/// own repaint — means the frame was not just a scroll, and is painted whole.
#[test]
fn other_damage_in_the_frame_means_a_repaint_of_the_rows() {
    let mut f = Fixture::new(false);
    let root = f.ui.root();
    let label =
        f.ui.add(root, Panel::default(), Rect::new(0, 100, 200, 20))
            .expect("label");
    f.paint(BufferAge::Undefined);

    f.ui.invalidate(label);
    f.scroll(3.0);
    let painted = f.paint(BufferAge::Frames(1));
    assert_eq!(painted, vec![BOUNDS], "painted {painted:?}");
    f.assert_rows_are(103);
}

/// A painter that cannot move its own pixels is told nothing was moved and
/// paints the viewport whole — the honest default every compositor keeps.
#[test]
fn a_painter_that_cannot_move_rows_gets_the_whole_viewport() {
    use denise::{Mask, Paint, Painter, PixelView};

    /// Records what it is asked to fill, and moves nothing.
    struct Tally {
        clip: Rect,
        painted: Vec<Rect>,
    }
    impl Painter for Tally {
        fn size(&self) -> Size {
            SIZE
        }
        fn format(&self) -> PixelFormat {
            PixelFormat::Xrgb8888
        }
        fn clip(&self) -> Rect {
            self.clip
        }
        fn push_clip(&mut self, rect: Rect) -> denise::ClipToken {
            let previous = self.clip;
            self.clip = self.clip.intersect(&rect).unwrap_or(Rect::ZERO);
            denise::ClipToken::restoring(previous)
        }
        fn pop_clip(&mut self, token: denise::ClipToken) {
            self.clip = token.previous();
        }
        fn clear(&mut self, _: Color) {}
        fn fill_rect(&mut self, rect: Rect, _: Paint) {
            if let Some(visible) = rect.intersect(&self.clip) {
                self.painted.push(visible);
            }
        }
        fn fill_rounded_rect(&mut self, _: Rect, _: i32, _: Paint) {}
        fn stroke_rounded_rect(&mut self, _: Rect, _: i32, _: i32, _: Paint) {}
        fn fill_circle(&mut self, _: Point, _: i32, _: Paint) {}
        fn stroke_circle(&mut self, _: Point, _: i32, _: i32, _: Paint) {}
        fn stroke_arc(&mut self, _: Point, _: i32, _: i32, _: i32, _: i32, _: Paint) {}
        fn draw_line(&mut self, _: Point, _: Point, _: Paint) {}
        fn fill_polygon_fx(&mut self, _: &[(i32, i32)], _: Paint) {}
        fn blit_mask(&mut self, _: Point, _: &Mask<'_>, _: Paint) {}
        fn blit(&mut self, _: &PixelView<'_>, _: Point) {}
        fn blit_scaled(&mut self, _: &PixelView<'_>, _: Rect) {}
        fn blit_rounded(&mut self, _: &PixelView<'_>, _: Rect, _: Rect, _: i32) {}
    }

    let mut f = Fixture::new(false);
    f.scroll(3.0);
    let mut tally = Tally {
        clip: Rect::from_size(SIZE),
        painted: Vec::new(),
    };
    f.ui.paint_with(&mut Pen::new(&mut tally), BufferAge::Frames(1));
    // Every row of the ledger was filled, not just a strip of it.
    let rows = Ledger::rows(BOUNDS);
    let filled: Vec<&Rect> = tally.painted.iter().filter(|r| r.height == 1).collect();
    assert_eq!(filled.len(), rows.height as usize, "filled {filled:?}");
}
