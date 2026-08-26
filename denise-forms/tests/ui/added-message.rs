//! The other half: a `match` on the generated enum stops being exhaustive the
//! moment the form gains a message.
//!
//! `hello.dform` emits one message, `greet`. This matches a variant that is not
//! there — which is the shape of the error an application gets when a message is
//! *removed*; and the missing-arm error is what it gets when one is added.

include!("hello_form.rs");

fn main() {
    let message = HelloMessage::Greet;
    match message {
        HelloMessage::Greet => {}
        HelloMessage::Cancel => {}
    }
}
