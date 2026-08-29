//! A clock and something to abandon, for callers that read a form they did not
//! write.
//!
//! [`Form::parse`](crate::Form::parse) is bounded in *shape* — [`MAX_SOURCE`],
//! [`MAX_DEPTH`], [`MAX_COMMENTED_DEPTH`] and a brace count, all applied by a
//! byte scan before the file reaches `kdl` — and not in *time*. It cannot be.
//! `kdl` 6.7.1 parses some malformed documents in exponential time
//! ([kdl-org/kdl-rs#177](https://github.com/kdl-org/kdl-rs/issues/177)): a
//! hundred and thirty bytes takes seventy-eight seconds, and a couple of
//! hundred does not finish. The byte scan refuses every shape the fuzzer has
//! found, but agreeing with `kdl` about where a string ends means *being*
//! `kdl`'s lexer, and a fourth divergence turned up five minutes after the
//! third was fixed. Anything that slips past costs whatever `kdl` costs.
//!
//! So the complete answer is a deadline, and it lives here rather than in each
//! caller because the reason for it does.
//!
//! # Abandoned, not stopped
//!
//! There is no way to stop a running parse. A thread cannot be cancelled and
//! `kdl` has no interruption point to ask it at, so what
//! [`Form::parse_within`] does when the deadline passes is walk away: the call
//! returns, and the worker keeps parsing until it finishes, which for the
//! exponential shapes may be never.
//!
//! That bounds the *call* and not the *process*, so one abandoned thread would
//! be a core burned for as long as the program runs, and an unbounded number of
//! them would be the denial of service this is here to prevent. Hence
//! [`MAX_ABANDONED`]: a parse whose predecessors are still wedged is refused
//! before it is started. The alternative — spawning anyway — makes a machine
//! that opened one hostile file unusable rather than merely annoyed.

use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use crate::error::{At, Error, Reason};
use crate::form::Form;
#[allow(unused_imports)] // Named by the module documentation above.
use crate::form::{MAX_COMMENTED_DEPTH, MAX_DEPTH, MAX_SOURCE};

/// How long to give a form before abandoning it, when there is no better
/// number to hand.
///
/// One second, against release measurements on an M5 Pro: the
/// [reference form](https://github.com/bisand/denise/blob/main/forms/reference.dform)
/// — every node kind this toolkit has, in nine and a half kilobytes — parses in
/// **under 3 ms**, and the other five forms in the repository in under 300 µs
/// each. So this is three hundred times the slowest real form, and
/// still short enough that a person reads the pause as *slow* rather than as
/// *hung*.
///
/// It is a default rather than a rule, and [`Form::parse_within`] takes the
/// number as an argument for two reasons. A file near [`MAX_SOURCE`] is
/// legitimately slower than this — four megabytes of real nodes measures 1.7 s,
/// and ten times that unoptimised — so a program that generates enormous forms
/// has to say so. And the machines this toolkit is for are not the machine
/// those numbers came off: the margin above is there to be spent, and a panel
/// that finds it is not enough should ask for more rather than go without.
pub const PATIENCE: Duration = Duration::from_secs(1);

/// How many parses may be running past their deadline before another is
/// refused outright.
///
/// Each one is a thread that cannot be stopped, so each one is a core this
/// process will not get back. Four is enough that a person who opens a bad
/// file, fixes it, and opens it again is never told no, and few enough that a
/// four-core panel keeps a core to draw with.
///
/// Reaching it is [`Reason::NoThread`], and it does not clear: the threads are
/// wedged for the life of the process, and the honest advice in that message is
/// to restart.
pub const MAX_ABANDONED: usize = 4;

/// The "I have finished" flag of every parse that overran its deadline.
///
/// Read rather than counted down, because the alternative races: a worker that
/// finishes in the same instant the caller gives up would otherwise either be
/// counted forever or not at all. A flag is a fact either side can check
/// whenever it likes.
static ABANDONED: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());

impl Form {
    /// Parses a form, giving up after `limit`.
    ///
    /// Otherwise exactly [`Form::parse`] — same document, same errors, same
    /// byte-for-byte round trip — with two more ways to fail:
    /// [`Reason::TooSlow`] when the deadline passes, and [`Reason::NoThread`]
    /// when the parse could not be started at all.
    ///
    /// **Use this for any form the program did not write**: opened by a person,
    /// pasted, downloaded, handed over on a stick, or watched on disk while a
    /// text editor has it too. A form compiled in with `include_str!` is read at
    /// build time from a file in the repository and needs nothing from here.
    ///
    /// The call returns within `limit` plus the cost of spawning a thread. What
    /// it does not do is **stop** the parse. A thread cannot be cancelled and
    /// `kdl` has no point at which to ask it to stop, so an overrun is
    /// abandoned: this returns, and the worker keeps parsing until it finishes,
    /// which for the exponential shapes may be never. That bounds the call and
    /// not the process, which is what [`MAX_ABANDONED`] is for.
    ///
    /// ```
    /// # use denise_forms::{Form, PATIENCE};
    /// let source = "form \"F\" version=1 width=64 height=32 {\n\
    ///     \x20   label \"Hello\" x=0 y=0 w=64 h=16\n\
    ///     }\n";
    ///
    /// let form = Form::parse_within(source, PATIENCE).expect("a form, in time");
    /// assert_eq!(form.text(), source);
    /// ```
    pub fn parse_within(source: &str, limit: Duration) -> Result<Self, Error> {
        // Two callers can pass this at the same count, so the cap is a bound
        // and not an invariant: it holds within the number of threads parsing
        // forms at once, which in every caller here is one. Holding a lock
        // across a spawn to make it exact would buy a fifth wedged thread's
        // worth of nothing.
        let wedged = still_running(&ABANDONED);
        if wedged >= MAX_ABANDONED {
            return Err(Error::new(
                At::START,
                Reason::NoThread { abandoned: wedged },
            ));
        }

        let done = Arc::new(AtomicBool::new(false));
        let finished = Arc::clone(&done);
        let owned = source.to_string();
        // Buffered, so the worker's send never blocks and never depends on
        // anyone still listening. An abandoned parse must be able to run to its
        // end and exit rather than parking on a channel forever.
        let (sender, results) = mpsc::sync_channel(1);

        let Ok(worker) = thread::Builder::new()
            .name(String::from("dform-parse"))
            .spawn(move || {
                let parsed = Self::parse(&owned);
                let _ = sender.send(parsed);
                finished.store(true, Ordering::Release);
            })
        else {
            // The system would not give us a thread, so there is nowhere to do
            // this that can be walked away from. Parsing here anyway is the one
            // thing this function exists not to do.
            return Err(Error::new(
                At::START,
                Reason::NoThread { abandoned: wedged },
            ));
        };

        match results.recv_timeout(limit) {
            Ok(parsed) => parsed,
            Err(RecvTimeoutError::Timeout) => {
                abandon(&ABANDONED, done);
                Err(Error::new(At::START, Reason::TooSlow { limit }))
            }
            // The worker ended without sending, which it can only do by
            // panicking — the channel has room and the receiver is right here.
            // A panic in `Form::parse` is a bug, and the fuzz target
            // `parse_form` exists to find them; putting a deadline on the parse
            // must not turn one into a quiet `Err`. So it is raised on this
            // thread, where it would have happened without the deadline. The
            // join cannot block: a disconnected channel means the closure has
            // already unwound.
            Err(RecvTimeoutError::Disconnected) => match worker.join() {
                Err(panicked) => panic::resume_unwind(panicked),
                Ok(()) => unreachable!("the worker returned without sending a result"),
            },
        }
    }
}

/// How many abandoned parses are still running, forgetting the ones that have
/// since finished.
///
/// Split out from [`Form::parse_within`] to be testable: wedging four real
/// threads to watch the fifth call be refused is not a test, it is a way to
/// make CI flaky.
fn still_running(abandoned: &Mutex<Vec<Arc<AtomicBool>>>) -> usize {
    let mut wedged = abandoned.lock().unwrap_or_else(PoisonError::into_inner);
    wedged.retain(|done| !done.load(Ordering::Acquire));
    wedged.len()
}

/// Takes note of a parse that overran, so that the next caller can count it.
fn abandon(abandoned: &Mutex<Vec<Arc<AtomicBool>>>, done: Arc<AtomicBool>) {
    abandoned
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(done);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(finished: bool) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(finished))
    }

    #[test]
    fn a_form_parses_within_the_default() {
        let source = std::fs::read_to_string("../forms/reference.dform").expect("the form is here");
        let form = Form::parse_within(&source, PATIENCE).expect("the reference form, in a second");
        assert_eq!(form.text(), source);
    }

    #[test]
    fn a_deadline_of_nothing_is_never_met() {
        // The reference form takes milliseconds and a thread takes microseconds
        // to start, so no scheduling accident makes this finish in no time at
        // all. Which is the only way to write this test: every input known to
        // be slow is refused by the byte scan before it reaches `kdl`, so the
        // way to make a parse miss a deadline is to move the deadline.
        let source = std::fs::read_to_string("../forms/reference.dform").expect("the form is here");
        let error = Form::parse_within(&source, Duration::ZERO).expect_err("no time at all");
        assert_eq!(
            error.reason,
            Reason::TooSlow {
                limit: Duration::ZERO
            }
        );
        assert!(error.to_string().contains("longer than"), "{error}");
    }

    #[test]
    fn the_error_a_missed_deadline_gives_is_not_about_the_file() {
        // A form that is refused for being slow has not been read, so the
        // position can only be the top of it. Worth asserting: every other
        // error in this crate points at the byte that caused it, and somebody
        // will reasonably expect this one to as well.
        let source = std::fs::read_to_string("../forms/reference.dform").expect("the form is here");
        let error = Form::parse_within(&source, Duration::ZERO).expect_err("no time at all");
        assert_eq!(error.at, At::START);
    }

    #[test]
    fn a_deadline_changes_nothing_about_what_a_form_means() {
        // The deadline wraps the parse; it must not stand in front of it. A
        // file the byte scan refuses has to come back as the refusal it is,
        // with the position it has, rather than as a timeout — otherwise the
        // safe call is the one with the worse error messages, and nobody would
        // use it.
        let refused = "form \"F\" version=1 width=1 height=1 {\n    panel \"p\" x=0 y=0 w=1 h=1\n";
        let direct = Form::parse(refused).expect_err("an unclosed brace");
        let bounded = Form::parse_within(refused, PATIENCE).expect_err("an unclosed brace");
        assert_eq!(direct.at, bounded.at);
        assert_eq!(direct.reason, bounded.reason);
        assert_eq!(bounded.reason, Reason::Unbalanced { open: true });
    }

    #[test]
    fn a_parse_that_has_finished_stops_being_counted() {
        let list = Mutex::new(vec![flag(true), flag(false), flag(true)]);
        assert_eq!(still_running(&list), 1);
        // And the finished ones are gone rather than counted again.
        assert_eq!(list.lock().expect("not poisoned").len(), 1);
    }

    #[test]
    fn wedged_parses_pile_up_until_the_limit() {
        let list = Mutex::new(Vec::new());
        for _ in 0..MAX_ABANDONED {
            assert!(still_running(&list) < MAX_ABANDONED);
            abandon(&list, flag(false));
        }
        assert_eq!(still_running(&list), MAX_ABANDONED);
    }

    #[test]
    fn no_thread_says_which_of_the_two_things_went_wrong() {
        let full = Error::new(
            At::START,
            Reason::NoThread {
                abandoned: MAX_ABANDONED,
            },
        );
        assert!(full.to_string().contains("restart"), "{full}");

        let refused = Error::new(At::START, Reason::NoThread { abandoned: 0 });
        assert!(refused.to_string().contains("thread"), "{refused}");
        assert!(!refused.to_string().contains("restart"), "{refused}");
    }
}
