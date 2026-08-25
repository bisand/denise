//! A form saying whether it may be drawn at another size, and the engine doing
//! the multiplying.
//!
//! [#111]. The three lines that are the whole DPI story for an application that
//! computes its own rectangles are not available to a form file, because there is
//! no application computing them — so the multiply goes where the rectangles are
//! computed, which is `denise-forms`.
//!
//! What is asserted here is the policy, the arithmetic and the rounding. What it
//! *looks* like is `denise-forms render --scale`, and three snapshots of the
//! reference form are committed beside this.
//!
//! [#111]: https://github.com/bisand/denise/issues/111

use denise::{Rect, Size};
use denise_forms::{Form, Handler, Payload, Picture, Scaling, Wiring};
use denise_ui::widgets::{Panel, Value};
use denise_ui::{Ui, Void};

/// Wiring that answers every name with whatever shape is asked for.
///
/// The same one `tests/build.rs` has, for the same reason: what an application
/// does with a message name is the application's, and none of this is about it.
struct Anything;

impl Wiring<Void> for Anything {
    fn message(&mut self, _name: &str, payload: Payload) -> Option<Handler<Void>> {
        Some(match payload {
            Payload::None => Handler::Plain(Void),
            Payload::Bool => Handler::Bool(|_| Void),
            Payload::Index => Handler::Index(|_| Void),
            Payload::Number => Handler::Number(|_| Void),
        })
    }

    fn asset(&mut self, _path: &str) -> Option<Picture> {
        Some(Picture {
            pixels: vec![0xFF00_0000; 4],
            size: Size::new(2, 2),
        })
    }
}

fn form(scaling: &str, body: &str) -> Form {
    let said = if scaling.is_empty() {
        String::new()
    } else {
        format!(" scaling={scaling}")
    };
    Form::parse(&format!(
        "form \"F\" version=1 width=200 height=100{said} {{\n{body}}}\n"
    ))
    .expect("a test form parses")
}

/// Builds a form onto a surface the way an application does: the theme scaled
/// once, a panel where the fit says, and the form built into it.
fn shown(form: &Form, surface: Size) -> (Ui<Void>, denise_forms::Built) {
    let fit = form.fit(surface);
    let mut ui: Ui<Void> = Ui::new(surface, form.theme().scaled(fit.uniform()));
    let root = ui.root();
    let stage = ui
        .add(root, Panel::filled(form.background()), fit.rect)
        .expect("the root takes a child");
    let mut nothing = Anything;
    let built = form
        .build_fitted(&mut ui, stage, fit, &mut nothing)
        .expect("builds");
    (ui, built)
}

#[test]
fn a_form_that_says_nothing_is_never_scaled() {
    // The default has to be what every form written before this property existed
    // already did, or adding the property changed those forms.
    assert_eq!(form("", "").scaling(), Scaling::None);
    assert_eq!(form("none", "").scaling(), Scaling::None);
    assert_eq!(form("proportional", "").scaling(), Scaling::Proportional);
    assert_eq!(form("stretch", "").scaling(), Scaling::Stretch);
}

#[test]
fn a_fixed_form_on_a_bigger_surface_is_its_own_size_in_the_middle() {
    let fixed = form(
        "none",
        "    label \"hi\" name=hi x=10 y=10 w=100 h=20 size=16\n",
    );
    let fit = fixed.fit(Size::new(800, 600));

    assert_eq!((fit.x, fit.y), (1.0, 1.0));
    assert_eq!(fit.rect, Rect::new(300, 250, 200, 100), "not centred");

    // And nothing inside it moved either: same rectangle, same text size.
    let (ui, built) = shown(&fixed, Size::new(800, 600));
    let hi = built.node("hi").expect("named");
    assert_eq!(ui.layout(hi), Some(Rect::new(10, 10, 100, 20)));
    assert_eq!(ui.get_property(hi, "size"), Some(Value::Int(16)));
}

#[test]
fn a_fixed_form_on_a_smaller_surface_is_still_its_own_size() {
    // Centring a form larger than the surface puts its origin negative, which is
    // right: it is cropped evenly rather than cropped from one corner, and the
    // application that gave it too little room can see that it did.
    let fixed = form("none", "");
    let fit = fixed.fit(Size::new(100, 100));
    assert_eq!(fit.rect, Rect::new(-50, 0, 200, 100));
}

#[test]
fn proportional_letterboxes_on_whichever_axis_has_room_left() {
    let fits = form("proportional", "");

    // A target twice as wide in proportion: height decides, and the margin is
    // left and right.
    let wide = fits.fit(Size::new(800, 200));
    assert_eq!((wide.x, wide.y), (2.0, 2.0), "the tighter axis decides");
    assert_eq!(wide.rect, Rect::new(200, 0, 400, 200));

    // A target twice as tall in proportion: width decides, margin top and bottom.
    let tall = fits.fit(Size::new(400, 400));
    assert_eq!((tall.x, tall.y), (2.0, 2.0));
    assert_eq!(tall.rect, Rect::new(0, 100, 400, 200));

    // Exactly in proportion: no margin at all, on either axis.
    let exact = fits.fit(Size::new(400, 200));
    assert_eq!(exact.rect, Rect::from_size(Size::new(400, 200)));
}

#[test]
fn stretch_fills_the_surface_and_distorts_to_do_it() {
    let fills = form(
        "stretch",
        "    label \"hi\" name=hi x=10 y=10 w=100 h=20 size=16\n",
    );
    let surface = Size::new(400, 400);
    let fit = fills.fit(surface);

    assert_eq!((fit.x, fit.y), (2.0, 4.0));
    assert_eq!(fit.rect, Rect::from_size(surface), "left a gap");

    let (ui, built) = shown(&fills, surface);
    let hi = built.node("hi").expect("named");
    assert_eq!(ui.layout(hi), Some(Rect::new(20, 40, 200, 80)));
    // Text takes the *smaller* factor, so a stretched layout never grows text
    // taller than the axis with least room to give.
    assert_eq!(ui.get_property(hi, "size"), Some(Value::Int(32)));
}

#[test]
fn panels_designed_to_touch_still_touch_at_a_fractional_scale() {
    // The whole reason `Rect::scaled_by` works on edges. At 0.75 these two round
    // to 30 and 45 wide; scaling width and height directly would give 30 and 30
    // and open a one-pixel seam between panels that were designed to meet.
    let side_by_side = form(
        "proportional",
        "    panel name=left x=0 y=0 w=40 h=100\n    panel name=right x=40 y=0 w=60 h=100\n",
    );
    let (ui, built) = shown(&side_by_side, Size::new(150, 75));

    let left = ui
        .layout(built.node("left").expect("named"))
        .expect("laid out");
    let right = ui
        .layout(built.node("right").expect("named"))
        .expect("laid out");
    assert_eq!(left.right(), right.x, "a seam opened between them");
    assert_eq!((left.width, right.width), (30, 45));
}

#[test]
fn a_length_that_would_round_to_nothing_keeps_a_pixel() {
    // A one-pixel border at 0.75 is 0.75, and `0` is the file saying "no border"
    // rather than the arithmetic saying so. Deleting a hairline is a visible
    // change; keeping it is not.
    let hairline = form(
        "proportional",
        "    panel name=box x=0 y=0 w=100 h=50 border=primary border-width=1\n",
    );
    let (ui, built) = shown(&hairline, Size::new(100, 50));
    let box_id = built.node("box").expect("named");
    assert_eq!(ui.get_property(box_id, "border-width"), Some(Value::Int(1)));

    // And a zero stays zero, because that was somebody saying none.
    let none = form(
        "proportional",
        "    panel name=box x=0 y=0 w=100 h=50 border-width=0\n",
    );
    let (ui, built) = shown(&none, Size::new(400, 200));
    let box_id = built.node("box").expect("named");
    assert_eq!(ui.get_property(box_id, "border-width"), Some(Value::Int(0)));
}

#[test]
fn what_is_not_a_length_is_not_multiplied() {
    // The distinction the widget declares and this crate reads: a duration is
    // milliseconds at every scale, and a selected row is the same row on a wall.
    let mixed = form(
        "stretch",
        "    list name=nav x=0 y=0 w=100 h=80 selected=2 row-height=20 size=12 {\n        \
         item \"one\"\n        item \"two\"\n        item \"three\"\n    }\n\
         \x20   spinner name=busy x=100 y=0 w=20 h=20 period-ms=900 frame-ms=100 thickness=3\n",
    );
    let (ui, built) = shown(&mixed, Size::new(400, 200));

    let nav = built.node("nav").expect("named");
    assert_eq!(
        ui.get_property(nav, "row-height"),
        Some(Value::Int(40)),
        "a length"
    );
    assert_eq!(
        ui.get_property(nav, "size"),
        Some(Value::Int(24)),
        "a length"
    );
    assert_eq!(
        ui.get_property(nav, "selected"),
        Some(Value::Int(2)),
        "an index"
    );

    let busy = built.node("busy").expect("named");
    assert_eq!(
        ui.get_property(busy, "thickness"),
        Some(Value::Int(6)),
        "a length"
    );
    assert_eq!(
        ui.get_property(busy, "period-ms"),
        Some(Value::Int(900)),
        "a duration is not a length",
    );
    assert_eq!(ui.get_property(busy, "frame-ms"), Some(Value::Int(100)));
}

#[test]
fn build_scaled_is_build_fitted_with_one_factor() {
    let plain = form(
        "",
        "    label \"hi\" name=hi x=10 y=10 w=100 h=20 size=16\n",
    );
    let surface = Size::new(400, 200);

    let mut one: Ui<Void> = Ui::new(surface, plain.theme().scaled(2.0));
    let root = one.root();
    let mut nothing = Anything;
    let a = plain
        .build_scaled(&mut one, root, 2.0, &mut nothing)
        .expect("builds");

    let mut two: Ui<Void> = Ui::new(surface, plain.theme().scaled(2.0));
    let root = two.root();
    let mut nothing = Anything;
    let b = plain
        .build_fitted(
            &mut two,
            root,
            denise_forms::Placement {
                x: 2.0,
                y: 2.0,
                rect: Rect::new(0, 0, 400, 200),
            },
            &mut nothing,
        )
        .expect("builds");

    let of = |ui: &Ui<Void>, built: &denise_forms::Built| {
        let id = built.node("hi").expect("named");
        (ui.layout(id), ui.get_property(id, "size"))
    };
    assert_eq!(of(&one, &a), of(&two, &b));
    assert_eq!(of(&one, &a).0, Some(Rect::new(20, 20, 200, 40)));
}

#[test]
fn building_a_form_the_old_way_is_still_the_old_way() {
    // `build` is `build_fitted` at 1:1, and the whole corpus of forms in this
    // repository goes through it. Nothing about them may have moved.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../forms/reference.dform"),
    )
    .expect("the reference form");
    let reference = Form::parse(&source).expect("parses");

    let mut ui: Ui<Void> = Ui::new(reference.size(), reference.theme());
    let root = ui.root();
    let mut nothing = Anything;
    let built = reference
        .build(&mut ui, root, &mut nothing)
        .expect("builds");

    let volume = built.node("volume").expect("the reference form names it");
    let was = ui.layout(volume).expect("laid out");

    let mut same: Ui<Void> = Ui::new(reference.size(), reference.theme());
    let root = same.root();
    let mut nothing = Anything;
    let built = reference
        .build_scaled(&mut same, root, 1.0, &mut nothing)
        .expect("builds");
    assert_eq!(
        same.layout(built.node("volume").expect("named")),
        Some(was),
        "1x is not the same as not scaling",
    );
}
