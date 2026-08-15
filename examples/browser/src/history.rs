//! Where you have been, and where Forward still leads.
//!
//! A vector and a cursor. Navigating somewhere new truncates the forward
//! tail — the rule every browser shares — and each entry keeps the scroll
//! position it was left at, so Back returns to the paragraph, not the top.
//! Nothing is cached; going back fetches again, which an example is allowed
//! to say out loud.

use denise::Point;
use url::Url;

pub struct Entry {
    pub url: Url,
    pub scroll: Point,
}

#[derive(Default)]
pub struct History {
    entries: Vec<Entry>,
    /// Index of the current entry, meaningful only when non-empty.
    cursor: usize,
}

impl History {
    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    /// Remembers how far down the current page was scrolled, for when Back
    /// or Forward returns here.
    pub fn save_scroll(&mut self, scroll: Point) {
        if let Some(entry) = self.entries.get_mut(self.cursor) {
            entry.scroll = scroll;
        }
    }

    /// A new place: everything Forward pointed at is forgotten.
    pub fn push(&mut self, url: Url) {
        if !self.entries.is_empty() {
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(Entry {
            url,
            scroll: Point::ZERO,
        });
        self.cursor = self.entries.len() - 1;
    }

    /// The current entry's address changed under it — a redirect landed
    /// somewhere else than the link said.
    pub fn replace(&mut self, url: Url) {
        if let Some(entry) = self.entries.get_mut(self.cursor) {
            entry.url = url;
        }
    }

    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_forward(&self) -> bool {
        !self.entries.is_empty() && self.cursor + 1 < self.entries.len()
    }

    pub fn back(&mut self) -> Option<&Entry> {
        if !self.can_back() {
            return None;
        }
        self.cursor -= 1;
        self.entries.get(self.cursor)
    }

    pub fn forward(&mut self) -> Option<&Entry> {
        if !self.can_forward() {
            return None;
        }
        self.cursor += 1;
        self.entries.get(self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn pushing_truncates_the_forward_tail() {
        let mut h = History::default();
        h.push(url("https://a.example"));
        h.push(url("https://b.example"));
        h.back();
        h.push(url("https://c.example"));
        assert!(!h.can_forward(), "b is forgotten");
        assert_eq!(h.current().unwrap().url.host_str(), Some("c.example"));
        assert!(h.can_back());
    }

    #[test]
    fn scroll_survives_the_round_trip() {
        let mut h = History::default();
        h.push(url("https://a.example"));
        h.push(url("https://b.example"));
        h.back();
        h.save_scroll(Point::new(0, 420));
        h.forward();
        let back = h.back().unwrap();
        assert_eq!(back.scroll.y, 420);
    }

    #[test]
    fn empty_history_goes_nowhere() {
        let mut h = History::default();
        assert!(h.back().is_none());
        assert!(h.forward().is_none());
        assert!(h.current().is_none());
    }
}
