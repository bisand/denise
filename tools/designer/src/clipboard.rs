//! The system clipboard, and what a designer puts on it.
//!
//! # Why the clipboard carries source
//!
//! Copying nodes puts **`.dform` text** on the clipboard, not a private
//! encoding. That is Delphi's trick and it is worth stealing: paste into a text
//! editor and you have the source, paste from a text editor and you have the
//! nodes, and copying between two running designers is free because there was
//! never a second format to agree on. It also means the clipboard is
//! inspectable, which a private encoding is not.
//!
//! # Why there is a fallback
//!
//! There is not always a system clipboard. A headless machine has no display to
//! own a selection, and a test run must not reach for the one belonging to the
//! person at the keyboard — a `cargo test` that clobbered what somebody had
//! copied would be a rude test suite, and two tests running at once would
//! clobber each other. So this keeps its own copy, and uses it when there is no
//! other.

/// A handle on wherever copied text goes.
pub struct Clipboard {
    /// The machine's own, when there is one. Held open rather than opened per
    /// use: on X11 the clipboard's contents belong to a live connection, and a
    /// handle that was dropped takes what it was holding with it.
    system: Option<arboard::Clipboard>,
    /// What this designer last copied, for when there is no system clipboard.
    own: String,
}

impl Clipboard {
    /// Opens the system clipboard, if this machine has one.
    pub fn new() -> Self {
        Self {
            system: if cfg!(test) {
                None
            } else {
                arboard::Clipboard::new().ok()
            },
            own: String::new(),
        }
    }

    /// One that never touches the machine's own, whatever machine this is.
    #[cfg(test)]
    pub const fn detached() -> Self {
        Self {
            system: None,
            own: String::new(),
        }
    }

    /// Whether there is a system clipboard behind this at all.
    pub const fn is_system(&self) -> bool {
        self.system.is_some()
    }

    /// Puts text on the clipboard.
    ///
    /// The copy kept here is written either way, so a machine that refused the
    /// text a moment ago can still be pasted back into this designer.
    pub fn put(&mut self, text: &str) {
        self.own = String::from(text);
        if let Some(system) = self.system.as_mut() {
            let _ = system.set_text(text);
        }
    }

    /// What is on the clipboard now, if it is text.
    ///
    /// The system's own comes first, because that is what somebody pressing
    /// paste means — including text they typed somewhere else entirely, which
    /// is the point of carrying source.
    pub fn take(&mut self) -> Option<String> {
        let text = self
            .system
            .as_mut()
            .and_then(|system| system.get_text().ok())
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| self.own.clone());
        (!text.trim().is_empty()).then_some(text)
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Clipboard")
            .field("system", &self.is_system())
            .field("own", &self.own.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detached_clipboard_hands_back_what_was_put_on_it() {
        let mut clipboard = Clipboard::detached();
        assert!(!clipboard.is_system());
        assert_eq!(clipboard.take(), None, "it starts empty");

        clipboard.put("label \"a\" x=0 y=0 w=1 h=1\n");
        assert_eq!(
            clipboard.take().as_deref(),
            Some("label \"a\" x=0 y=0 w=1 h=1\n")
        );
    }

    #[test]
    fn a_test_run_never_reaches_for_the_machines_own() {
        assert!(
            !Clipboard::new().is_system(),
            "a test would clobber what somebody had copied"
        );
    }

    #[test]
    fn whitespace_is_not_something_to_paste() {
        let mut clipboard = Clipboard::detached();
        clipboard.put("   \n  ");
        assert_eq!(clipboard.take(), None);
    }
}
