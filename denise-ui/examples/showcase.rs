//! Renders every widget in every state to a PPM, so the look can be reviewed
//! without a display.
//!
//! ```text
//! cargo run -p denise-ui --example showcase -- dark showcase.ppm
//! ```
//!
//! Useful in review, and useful when a change to the theme or the font is supposed
//! to be invisible: render before and after and diff the files.

use std::io::Write as _;

use denise::{
    BufferAge, ElementState, Frame, InputEvent, Modifiers, PixelFormat, Point, PointerButton, Rect,
    Role, Size, Theme, theme,
};
use denise_ui::Ui;
use denise_ui::widgets::{
    Alert, Align, Badge, Button, Checkbox, Divider, Label, Panel, Progress, RadioGroup, Slider,
    Tabs, TextInput, Toggle,
};

const SIZE: Size = Size::new(800, 1008);

// No `Eq`: `Level` carries an f32, and nothing here compares messages anyway.
#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Noop,
    Remember(bool),
    Mode(usize),
    Level(f32),
    Page(usize),
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let theme = match args.next().as_deref() {
        Some("light") => theme::LIGHT,
        Some("contrast") => theme::HIGH_CONTRAST,
        _ => theme::DARK,
    };
    let path = args.next().unwrap_or_else(|| "showcase.ppm".to_owned());

    let mut ui = build(theme);
    let mut pixels = vec![0u32; (SIZE.width * SIZE.height) as usize];
    {
        let mut frame = Frame::new(
            &mut pixels,
            SIZE,
            SIZE.width,
            PixelFormat::Xrgb8888,
            BufferAge::Undefined,
        )
        .expect("frame");
        ui.paint(&mut frame);
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
    write!(out, "P6\n{} {}\n255\n", SIZE.width, SIZE.height)?;
    for word in &pixels {
        out.write_all(&[(word >> 16) as u8, (word >> 8) as u8, *word as u8])?;
    }
    out.flush()?;
    eprintln!(
        "wrote {path} ({}) at {}x{}",
        theme.name, SIZE.width, SIZE.height
    );
    Ok(())
}

fn build(theme: Theme) -> Ui<Msg> {
    let mut ui: Ui<Msg> = Ui::new(SIZE, theme);
    let root = ui.root();

    ui.add(
        root,
        Label::new("Denise M3 — widgets, states and a modal")
            .with_size(24)
            .with_align(Align::Center, Align::Center),
        Rect::new(0, 12, SIZE.width as i32, 30),
    )
    .expect("title");

    // Every role, so a theme change can be checked against all of them at once.
    let roles = [
        Role::Primary,
        Role::Secondary,
        Role::Accent,
        Role::Neutral,
        Role::Info,
        Role::Success,
        Role::Warning,
        Role::Error,
    ];
    let strip = ui
        .add(root, Panel::default(), Rect::new(20, 56, 760, 124))
        .expect("strip");
    for (i, role) in roles.iter().enumerate() {
        let x = 16 + (i as i32 % 4) * 186;
        let y = 16 + (i as i32 / 4) * 56;
        ui.add(
            strip,
            Button::new(label_for(*role), Msg::Noop).with_role(*role),
            Rect::new(x, y, 170, 42),
        )
        .expect("role button");
    }

    // A tab strip, sized by asking, with the middle one selected.
    let tabs = Tabs::new(["Oversikt", "Alarmer", "Innstillinger"], Msg::Page).with_selected(1);
    let width = tabs.preferred_width(ui.text_mut());
    ui.add(root, tabs, Rect::new(20, 152, width, 34))
        .expect("tabs");

    // The visual states, acted out through real input rather than forced, so the
    // picture cannot drift from what the tree would actually do. Hover is missing
    // on purpose: the tree hovers exactly one node, and that node here is the one
    // being held.
    let states = ui
        .add(root, Panel::default(), Rect::new(20, 194, 760, 96))
        .expect("states");
    ui.add(
        states,
        Label::new("rest / held / disabled"),
        Rect::new(16, 8, 500, 22),
    )
    .expect("states label");
    let rest = ui
        .add(
            states,
            Button::new("Rest", Msg::Noop),
            Rect::new(16, 38, 170, 42),
        )
        .expect("rest");
    let held = ui
        .add(
            states,
            Button::new("Held", Msg::Noop),
            Rect::new(202, 38, 170, 42),
        )
        .expect("held");
    let off = ui
        .add(
            states,
            Button::new("Disabled", Msg::Noop),
            Rect::new(388, 38, 170, 42),
        )
        .expect("disabled");
    ui.add(
        states,
        Button::new("Warning", Msg::Noop).with_role(Role::Warning),
        Rect::new(574, 38, 170, 42),
    )
    .expect("warning");
    let _ = rest;
    ui.set_enabled(off, false);

    // Fields, one focused with a caret and one showing its placeholder.
    let form = ui
        .add(root, Panel::default(), Rect::new(20, 304, 760, 470))
        .expect("form");
    ui.add(form, Label::new("Navn"), Rect::new(16, 10, 200, 22))
        .expect("name label");
    let name = ui
        .add(
            form,
            TextInput::<Msg>::new().with_placeholder("Ola Nordmann"),
            Rect::new(16, 36, 360, 40),
        )
        .expect("name");
    ui.add(form, Label::new("PIN"), Rect::new(396, 10, 200, 22))
        .expect("pin label");
    let pin = ui
        .add(
            form,
            TextInput::<Msg>::new().with_password(true),
            Rect::new(396, 36, 348, 40),
        )
        .expect("pin");
    ui.add(
        form,
        Label::new("Kjærlighet på Øy — æøå ÆØÅ 0123456789 ±25 °C"),
        Rect::new(16, 90, 728, 24),
    )
    .expect("charset");

    // Both states side by side, and a disabled one, since a checkbox that only
    // ever appears unticked in a picture proves nothing about the tick.
    ui.add(
        form,
        Checkbox::new("Husk meg", Msg::Remember).with_checked(true),
        Rect::new(16, 120, 220, 28),
    )
    .expect("remember");
    ui.add(
        form,
        Checkbox::new("Send rapport", Msg::Remember),
        Rect::new(256, 120, 220, 28),
    )
    .expect("report");
    let locked = ui
        .add(
            form,
            Checkbox::new("Låst", Msg::Remember).with_checked(true),
            Rect::new(496, 120, 220, 28),
        )
        .expect("locked");
    ui.set_enabled(locked, false);

    // The same boolean as a switch. Both states, because a knob only ever drawn
    // at one end proves nothing about the travel.
    ui.add(
        form,
        Toggle::new("Nattmodus", Msg::Remember).with_checked(true),
        Rect::new(16, 156, 220, 28),
    )
    .expect("night");
    ui.add(
        form,
        Toggle::new("Demping", Msg::Remember),
        Rect::new(256, 156, 220, 28),
    )
    .expect("dim");
    let fixed = ui
        .add(
            form,
            Toggle::new("Fastlåst", Msg::Remember).with_checked(true),
            Rect::new(496, 156, 220, 28),
        )
        .expect("fixed");
    ui.set_enabled(fixed, false);

    // One choice from three. Drawn beside a disabled copy, because the chosen
    // disc is what the disabled state has to keep readable.
    ui.add(
        form,
        RadioGroup::new(["Automatisk", "Manuell", "Av"], Msg::Mode).with_selected(1),
        Rect::new(16, 194, 220, 84),
    )
    .expect("mode");
    let frozen = ui
        .add(
            form,
            RadioGroup::new(["Automatisk", "Manuell", "Av"], Msg::Mode).with_selected(2),
            Rect::new(256, 194, 220, 84),
        )
        .expect("frozen");
    ui.set_enabled(frozen, false);

    // Bars, including the two ends. Empty and full are the values a bar drawn
    // only at 60% would never show were wrong.
    for (index, (value, role)) in [
        (0.0, Role::Primary),
        (0.35, Role::Primary),
        (0.8, Role::Warning),
        (1.0, Role::Success),
    ]
    .into_iter()
    .enumerate()
    {
        ui.add(
            form,
            Progress::new(value).with_role(role),
            Rect::new(496, 200 + index as i32 * 22, 220, 12),
        )
        .expect("bar");
    }

    // Sliders at both ends and in the middle, plus one that snaps. The knob has
    // to stay inside its rectangle at the extremes, which is the thing a picture
    // taken only at 50% would never show.
    for (index, (value, step)) in [(0.0, None), (0.5, None), (1.0, None), (0.4, Some(0.2))]
        .into_iter()
        .enumerate()
    {
        let mut slider = Slider::new(0.0, 1.0, value, Msg::Level);
        if let Some(step) = step {
            slider = slider.with_step(step);
        }
        ui.add(
            form,
            slider,
            Rect::new(16, 290 + index as i32 * 30, 220, 26),
        )
        .expect("slider");
    }

    // A plain rule, a labelled one, and the vertical case between the columns.
    ui.add(form, Divider::new(), Rect::new(256, 296, 220, 14))
        .expect("rule");
    ui.add(
        form,
        Divider::labelled("eller"),
        Rect::new(256, 330, 220, 24),
    )
    .expect("labelled rule");
    ui.add(form, Divider::vertical(), Rect::new(486, 290, 14, 100))
        .expect("vertical rule");

    // Badges, sized by asking rather than by guessing — which is the whole
    // sizing convention demonstrated in three lines.
    let mut x = 516;
    for (text, role) in [
        ("3", Role::Primary),
        ("PÅ", Role::Success),
        ("VENTER", Role::Warning),
        ("FEIL", Role::Error),
    ] {
        let badge = Badge::new(text).with_role(role);
        let width = badge.preferred_width(ui.text_mut());
        let height = badge.preferred_height(ui.text_mut());
        ui.add(form, badge, Rect::new(x, 296, width, height))
            .expect("badge");
        x += width + 10;
    }

    // Banners: one short, one long enough to wrap, sized by asking.
    let mut y = 330;
    for (role, message, icon) in [
        (Role::Success, "Lagret", Some('+')),
        (
            Role::Error,
            "Kunne ikke lagre fordi disken er full og det er ingen plass igjen",
            Some('!'),
        ),
    ] {
        let mut alert = Alert::new(role, message);
        if let Some(icon) = icon {
            alert = alert.with_icon(icon);
        }
        let height = alert.preferred_height(ui.text_mut(), 244);
        ui.add(form, alert, Rect::new(496, y, 244, height))
            .expect("alert");
        y += height + 8;
    }

    // Drive the states through real input so the picture cannot drift from what
    // the tree would actually do.
    ui.focus(Some(name));
    for ch in "Kjærlighet".chars() {
        ui.handle(&[InputEvent::Text { ch }]);
    }
    ui.widget_mut::<TextInput<Msg>>(pin)
        .expect("pin")
        .set_text("1234");

    let at = |ui: &Ui<Msg>, id| {
        let b: Rect = ui.bounds(id).expect("bounds");
        Point::new(b.x + b.width / 2, b.y + b.height / 2)
    };
    let held_at = at(&ui, held);
    ui.handle(&[
        InputEvent::PointerMoved { position: held_at },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: ElementState::Down,
            position: held_at,
            modifiers: Modifiers::NONE,
        },
    ]);
    assert!(
        ui.hovered() == Some(held),
        "a held button is also the hovered one"
    );

    // And the modal, over a dimmed backdrop.
    let scene = ui.push_scene(110);
    let w = 420;
    let h = 170;
    let dialog = ui
        .add(
            scene,
            Panel::default().with_border(Role::Primary, 2),
            Rect::new(
                (SIZE.width as i32 - w) / 2,
                SIZE.height as i32 - h - 40,
                w,
                h,
            ),
        )
        .expect("dialog");
    ui.add(
        dialog,
        Label::new("Lagre endringer?")
            .with_size(24)
            .with_align(Align::Center, Align::Center),
        Rect::new(20, 18, w - 40, 30),
    )
    .expect("dialog title");
    ui.add(
        dialog,
        Label::new("nothing below takes input")
            .with_role(Role::Base300)
            .with_align(Align::Center, Align::Center),
        Rect::new(20, 56, w - 40, 22),
    )
    .expect("dialog body");
    let yes = ui
        .add(
            dialog,
            Button::new("Ja", Msg::Noop),
            Rect::new(20, h - 62, (w - 60) / 2, 44),
        )
        .expect("yes");
    ui.add(
        dialog,
        Button::new("Avbryt", Msg::Noop).with_role(Role::Neutral),
        Rect::new(40 + (w - 60) / 2, h - 62, (w - 60) / 2, 44),
    )
    .expect("no");
    ui.focus(Some(yes));
    assert!(
        ui.widget::<Button<Msg>>(yes).is_some(),
        "the dialog's default button should be focused"
    );

    // Park the cursor sprite somewhere it can be seen against the dim.
    ui.handle(&[InputEvent::PointerMoved {
        position: Point::new(560, 470),
    }]);

    ui
}

fn label_for(role: Role) -> &'static str {
    match role {
        Role::Primary => "Primary",
        Role::Secondary => "Secondary",
        Role::Accent => "Accent",
        Role::Neutral => "Neutral",
        Role::Info => "Info",
        Role::Success => "Success",
        Role::Warning => "Warning",
        Role::Error => "Error",
        _ => "Role",
    }
}
