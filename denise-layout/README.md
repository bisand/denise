# denise-layout

Keyboard layouts for [Denise](https://github.com/bisand/denise): what a key
*position* types, once you know which layout the user is on.

A backend answers where a key is — `KeyCode` is a position, and each platform
maps its own scancodes onto it. This crate answers what that position produces:
shift levels, dead keys and their composition, and which layout the machine is
already configured for.

```rust
use denise_layout::{Composer, from_system};
use denise::{ElementState, KeyCode, Modifiers};

// Whatever the machine is set to, with no configuration of our own.
let (layout, source) = from_system();
println!("{} (from {source:?})", layout.name);

let mut composer = Composer::new(layout);
let typed = composer.feed(KeyCode::A, ElementState::Down, Modifiers::NONE);
assert_eq!(typed.as_slice(), &['a']);
```

Dead keys compose across two presses, and a mark that cannot combine with what
follows emits both characters rather than swallowing one:

```rust
# use denise_layout::{Composer, by_name};
# use denise::{ElementState, KeyCode, Modifiers};
let mut composer = Composer::new(by_name("no").expect("built in"));
let press = |c: &mut Composer, k| c.feed(k, ElementState::Down, Modifiers::NONE);

assert!(press(&mut composer, KeyCode::BracketRight).is_empty()); // ¨, held
assert_eq!(press(&mut composer, KeyCode::O).as_slice(), &['ö']);
```

`US` and `NORWEGIAN` ship with the crate; `BUILT_IN` lists them and `by_name`
looks one up. Adding a layout is a table of positions and about thirty lines.

## Where the layout comes from

`from_system` reads the machine's *choice* — `DENISE_KEYMAP`, then
`XKB_DEFAULT_LAYOUT`, then the console keymap files distributions actually
write — and `LayoutSource` says which of them answered, or that nothing did and
US was assumed. The layout *data* comes from this crate rather than the system,
because the two ways to read the system's tables are `KDGKBENT` on a VT, which
needs root, and libxkbcommon, which is a C library with a runtime data
directory. Denise runs unprivileged as one static binary, and gives up neither.
