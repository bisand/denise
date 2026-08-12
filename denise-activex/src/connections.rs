//! Cookies, and the table they index.
//!
//! A connection point hands out a number when a host advises a sink and takes it
//! back when the host unadvises. All of the ways that goes wrong are about the
//! numbers rather than the sinks — a cookie of zero matching something, a reused
//! cookie disconnecting the wrong host, a double unadvise silently succeeding —
//! so the table is generic over what it holds and lives outside `cfg(windows)`,
//! where it can be tested. [`crate::model`] fills it with `IDispatch`.

/// Sinks, each with the cookie that identifies it.
pub struct Connections<T> {
    entries: Vec<(u32, T)>,
    /// The next cookie to hand out. Never decreases.
    next: u32,
}

impl<T> Default for Connections<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Connections<T> {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            // One, not zero: zero is what an uninitialised cookie variable holds
            // in every host that ever forgot to store one, and it must match
            // nothing.
            next: 1,
        }
    }

    /// Adds a sink and returns its cookie.
    pub fn advise(&mut self, sink: T) -> u32 {
        let cookie = self.next;
        // Monotonic on purpose: reusing a cookie after an unadvise would let a
        // host holding a stale one disconnect whoever came next.
        self.next += 1;
        self.entries.push((cookie, sink));
        cookie
    }

    /// Removes a sink, reporting whether that cookie was connected.
    ///
    /// The report is the point. A host that unadvises twice, or with a cookie
    /// from a different object, has a bug, and `CONNECT_E_NOCONNECTION` is how it
    /// finds out.
    pub fn unadvise(&mut self, cookie: u32) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(held, _)| *held != cookie);
        self.entries.len() != before
    }

    /// Drops every sink.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// How many sinks are connected.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: Clone> Connections<T> {
    /// The sinks to raise an event on.
    ///
    /// A copy, deliberately: a handler is allowed to advise or unadvise while it
    /// runs, and iterating the live table while it did would be a borrow of
    /// something that is being written.
    pub fn sinks(&self) -> Vec<T> {
        self.entries.iter().map(|(_, sink)| sink.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zero is what an uninitialised cookie variable holds, and there are a lot
    /// of those. It must never name a connection.
    #[test]
    fn cookies_start_at_one_and_zero_matches_nothing() {
        let mut connections = Connections::new();
        assert_eq!(connections.advise("a"), 1);
        assert_eq!(connections.advise("b"), 2);
        assert!(!connections.unadvise(0));
        assert_eq!(connections.len(), 2);
    }

    /// The failure this prevents: two hosts advise, the first unadvises, a third
    /// advises and is given the freed cookie — and now the first host's stale
    /// cookie disconnects the third.
    #[test]
    fn a_cookie_is_never_handed_out_twice() {
        let mut connections = Connections::new();
        let first = connections.advise("a");
        connections.advise("b");
        assert!(connections.unadvise(first));

        let third = connections.advise("c");
        assert_ne!(third, first);
        assert!(
            !connections.unadvise(first),
            "the stale cookie still matches"
        );
        assert_eq!(connections.sinks(), vec!["b", "c"]);
    }

    /// A double unadvise is a bug in the host, and saying so is the only way it
    /// ever finds out.
    #[test]
    fn unadvising_twice_is_reported_the_second_time() {
        let mut connections = Connections::new();
        let cookie = connections.advise("a");
        assert!(connections.unadvise(cookie));
        assert!(!connections.unadvise(cookie));
        assert!(connections.is_empty());
    }

    /// `Close` breaks the cycle a container left behind, and has to break all of
    /// it.
    #[test]
    fn clearing_drops_every_sink() {
        let mut connections = Connections::new();
        connections.advise("a");
        connections.advise("b");
        connections.clear();
        assert!(connections.is_empty());
        assert!(connections.sinks().is_empty());
    }
}
