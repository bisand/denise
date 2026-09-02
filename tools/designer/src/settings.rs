//! What the designer remembers between runs.
//!
//! Deliberately tiny, and deliberately without a dependency: `key = value` lines
//! in the platform's own configuration directory. A settings file is not worth a
//! parser, and an unreadable one is not worth an error — a designer that refuses
//! to start because it could not remember a window size would be a worse
//! designer than one that opens at its default.

use std::path::PathBuf;

/// How the palette lists its widgets.
///
/// Cycled by the small button beside the palette's heading, and remembered
/// here because it is a taste, not a document property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PaletteMode {
    /// A glyph and the name. The default: the glyph is for recognising, the
    /// name is for searching and for learning which glyph is which.
    #[default]
    Both,
    /// Names alone, as the palette originally was.
    Text,
    /// Glyphs alone, on tiles — the whole catalogue in a third of the height,
    /// with the names in the tiles' tooltips.
    Glyphs,
}

impl PaletteMode {
    /// The one after this, wrapping — the order the toggle button cycles in.
    pub const fn next(self) -> Self {
        match self {
            Self::Both => Self::Text,
            Self::Text => Self::Glyphs,
            Self::Glyphs => Self::Both,
        }
    }

    /// The spelling the settings file uses, and the toggle button shows.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Text => "text",
            Self::Glyphs => "glyphs",
        }
    }

    /// The mode a settings line names, or the default for anything else.
    fn from_name(name: &str) -> Self {
        match name {
            "text" => Self::Text,
            "glyphs" => Self::Glyphs,
            _ => Self::Both,
        }
    }
}

/// The window's last size, where the panes were split, how the palette shows
/// itself, and which editor opens the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
    /// The palette and outline column.
    pub left: i32,
    /// The inspector column.
    pub right: i32,
    /// How the palette lists its widgets.
    pub palette: PaletteMode,
    /// The command that opens an event's handler, with `{file}`, `{line}` and
    /// `{column}` filled in. See [`crate::code`].
    pub editor: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            left: 240,
            right: 300,
            palette: PaletteMode::default(),
            editor: String::from(crate::code::DEFAULT_EDITOR),
        }
    }
}

impl Settings {
    /// Reads the settings, falling back to the defaults for anything missing,
    /// unreadable or absurd.
    pub fn load() -> Self {
        let mut settings = Self::default();
        let Some(path) = Self::path() else {
            return settings;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return settings;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "width" => settings.width = value.parse().unwrap_or(settings.width),
                "height" => settings.height = value.parse().unwrap_or(settings.height),
                "left" => settings.left = value.parse().unwrap_or(settings.left),
                "right" => settings.right = value.parse().unwrap_or(settings.right),
                "palette" => settings.palette = PaletteMode::from_name(value),
                "editor" => settings.editor = value.to_string(),
                _ => {}
            }
        }
        settings.sane()
    }

    /// Writes the settings, and says nothing if it cannot.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Self {
            width,
            height,
            left,
            right,
            palette,
            editor,
        } = self.clone().sane();
        let _ = std::fs::write(
            path,
            format!(
                "width = {width}\nheight = {height}\nleft = {left}\nright = {right}\n\
                 palette = {}\neditor = {editor}\n",
                palette.name()
            ),
        );
    }

    /// Clamps everything into a range a person could actually use.
    ///
    /// A settings file is a text file somebody may edit, and a pane six pixels
    /// wide or a window of zero is a designer that looks broken for a reason
    /// nobody would guess at.
    fn sane(mut self) -> Self {
        self.width = self.width.clamp(640, 16_384);
        self.height = self.height.clamp(400, 16_384);
        self.left = self.left.clamp(160, 640);
        self.right = self.right.clamp(200, 720);
        // An editor line somebody emptied is the default, not a designer that
        // can open nothing.
        if self.editor.trim().is_empty() {
            self.editor = String::from(crate::code::DEFAULT_EDITOR);
        }
        self
    }

    /// Where the settings live, by the platform's own convention.
    ///
    /// Hand-rolled rather than pulled in: three `env` lookups and a join, against
    /// a crate and its dependency tree.
    fn path() -> Option<PathBuf> {
        let base = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library").join("Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .map(|home| home.join(".config"))
                })
        }?;
        Some(base.join("denise-designer").join("settings"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonsense_in_the_file_does_not_produce_a_nonsense_window() {
        let absurd = Settings {
            width: 1,
            height: 0,
            left: -50,
            right: 100_000,
            palette: PaletteMode::Glyphs,
            editor: String::from("   "),
        };
        let sane = absurd.sane();
        assert_eq!(
            sane.editor,
            crate::code::DEFAULT_EDITOR,
            "an empty editor is the default"
        );
        assert!(sane.width >= 640 && sane.height >= 400);
        assert!(sane.left >= 160 && sane.right <= 720);
    }

    #[test]
    fn the_defaults_are_already_sane() {
        assert_eq!(Settings::default(), Settings::default().sane());
    }

    /// The mode survives the file: what `save` writes, `load` reads back —
    /// and a line somebody hand-edited into nonsense is the default, not a
    /// refusal to start.
    #[test]
    fn the_palette_mode_round_trips_through_its_name() {
        for mode in [PaletteMode::Both, PaletteMode::Text, PaletteMode::Glyphs] {
            assert_eq!(PaletteMode::from_name(mode.name()), mode);
        }
        assert_eq!(PaletteMode::from_name("puce"), PaletteMode::Both);
        // And the cycle visits all three before coming round.
        let start = PaletteMode::default();
        assert_ne!(start.next(), start);
        assert_ne!(start.next().next(), start);
        assert_eq!(start.next().next().next(), start);
    }
}
