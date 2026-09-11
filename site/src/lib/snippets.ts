// The code the home page shows. Each of these is the real thing, trimmed:
// `rust` and `messages` are examples/hello, `form` is forms/hello.dform, and
// `c` is denise-ffi/examples/panel.c.

export const snippets: { id: string; label: string; lang: string; note: string; code: string }[] = [
	{
		id: 'rust',
		label: 'Rust',
		lang: 'rust',
		note: 'A tree of widgets, positioned with rectangles. No callbacks: a button holds a value of your type.',
		code: `use denise::{Rect, Role, Size, theme};
use denise_ui::widgets::{Button, Label, TextInput};
use denise_ui::Ui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Message {
    Greet,
}

let mut ui: Ui<Message> = Ui::new(Size::new(460, 260), theme::DARK);
let root = ui.root();

ui.add(root, Label::new("What is your name?"), Rect::new(20, 20, 388, 20));
let name = ui.add(root, TextInput::<Message>::new(), Rect::new(20, 44, 388, 34)).unwrap();
ui.add(
    root,
    Button::new("Greet", Message::Greet).with_role(Role::Primary),
    Rect::new(20, 90, 110, 34),
);`
	},
	{
		id: 'messages',
		label: 'The loop',
		lang: 'rust',
		note: 'Every state change happens in one match you wrote. There are no dirty flags and no invalidate() calls anywhere.',
		code: `for message in ui.drain_messages().collect::<Vec<_>>() {
    match message {
        Message::Greet => { /* read the field, update a label */ }
    }
}

// Type into the field and the toolkit repaints the field, not the window.
ui.tick(now_ms);
ui.render(&mut surface)?;`
	},
	{
		id: 'form',
		label: 'A form file',
		lang: 'kdl',
		note: 'The same screen as a .dform file: text, in git diff, hand-editable, and loaded at run time by denise-forms. The designer reads and writes it.',
		code: `form "Hello" version=1 kind=screen width=460 height=260 theme=dark {
    panel name=card x=16 y=16 w=428 h=228 {
        label "Hello, Denise"      x=20 y=18 w=388 h=28 size=22
        label "What is your name?" x=20 y=58 w=388 h=20 size=16

        // Enter submits, so a keypad-only panel never needs the button.
        text-input name=who x=20 y=82 w=388 h=34 placeholder="your name" \\
            on-submit=greet size=16 focus=#true

        button "Greet" x=20 y=128 w=110 h=34 role=primary on-press=greet size=16

        // Filled in by the application from what was typed.
        label "" name=greeting x=20 y=176 w=388 h=24 size=16
    }
}`
	},
	{
		id: 'c',
		label: 'From C',
		lang: 'c',
		note: 'The same toolkit through a stable C ABI and a hand-written header. The host owns the buffer; swap it for a DIB section and this is the Win32 control.',
		code: `DeniseUi *ui = denise_ui_new(WIDTH, HEIGHT, DENISE_THEME_DARK);
uint64_t root = denise_ui_root(ui);

DeniseRect card = { 40, 30, WIDTH - 80, HEIGHT - 60 };
uint64_t panel = denise_ui_add_panel(ui, root, card,
                                     DENISE_ROLE_BASE_200, DENISE_ROLE_BASE_300, 1);

DeniseRect save_r = { 24, 130, 140, 44 };
denise_ui_add_button(ui, panel, save_r, "Lagre", MSG_SAVE, DENISE_ROLE_PRIMARY);

uint32_t message;
while (denise_ui_poll_message(ui, &message)) {
    if (message == MSG_SAVE) denise_ui_set_text(ui, note, "Lagret.");
}

denise_ui_tick(ui, 0);
denise_ui_paint(ui, &frame);                  /* the host's own pixels */
intptr_t rects = denise_ui_damage(ui, damage, DENISE_MAX_DAMAGE_RECTS);`
	}
];

export const install = `[dependencies]
denise = "VERSION"
denise-ui = "VERSION"
denise-winit = "VERSION"    # develop on a desktop
# denise-drm = "VERSION"    # ship on a display with no compositor
# denise-image = "VERSION"  # decode PNG, JPEG, GIF and BMP`;
