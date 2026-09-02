//! The trait's half of the claim: a type that handles the form has to have a
//! method for every event, and the compiler names the one it lacks.
//!
//! `hello.dform` emits `greet`. This handles the form and forgets it — which is
//! what an application looks like the moment somebody adds an event in the
//! designer — and the error names `greet`, the method the designer would write.

include!("hello_form.rs");

struct App;

impl HelloHandlers for App {}

fn main() {
    let mut app = App;
    HelloMessage::Greet.dispatch(&mut app);
}
