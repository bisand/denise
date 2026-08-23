//! What the designer remembers between runs.
//!
//! Deliberately tiny, and deliberately without a dependency: `key = value` lines
//! in the platform's own configuration directory. A settings file is not worth a
//! parser, and an unreadable one is not worth an error — a designer that refuses
//! to start because it could not remember a window size would be a worse
//! designer than one that opens at its default.

use std::path::PathBuf;

/// The window's last size, and where the panes were split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
    pub height: u32,
    /// The palette and outline column.
    pub left: i32,
    /// The inspector column.
    pub right: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            left: 240,
            right: 300,
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
                _ => {}
            }
        }
        settings.sane()
    }

    /// Writes the settings, and says nothing if it cannot.
    pub fn save(self) {
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
        } = self.sane();
        let _ = std::fs::write(
            path,
            format!("width = {width}\nheight = {height}\nleft = {left}\nright = {right}\n"),
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
        };
        let sane = absurd.sane();
        assert!(sane.width >= 640 && sane.height >= 400);
        assert!(sane.left >= 160 && sane.right <= 720);
    }

    #[test]
    fn the_defaults_are_already_sane() {
        assert_eq!(Settings::default(), Settings::default().sane());
    }
}
