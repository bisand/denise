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
use denise_ui::widgets::{
    Alert, Align, Avatar, Badge, Button, Carousel, Checkbox, Collapse, Column, Divider, Fit, Image,
    Label, List, ListItem, Panel, Presence, Progress, RadialProgress, RadioGroup, Rating, Select,
    Slider, Spinner, Table, Tabs, TextInput, Timeline, TimelineItem, Toggle, set_open,
};
use denise_ui::{TextStyle, Ui};

const SIZE: Size = Size::new(800, 2160);

// No `Eq`: `Level` carries an f32, and nothing here compares messages anyway.
#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Noop,
    Remember(bool),
    Mode(usize),
    Level(f32),
    Page(usize),
    OpenSelect,
    Stars(f32),
    Row(usize),
    Open(usize),
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
        .add(root, Panel::default(), Rect::new(20, 304, 760, 1564))
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

    // A settings list, beside a disabled copy. Four rows with one of them
    // unselectable, because a row nobody can choose is the state that has to stay
    // readable and still look different from one that can be.
    let rows = || {
        vec![
            ListItem::new("Nettverk").with_trailing("DHCP"),
            ListItem::new("Skjerm").with_trailing("70 %"),
            ListItem::new("Avriming").with_trailing("av").disabled(),
            ListItem::new("Om").with_leading(">"),
        ]
    };
    ui.add(
        form,
        List::new(rows(), Msg::Row)
            .on_activate(Msg::Open)
            .with_selected(Some(1)),
        Rect::new(16, 424, 224, 148),
    )
    .expect("list");
    let locked = ui
        .add(
            form,
            List::new(rows(), Msg::Row).with_selected(Some(1)),
            Rect::new(256, 424, 224, 148),
        )
        .expect("locked list");
    ui.set_enabled(locked, false);

    // Rings: the value at several points, one disabled, and a spinner. The
    // spinner is deliberately two calls — adding it costs nothing, and asking
    // it to animate is what keeps a device awake.
    let mut x = 16;
    for (value, label, role) in [
        (0.0, "0 %", Role::Primary),
        (0.35, "35 %", Role::Primary),
        (0.8, "80 %", Role::Warning),
        (1.0, "100 %", Role::Success),
    ] {
        ui.add(
            form,
            RadialProgress::new(value)
                .with_label(label)
                .with_role(role)
                .with_style(TextStyle::built_in(14)),
            Rect::new(x, 588, 84, 84),
        )
        .expect("ring");
        x += 96;
    }
    let stopped = ui
        .add(
            form,
            RadialProgress::new(0.6).with_label("60 %"),
            Rect::new(x, 588, 84, 84),
        )
        .expect("disabled ring");
    ui.set_enabled(stopped, false);
    x += 108;

    let spinner = ui
        .add(form, Spinner::new(), Rect::new(x, 604, 52, 52))
        .expect("spinner");
    ui.request_animation(spinner);
    // A quarter turn in, so the still picture shows an arc rather than a ring
    // that happens to start at twelve o'clock.
    ui.tick(250);

    // A select, closed and showing its placeholder, beside one with a value —
    // the two states the control has. The open list is a popup, which a still
    // picture of the base scene cannot show; `a_dropdown_opens_chooses_and_closes`
    // is where that half is checked.
    ui.add(
        form,
        Select::new(["Automatisk", "Manuell", "Av"], Msg::OpenSelect)
            .with_placeholder("Velg modus"),
        Rect::new(16, 692, 220, 36),
    )
    .expect("select");
    ui.add(
        form,
        Select::new(["Automatisk", "Manuell", "Av"], Msg::OpenSelect).with_selected(Some(1)),
        Rect::new(256, 692, 220, 36),
    )
    .expect("chosen select");
    let locked_select = ui
        .add(
            form,
            Select::new(["Automatisk", "Manuell", "Av"], Msg::OpenSelect).with_selected(Some(2)),
            Rect::new(496, 692, 220, 36),
        )
        .expect("locked select");
    ui.set_enabled(locked_select, false);

    // One picture, drawn once with the rasteriser itself, shown through every
    // fit — so the letterboxing, the cropping and the stretching are all
    // visible side by side — and once more as the circular avatar crop.
    let (picture, picture_size) = picture();
    let mut x = 16;
    for (fit, caption) in [
        (Fit::Contain, "Contain"),
        (Fit::Cover, "Cover"),
        (Fit::Center, "Center"),
        (Fit::Fill, "Fill"),
    ] {
        let frame_node = ui
            .add(form, Panel::default(), Rect::new(x, 744, 84, 84))
            .expect("picture frame");
        ui.add(
            frame_node,
            Image::new(picture.clone(), picture_size).with_fit(fit),
            Rect::new(2, 2, 80, 80),
        )
        .expect("picture");
        ui.add(
            form,
            Label::new(caption)
                .with_size(12)
                .with_align(Align::Center, Align::Center),
            Rect::new(x, 832, 84, 16),
        )
        .expect("caption");
        x += 96;
    }
    ui.add(
        form,
        Image::new(picture.clone(), picture_size)
            .with_fit(Fit::Cover)
            .with_corner_radius(40),
        Rect::new(x + 8, 746, 80, 80),
    )
    .expect("avatar");
    ui.add(
        form,
        Label::new("Avatar")
            .with_size(12)
            .with_align(Align::Center, Align::Center),
        Rect::new(x + 8, 832, 80, 16),
    )
    .expect("avatar caption");

    // Ratings: an interactive one, an average showing a part-filled star, and
    // a disabled one that must still show its value — the fourth outing for
    // that trap.
    ui.add(
        form,
        Rating::new(3.0, Msg::Stars),
        Rect::new(16, 864, 180, 32),
    )
    .expect("rating");
    ui.add(
        form,
        Rating::<Msg>::display(4.3),
        Rect::new(212, 864, 180, 32),
    )
    .expect("average rating");
    let locked_rating = ui
        .add(
            form,
            Rating::<Msg>::display(2.0),
            Rect::new(408, 864, 180, 32),
        )
        .expect("locked rating");
    ui.set_enabled(locked_rating, false);

    // Avatars: a photo cropped round, initials on colours derived from the
    // names themselves, a ring, a rounded square, and the presence dots.
    let mut x = 16;
    ui.add(
        form,
        Avatar::new(picture.clone(), picture_size).with_presence(Presence::Online),
        Rect::new(x, 908, 56, 56),
    )
    .expect("photo avatar");
    x += 68;
    for (name, presence) in [
        ("Ola Nordmann", Some(Presence::Online)),
        ("Kari Traa", None),
        ("Øystein Åsen", Some(Presence::Busy)),
        ("Per Hansen", Some(Presence::Offline)),
        ("Ida Berg", None),
    ] {
        let mut avatar = Avatar::initials(name);
        if let Some(presence) = presence {
            avatar = avatar.with_presence(presence);
        }
        ui.add(form, avatar, Rect::new(x, 908, 56, 56))
            .expect("avatar");
        x += 68;
    }
    ui.add(
        form,
        Avatar::initials("Nils Aas").with_ring(Role::Primary),
        Rect::new(x, 908, 56, 56),
    )
    .expect("ringed avatar");
    x += 68;
    ui.add(
        form,
        Avatar::initials("Eva Lund").with_corner_radius(12),
        Rect::new(x, 908, 56, 56),
    )
    .expect("squared avatar");

    // A table over fifty rows: five fit, the header is pinned, and the thumb
    // shows where the window sits. Scrolled and selected mid-data, so the
    // still picture proves the window arithmetic rather than the easy case.
    let mut grid = Table::new(
        [
            Column::new("Navn", 200),
            Column::flex("Rolle"),
            Column::new("Nr", 60).align_end(),
        ],
        Msg::Row,
    )
    .with_rows((0..50).map(|i| {
        [
            alloc_free_format(i),
            if i % 3 == 0 { "Operatør" } else { "Vakt" }.to_owned(),
            (100 + i).to_string(),
        ]
    }))
    .with_row_height(32);
    grid.set_scroll(20);
    grid.set_selected(Some(22));
    ui.add(form, grid, Rect::new(16, 980, 500, 196))
        .expect("table");

    // A timeline beside a carousel. The timeline shows mixed states — done,
    // done in Success, pending — with different time widths, so the straight
    // disc line is the thing on display.
    ui.add(
        form,
        Timeline::new([
            TimelineItem::new("Pumpe startet")
                .with_time("12:01")
                .with_role(Role::Success),
            TimelineItem::new("Trykk nådd").with_time("12:04:30"),
            TimelineItem::new("Ventil åpnes").pending(),
            TimelineItem::new("Ferdig").pending(),
        ])
        .with_row_height(34),
        Rect::new(16, 1196, 320, 140),
    )
    .expect("timeline");

    // The carousel, caught mid-slide: ticked to a frame where the first page
    // is still leaving, so the still picture shows the mechanism rather than
    // just a photo in a box.
    let mut rotator = Carousel::new(Msg::Page);
    for tint in [0xFF6688_u32, 0x88CC66, 0x6688EE] {
        let (mut px, size) = crate::picture();
        for word in &mut px {
            // Tint each page so the pages are tellable apart in the picture.
            *word =
                (*word & 0xFF00_0000) | (((*word & 0x00FE_FEFE) >> 1) + (tint & 0x00FF_FFFF) / 2);
        }
        rotator = rotator.with_picture(px, size);
    }
    let rotator = ui
        .add(form, rotator, Rect::new(360, 1196, 240, 140))
        .expect("carousel");
    ui.focus(Some(rotator));
    ui.handle(&[InputEvent::Key {
        code: denise::KeyCode::ArrowRight,
        state: ElementState::Down,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    ui.tick(725); // mid-slide: the previous showcase tick was 600

    // Two notifications, stacked from the bottom edge. Not nodes: the tree
    // times them, stacks them, fades them and removes them, and a still picture
    // catches them mid-hold.
    ui.toast("Lagret kl. 12:01", Role::Success);
    ui.toast("Sensor 3 svarer ikke", Role::Error);
    // Past the fade-in, so the still picture catches them at full opacity
    // rather than at the nothing they are born as.
    ui.tick(600);

    // An accordion: three collapse sections in a stack, the middle one open,
    // the first one caught mid-fold — ticked to a frame where its body is
    // half-cropped, so the still picture shows the mechanism.
    let accordion_stack = ui
        .add(form, Panel::default(), Rect::new(16, 1352, 400, 210))
        .expect("accordion");
    ui.set_stack(accordion_stack, 6);
    let mut sections = Vec::new();
    for title in ["Nettverk", "Skjerm", "Om"] {
        let section = ui
            .add(
                accordion_stack,
                Collapse::new(title, Msg::Remember),
                Rect::new(4, 0, 392, 34 + 56),
            )
            .expect("section");
        ui.add(
            section,
            Label::new("Innhold i seksjonen").with_role(Role::Base300),
            Rect::new(20, 40, 300, 20),
        )
        .expect("body");
        sections.push(section);
    }
    // Fold the first and third; catch the first mid-fold. The clock here
    // continues from the toasts' tick(600) — ticks must never go backwards,
    // or every tween in flight reads elapsed zero and freezes at its start.
    set_open(&mut ui, sections[2], false, 100);
    ui.tick(650);
    set_open(&mut ui, sections[0], false, 100);
    ui.tick(725); // three-quarters through the first fold, body half-cropped

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

/// A little landscape, drawn with the rasteriser itself: the showcase needs a
/// picture and has no business shipping an asset file to get one.
fn picture() -> (Vec<u32>, Size) {
    use denise::{Color, PixelFormat};
    use denise_render::Canvas;

    let size = Size::new(64, 48);
    let mut pixels = vec![0u32; (size.width * size.height) as usize];
    {
        let mut c = Canvas::from_pixels(&mut pixels, size, size.width, PixelFormat::Argb8888)
            .expect("picture buffer");
        c.clear(Color::from_rgb888(0x89B4FA));
        c.fill_rect(Rect::new(0, 36, 64, 12), Color::from_rgb888(0xA6E3A1));
        c.fill_circle(Point::new(48, 12), 8, Color::from_rgb888(0xF9E2AF));
        c.fill_rounded_rect(Rect::new(8, 22, 20, 14), 3, Color::from_rgb888(0xF38BA8));
    }
    // Every pixel is opaque, so the buffer is already premultiplied.
    (pixels, size)
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

/// Names for the table rows, without pulling in anything new.
fn alloc_free_format(i: usize) -> String {
    format!("Person {i}")
}
