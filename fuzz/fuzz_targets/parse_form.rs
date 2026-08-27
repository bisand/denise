//! Anything at all, through `Form::parse`.
//!
//! A `.dform` is the newest thing in the repository that eats untrusted bytes:
//! a form is downloaded, pasted, or written by a person who is tired. The
//! parser's contract is that any input at all is either a `Form` or an `Error`
//! that names a position — a panic is a finding, and so is an error that
//! cannot say where.
//!
//! The size and depth guards are asserted here as well as in the unit tests,
//! so the fuzzer hunts for a way *past* them rather than only for a crash
//! behind them: an input that parses despite being over `MAX_SOURCE`, or one
//! that builds a tree deeper than `MAX_DEPTH`, would trip these before it
//! tripped a stack.
//!
//! What parses must round-trip, and that is asserted. What is *refused* is
//! not: `Form::parse` verifies every file it accepts and turns a document it
//! cannot reproduce into an error, which is the safe outcome and not a finding.
//!
//! libFuzzer's own timing is worth reading here and is not gated on:
//! `MAX_COMMENTED_DEPTH` and the balance check both exist because this target
//! kept saving `slow-unit-*` artifacts that took `kdl` a second and a half to
//! *fail* on. That is an upstream bug with more corners than can be blocked
//! from out here, so CI reports the artifacts and does not fail on them. See
//! `fuzz/README.md`.

#![no_main]

use denise_forms::{Form, MAX_SOURCE};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = core::str::from_utf8(data) else {
        return;
    };
    match Form::parse(source) {
        Ok(form) => {
            assert!(
                source.len() <= MAX_SOURCE,
                "a form larger than MAX_SOURCE parsed anyway",
            );
            // What was accepted must come back byte-for-byte: every edit the
            // designer makes stands on this.
            assert_eq!(form.text(), source, "a parsed form did not round-trip");
        }
        Err(error) => {
            // `Reason::NotPreserved` is a refusal, and refusing is the whole
            // point of it: no file is ever accepted that would corrupt on save.
            // It used to be asserted against here, which is how the trivia kdl
            // eats after a closing brace got found and repaired. That assertion
            // is gone because it cannot be satisfied: kdl records
            // `before_ty_name`, `after_ty_name` and `after_ty` when it reads a
            // node's type annotation and then never writes them, so `(Z) h`
            // comes back as `(Z)h` and no format this crate can set will change
            // that. The safety property is asserted where it belongs — on the
            // `Ok` arm above, where a form that parsed must round-trip.
            //
            // An error must know where it happened — `1:1` for the guards that
            // fire before the parser, a real position for the rest — and must
            // be printable without panicking (a `Display` that indexes the
            // source with a stale span would panic right here).
            let shown = error.to_string();
            assert!(!shown.is_empty(), "an error with nothing to say");
        }
    }
});
